//! The validated configuration. A `RuntimeConfig` only exists after every
//! rule has been checked and every domain object has been built, so the
//! composition root never sees a half-valid file.

use std::path::PathBuf;
use std::time::Duration;

use super::model::{
    GithubConfig, LdapConfig, ObservabilityConfig, OidcConfig, ServerConfig, UiConfig,
};
use crate::tls::TlsOptions;
use sql_portal_domain::{AccessPolicy, ScriptDefinition};

pub struct RuntimeConfig {
    pub server: ServerConfig,
    pub ui: UiConfig,
    pub observability: ObservabilityConfig,
    pub database: DatabaseRuntime,
    pub auth: AuthRuntime,
    pub grants: GrantsRuntime,
    /// Local roles and permissions, used when no external grant service is set.
    pub policy: AccessPolicy,
    pub scripts: Vec<ScriptDefinition>,
}

pub struct DatabaseRuntime {
    pub url: String,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
    pub tls: TlsOptions,
    pub init_files: Vec<PathBuf>,
    /// `Some` only when migrations are enabled in the configuration.
    pub migrations: Option<PathBuf>,
}

/// A configured remote endpoint with the TLS options that apply to it.
pub struct Connection<S> {
    pub settings: S,
    pub tls: TlsOptions,
}

pub enum AuthRuntime {
    /// Every request is an anonymous administrator.
    Disabled,
    Enabled {
        ldap: Option<Box<Connection<LdapConfig>>>,
        oidc: Option<Box<Connection<OidcConfig>>>,
        github: Option<Box<GithubConfig>>,
    },
}

pub enum GrantsRuntime {
    Local,
    External(Connection<ExternalGrants>),
}

pub struct ExternalGrants {
    pub url: String,
    pub timeout: Duration,
    pub cache_ttl: Duration,
    pub bearer_token: Option<String>,
}
