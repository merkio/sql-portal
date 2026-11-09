//! Infrastructure layer: adapters implementing the domain ports plus
//! configuration loading, health checks, and telemetry.

pub mod config;
pub mod github;
pub mod grants;
pub mod health;
pub mod ldap;
pub mod oidc;
pub mod repository;
pub mod sql;
pub mod telemetry;
pub mod tls;

// Re-export commonly used infrastructure adapters
pub use health::{CheckResult, DatabaseProbe, HealthService, OidcProbe, ReadinessReport};
pub use telemetry::{describe_and_init, install_prometheus, MetricsTelemetry};
