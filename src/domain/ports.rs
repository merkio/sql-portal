//! Ports (traits) that are implemented by the infrastructure layer.
//! This module re-exports all outbound seams in one place for convenience.

pub use crate::domain::access::{
    CredentialAuthenticator, GithubLogin, GrantSource, OidcAuthorization, OidcPending,
    OidcProvider, ScriptAccessRule,
};
pub use crate::domain::health::HealthProbe;
pub use crate::domain::query::QueryExecutor;
pub use crate::domain::scripts::ScriptCatalog;
pub use crate::domain::telemetry::Telemetry;
