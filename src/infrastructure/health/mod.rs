//! Health checking: probes and readiness service.
//!
//! This module aggregates infrastructure health checks (database, OIDC, etc.)
//! and determines service readiness based on critical probe failures.

pub mod probes;
pub mod service;

pub use crate::domain::health::HealthProbe;
pub use probes::{DatabaseProbe, OidcProbe};
pub use service::{CheckResult, HealthService, ReadinessReport};
