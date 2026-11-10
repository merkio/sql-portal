//! SQL Portal application layer: use cases that orchestrate domain objects
//! through ports. No knowledge of HTTP, SQL drivers or identity providers.
//!
//! One module per use-case area:
//! - [`auth`]: login (LDAP, OIDC, GitHub), principal resolution, auth switch
//! - [`observability`]: the `Metrics` port the use cases report to
//! - [`scripts`]: list / view / run scripts with permission checks

pub mod auth;
pub mod observability;
pub mod scripts;

pub use auth::{AuthMode, AuthService, OidcCallback, Providers};
pub use observability::{Metrics, NoopMetrics};
pub use scripts::{AuthorizedScript, RequiredPermissions, ScriptRun, ScriptService, ScriptSummary};
pub use sql_portal_domain::LocalGrants;
