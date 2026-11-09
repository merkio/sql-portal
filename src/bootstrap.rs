//! Composition root. [`wire`] builds the object graph from a validated
//! [`RuntimeConfig`]; [`router`] turns the services into HTTP. Tests reuse both
//! and replace only the adapters they need through [`Overrides`].

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use tracing::{info, warn};

use crate::infrastructure::grants::{CachedGrants, RemoteGrants};
use crate::application::{
    AuthMode, AuthService, LocalGrants, Providers, RequiredPermissions, ScriptService,
};
use crate::domain::ports::{
    CredentialAuthenticator, GithubLogin, GrantSource, HealthProbe, OidcProvider, QueryExecutor,
    ScriptAccessRule, Telemetry,
};
use crate::infrastructure::config::runtime::{AuthRuntime, GrantsRuntime, RuntimeConfig};
use crate::infrastructure::github::GithubAdapter;
use crate::infrastructure::health::{DatabaseProbe, HealthService, OidcProbe};
use crate::infrastructure::ldap::LdapAuthenticator;
use crate::infrastructure::oidc::OidcAdapter;
use crate::infrastructure::repository::ConfigScriptCatalog;
use crate::infrastructure::sql::{connect_database, prepare_database};
use crate::infrastructure::telemetry::MetricsTelemetry;
use crate::ui::{build_router, AppState, RouterSettings, UiSettings};

/// Adapters a caller may replace. Anything left `None` is built from the
/// configuration.
#[derive(Default)]
pub struct Overrides {
    pub executor: Option<Arc<dyn QueryExecutor>>,
    pub auth: Option<AuthMode>,
    pub grants: Option<Arc<dyn GrantSource>>,
    pub script_access: Option<Arc<dyn ScriptAccessRule>>,
    pub telemetry: Option<Arc<dyn Telemetry>>,
    /// Probes added after the database and OIDC probes.
    pub extra_probes: Vec<Arc<dyn HealthProbe>>,
}

/// The application services, ready to be served.
pub struct Services {
    pub auth: Arc<AuthService>,
    pub scripts: Arc<ScriptService>,
    pub health: Arc<crate::infrastructure::health::HealthService>,
    pub ui: Arc<UiSettings>,
    pub settings: RouterSettings,
}

/// Builds the object graph. Prepares the database (migrations, init files)
/// through whichever executor is used.
pub async fn wire(rt: &RuntimeConfig, overrides: Overrides) -> anyhow::Result<Services> {
    let Overrides {
        executor,
        auth,
        grants,
        script_access,
        telemetry,
        extra_probes,
    } = overrides;

    // --- database -------------------------------------------------------
    let executor = match executor {
        Some(executor) => executor,
        None => connect_database(
            &rt.database.url,
            rt.database.max_connections,
            rt.database.acquire_timeout,
            &rt.database.tls,
        )
        .await
        .context("cannot connect to the database")?,
    };
    prepare_database(
        executor.as_ref(),
        rt.database.migrations.as_deref(),
        &rt.database.init_files,
    )
    .await?;

    // --- scripts and grants ----------------------------------------------
    info!(count = rt.scripts.len(), "scripts loaded");
    let catalog = Arc::new(ConfigScriptCatalog::new(rt.scripts.clone()));
    let grants: Arc<dyn GrantSource> = match grants {
        Some(grants) => grants,
        None => match &rt.grants {
            GrantsRuntime::Local => Arc::new(LocalGrants::new(rt.policy.clone())),
            GrantsRuntime::External(external) => {
                let remote = RemoteGrants::new(
                    &external.settings.url,
                    external.settings.bearer_token.clone(),
                    external.settings.timeout,
                    &external.tls,
                )
                .context("cannot configure the grants service")?;
                info!(url = %external.settings.url, "grants loaded from an external service");
                Arc::new(CachedGrants::new(
                    Arc::new(remote),
                    external.settings.cache_ttl,
                ))
            }
        },
    };

    // --- authentication ----------------------------------------------------
    let mode = match auth {
        Some(mode) => mode,
        None => build_auth_mode(rt)?,
    };
    let oidc = match &mode {
        AuthMode::Enabled(providers) => providers.oidc.clone(),
        AuthMode::Disabled => None,
    };
    let telemetry = telemetry.unwrap_or_else(|| Arc::new(MetricsTelemetry));
    let auth = Arc::new(AuthService::new(mode, grants, telemetry.clone()));

    // --- services ----------------------------------------------------------
    let access = script_access.unwrap_or_else(|| Arc::new(RequiredPermissions));
    let scripts = Arc::new(ScriptService::new(
        catalog,
        executor.clone(),
        telemetry,
        access,
    ));

    let mut probes: Vec<Arc<dyn HealthProbe>> = vec![Arc::new(DatabaseProbe::new(executor))];
    if let Some(oidc) = oidc {
        probes.push(Arc::new(OidcProbe::new(oidc)));
    }
    probes.extend(extra_probes);

    Ok(Services {
        auth,
        scripts,
        health: Arc::new(HealthService::new(probes)),
        ui: Arc::new(UiSettings {
            title: rt.ui.title.clone(),
            chartjs_url: rt.ui.chartjs_url.clone(),
        }),
        settings: RouterSettings {
            metrics_enabled: rt.observability.metrics_enabled,
            metrics_path: rt.observability.metrics_path.clone(),
            cookie_name: rt.server.session.cookie_name.clone(),
            cookie_secure: rt.server.session.secure,
            session_ttl_minutes: rt.server.session.ttl_minutes,
        },
    })
}

