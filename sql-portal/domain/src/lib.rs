//! SQL Portal Domain Layer
//!
//! Pure domain logic with no web, database, or auth dependencies.
//! Split into bounded contexts with clear boundaries.

pub mod access;
pub mod common;
pub mod query;
pub mod scripts;
pub mod telemetry;

// Re-export key types for convenient access
pub use access::{
    AccessPolicy, AuthError, AuthProvider, Grants, LocalGrants, Permission, PermissionSet,
    Principal, ScriptAccessRule, User,
};
pub use common::{DomainError, Value};
// Ports (outbound seams). Each trait lives in its bounded context; they are
// re-exported here so callers can write `sql_portal_domain::QueryExecutor`.
pub use access::{
    CredentialAuthenticator, GithubLogin, GrantSource, OidcAuthorization, OidcPending, OidcProvider,
};
pub use query::{PageInfo, PageRequest, QueryError, QueryExecutor, QueryRequest, QueryResult};
pub use scripts::{
    ChartData, ChartDefinition, ChartKind, Dataset, DatasetData, FieldError, ParamDefinition,
    ParamKind, ScriptCatalog, ScriptDefinition, ScriptError, ScriptName, SelectOption,
};
pub use telemetry::Telemetry;
