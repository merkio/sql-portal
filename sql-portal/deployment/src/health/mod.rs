//! Health checks: the probes, the `HealthService` that aggregates them, and
//! its wiring. `/healthz` and `/readyz` are served through the
//! [`Readiness`](sql_portal_api::Readiness) port.

mod probes;
mod service;

use std::sync::Arc;

use async_trait::async_trait;
use sql_portal_domain::{OidcProvider, QueryExecutor};

pub use probes::{DatabaseProbe, OidcProbe};
pub use service::HealthService;

/// A dependency health check used by the readiness endpoint.
#[async_trait]
pub trait HealthProbe: Send + Sync {
    fn name(&self) -> &'static str;
    /// A failing critical probe makes the service "not ready".
    fn critical(&self) -> bool {
        true
    }
    async fn check(&self) -> Result<(), String>;
}

/// Database probe first, then OIDC discovery (when enabled), then any extras.
pub fn service(
    executor: Arc<dyn QueryExecutor>,
    oidc: Option<Arc<dyn OidcProvider>>,
    extra: Vec<Arc<dyn HealthProbe>>,
) -> Arc<HealthService> {
    let mut probes: Vec<Arc<dyn HealthProbe>> = vec![Arc::new(DatabaseProbe::new(executor))];
    if let Some(oidc) = oidc {
        probes.push(Arc::new(OidcProbe::new(oidc)));
    }
    probes.extend(extra);
    Arc::new(HealthService::new(probes))
}
