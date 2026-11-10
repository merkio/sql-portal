use std::sync::Arc;

use async_trait::async_trait;
use sql_portal_domain::health::HealthProbe;
use sql_portal_domain::{OidcProvider, QueryExecutor};

/// Critical probe: can we run `SELECT 1`?
pub struct DatabaseProbe {
    executor: Arc<dyn QueryExecutor>,
}

impl DatabaseProbe {
    pub fn new(executor: Arc<dyn QueryExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl HealthProbe for DatabaseProbe {
    fn name(&self) -> &'static str {
        "database"
    }

    fn critical(&self) -> bool {
        true
    }

    async fn check(&self) -> Result<(), String> {
        self.executor.ping().await.map_err(|e| e.to_string())
    }
}

/// Non-critical probe: is the OIDC provider reachable? An unreachable IdP only
/// breaks SSO logins, so it must not take the whole pod out of rotation.
pub struct OidcProbe {
    provider: Arc<dyn OidcProvider>,
}

impl OidcProbe {
    pub fn new(provider: Arc<dyn OidcProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl HealthProbe for OidcProbe {
    fn name(&self) -> &'static str {
        "oidc"
    }

    fn critical(&self) -> bool {
        false
    }

    async fn check(&self) -> Result<(), String> {
        self.provider.probe().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sql_portal_domain::QueryError;
    use sql_portal_testing::{FakeExecutor, FakeOidc};

    #[tokio::test]
    async fn database_probe_reports_ping_result() {
        let ok = DatabaseProbe::new(Arc::new(FakeExecutor::default()));
        assert!(ok.check().await.is_ok());
        assert!(ok.critical());
        assert_eq!(ok.name(), "database");
        let _ = QueryError::Timeout; // referenced for completeness
    }

    #[tokio::test]
    async fn oidc_probe_is_not_critical() {
        let p = OidcProbe::new(Arc::new(FakeOidc::default()));
        assert!(!p.critical());
        assert!(p.check().await.is_ok());
    }
}
