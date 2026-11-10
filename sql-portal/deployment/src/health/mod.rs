//! Health checks → the `HealthService` behind `/healthz` and `/readyz`.

use std::sync::Arc;

use sql_portal_adapters::health::{DatabaseProbe, OidcProbe};
use sql_portal_application::HealthService;
use sql_portal_domain::{HealthProbe, OidcProvider, QueryExecutor};

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
