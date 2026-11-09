//! Domain layer: entities, value objects, domain services and the ports
//! (traits) that outer layers implement. Nothing here depends on web, database,
//! LDAP or OIDC crates.
//!
//! The domain is split into bounded contexts. A context may depend on the ones
//! listed before it, never on one listed after it:
//!
//! | context    | answers                              | depends on             |
//! |------------|--------------------------------------|------------------------|
//! | `shared`   | common kernel: values and errors     | nothing                |
//! | `access`   | which permissions exist              | `shared`               |
//! | `identity` | who the user is and what they hold   | `access`, `shared`     |
//! | `query`    | running SQL: requests and results    | `shared`               |
//! | `scripts`  | script definitions, parameters, charts | `access`, `query`, `shared` |
//! | `ports`    | outbound seams, one file per context | all of the above       |
//!
//! The flat re-exports below keep `crate::domain::Name` stable for callers.

pub mod access;
pub mod common;
pub mod health;
pub mod ports;
pub mod query;
pub mod scripts;
pub mod telemetry;
pub mod tls;

pub use access::{AccessPolicy, AuthError, AuthProvider, Grants, LocalGrants, Permission, PermissionSet, Principal, ScriptAccessRule, User};
pub use common::{DomainError, Value};
pub use query::{PageInfo, PageRequest, QueryError, QueryRequest, QueryResult, QueryExecutor};
pub use scripts::{
    ChartData, ChartDefinition, ChartKind, Dataset, DatasetData, FieldError, ParamDefinition,
    ParamKind, ScriptCatalog, ScriptDefinition, ScriptError, ScriptName, SelectOption,
};
pub use telemetry::Telemetry;
