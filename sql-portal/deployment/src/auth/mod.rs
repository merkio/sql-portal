//! `auth:` section → the `AuthService` and its identity providers.

use std::sync::Arc;

use sql_portal_adapters::auth::{GithubAdapter, LdapAuthenticator, OidcAdapter};
use sql_portal_adapters::config::runtime::{AuthRuntime, RuntimeConfig};
use sql_portal_application::{AuthMode, AuthService, Providers};
use sql_portal_domain::{
    CredentialAuthenticator, GithubLogin, GrantSource, OidcProvider, Telemetry,
};
use tracing::{info, warn};

/// The authentication mode: the supplied one, or providers built from config.
pub fn mode(rt: &RuntimeConfig, supplied: Option<AuthMode>) -> anyhow::Result<AuthMode> {
    match supplied {
        Some(mode) => Ok(mode),
        None => mode_from_config(rt),
    }
}

/// The OIDC provider of an enabled mode (its discovery endpoint is probed by `/readyz`).
pub fn oidc_provider(mode: &AuthMode) -> Option<Arc<dyn OidcProvider>> {
    match mode {
        AuthMode::Enabled(providers) => providers.oidc.clone(),
        AuthMode::Disabled => None,
    }
}

/// The fully wired authentication use case.
pub fn service(
    mode: AuthMode,
    grants: Arc<dyn GrantSource>,
    telemetry: Arc<dyn Telemetry>,
) -> Arc<AuthService> {
    Arc::new(AuthService::new(mode, grants, telemetry))
}

fn mode_from_config(rt: &RuntimeConfig) -> anyhow::Result<AuthMode> {
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
