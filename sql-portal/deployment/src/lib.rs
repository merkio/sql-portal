//! Composition root of the SQL Portal.
//!
//! Every part of the application is configured from the YAML-derived
//! [`RuntimeConfig`] in its own module:
//!
//! | module            | builds                                                   |
//! |-------------------|----------------------------------------------------------|
//! | [`database`]      | the `QueryExecutor` (SQLite / PostgreSQL / ClickHouse)   |
//! | [`access`]        | the `GrantSource` (local policy or external service)     |
//! | [`auth`]          | the `AuthService` with LDAP / OIDC / GitHub providers    |
//! | [`scripts`]       | the `ScriptService` with its catalog and access rule     |
//! | [`health`]        | the `HealthService` with database and OIDC probes        |
//! | [`observability`] | tracing, Prometheus and the `Telemetry` port             |
//! | [`http`]          | the axum router                                          |
//!
//! [`wire`] assembles them; tests replace single adapters through [`Overrides`].

use std::sync::Arc;

use axum::Router;
use sql_portal_adapters::config::RuntimeConfig;
use sql_portal_application::{AuthMode, AuthService, ScriptService};
use sql_portal_domain::{GrantSource, QueryExecutor, ScriptAccessRule, Telemetry};
pub use sql_portal_ui::MetricsRender;
use sql_portal_ui::{RouterSettings, UiSettings};

use crate::health::{HealthProbe, HealthService};

pub mod access;
pub mod auth;
pub mod database;
pub mod health;
pub mod http;
pub mod observability;
pub mod scripts;

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
    pub health: Arc<HealthService>,
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

    let executor = database::executor(rt, executor).await?;
    let telemetry = observability::telemetry(telemetry);
    let grants = access::grant_source(rt, grants)?;
    let mode = auth::mode(rt, auth)?;
    let oidc = auth::oidc_provider(&mode);
    let auth = auth::service(mode, grants, telemetry.clone());
    let scripts = scripts::service(rt, executor.clone(), telemetry, script_access);
    let health = health::service(executor, oidc, extra_probes);

    Ok(Services {
        auth,
        scripts,
        health,
        ui: http::ui_settings(rt),
        settings: http::router_settings(rt),
    })
}

/// The production application: default overrides, router included.
pub async fn build_app(rt: &RuntimeConfig, metrics: MetricsRender) -> anyhow::Result<Router> {
    build_app_with(rt, metrics, Overrides::default()).await
}

/// Like [`build_app`], with replaced adapters, for example a script access
/// rule implemented in another crate.
pub async fn build_app_with(
    rt: &RuntimeConfig,
    metrics: MetricsRender,
    overrides: Overrides,
) -> anyhow::Result<Router> {
    Ok(http::router(wire(rt, overrides).await?, metrics))
}
