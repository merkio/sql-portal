//! SQL Portal application layer: use cases that orchestrate domain objects
//! through ports. No knowledge of HTTP, SQL drivers or identity providers.
//!
//! One module per use-case area:
//! - [`auth`]: login (LDAP, OIDC, GitHub), principal resolution, auth switch
//! - [`scripts`]: list / view / run scripts with permission checks
//! - [`health`]: liveness and readiness from the registered probes

pub mod auth;
pub mod health;
pub mod scripts;

pub use auth::{AuthMode, AuthService, OidcCallback, Providers};
pub use health::{CheckResult, HealthService, ReadinessReport};
pub use scripts::{AuthorizedScript, RequiredPermissions, ScriptRun, ScriptService, ScriptSummary};
pub use sql_portal_domain::LocalGrants;