fn build_auth_mode(rt: &RuntimeConfig) -> anyhow::Result<AuthMode> {
    match &rt.auth {
        AuthRuntime::Disabled => {
            warn!(
                "authentication is DISABLED: every request is served as an anonymous administrator"
            );
            Ok(AuthMode::Disabled)
        }
        AuthRuntime::Enabled { ldap, oidc, github } => {
            let ldap = match ldap {
                Some(c) => {
                    info!(url = %c.settings.url, "LDAP authentication enabled");
                    Some(
                        Arc::new(LdapAuthenticator::new(c.settings.clone(), &c.tls)?)
                            as Arc<dyn CredentialAuthenticator>,
                    )
                }
                None => None,
            };
            let oidc = match oidc {
                Some(c) => {
                    info!(issuer = %c.settings.issuer_url, "OIDC authentication enabled");
                    Some(Arc::new(OidcAdapter::new(
                        c.settings.clone(),
                        &rt.server.public_url,
                        &c.tls,
                    )?) as Arc<dyn OidcProvider>)
                }
                None => None,
            };
            let github = match github {
                Some(cfg) => {
                    info!(web = %cfg.web_url, "GitHub authentication enabled");
                    Some(Arc::new(GithubAdapter::new(cfg, &rt.server.public_url)?)
                        as Arc<dyn GithubLogin>)
                }
                None => None,
            };
            Ok(AuthMode::Enabled(Providers { ldap, oidc, github }))
        }
    }
}

/// Builds the HTTP router from wired services.
pub fn router(services: Services, metrics: Arc<dyn Fn() -> String + Send + Sync>) -> Router {
    build_router(
        AppState {
            auth: services.auth,
            scripts: services.scripts,
            health: services.health,
            metrics,
            ui: services.ui,
        },
        &services.settings,
    )
}

/// The production application: default overrides, router included.
pub async fn build_app(
    rt: &RuntimeConfig,
    metrics: Arc<dyn Fn() -> String + Send + Sync>,
) -> anyhow::Result<Router> {
    build_app_with(rt, metrics, Overrides::default()).await
}

/// Like [`build_app`], with replaced adapters, for example a script access
/// rule implemented in another crate.
pub async fn build_app_with(
    rt: &RuntimeConfig,
    metrics: Arc<dyn Fn() -> String + Send + Sync>,
    overrides: Overrides,
) -> anyhow::Result<Router> {
    Ok(router(wire(rt, overrides).await?, metrics))
}
