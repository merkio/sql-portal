use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sql_portal_api::{CheckResult, Readiness, ReadinessReport};

use super::HealthProbe;

/// Liveness is trivial (the process answers); readiness aggregates probes.
pub struct HealthService {
    probes: Vec<Arc<dyn HealthProbe>>,
    probe_timeout: Duration,
}

impl HealthService {
    pub fn new(probes: Vec<Arc<dyn HealthProbe>>) -> Self {
        Self {
            probes,
            probe_timeout: Duration::from_secs(3),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.probe_timeout = timeout;
        self
    }

    /// Runs every probe. Only critical failures make the service not ready.
    pub async fn readiness(&self) -> ReadinessReport {
        let mut checks = Vec::with_capacity(self.probes.len());
        for probe in &self.probes {
            let outcome = tokio::time::timeout(self.probe_timeout, probe.check()).await;
            let (ok, detail) = match outcome {
                Ok(Ok(())) => (true, None),
                Ok(Err(e)) => (false, Some(e)),
                Err(_) => (false, Some("timed out".to_string())),
            };
            checks.push(CheckResult {
                name: probe.name(),
                ok,
                critical: probe.critical(),
                detail,
            });
        }
        let ready = checks.iter().all(|c| c.ok || !c.critical);
        ReadinessReport { ready, checks }
    }
}

#[async_trait]
impl Readiness for HealthService {
    async fn readiness(&self) -> ReadinessReport {
        HealthService::readiness(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProbe {
        name: &'static str,
        ok: bool,
        critical: bool,
        slow: bool,
    }

    impl FakeProbe {
        fn new(name: &'static str, ok: bool, critical: bool) -> Self {
            Self {
                name,
                ok,
                critical,
                slow: false,
            }
        }

        fn slow(name: &'static str) -> Self {
            Self {
                name,
                ok: true,
                critical: true,
                slow: true,
            }
        }
    }

    #[async_trait]
    impl HealthProbe for FakeProbe {
        fn name(&self) -> &'static str {
            self.name
        }

        fn critical(&self) -> bool {
            self.critical
        }

        async fn check(&self) -> Result<(), String> {
            if self.slow {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            if self.ok {
                Ok(())
            } else {
                Err("probe failed".into())
            }
        }
    }

    #[tokio::test]
    async fn ready_when_all_probes_pass() {
        let svc = HealthService::new(vec![Arc::new(FakeProbe::new("db", true, true))]);
        let r = svc.readiness().await;
        assert!(r.ready);
        assert_eq!(r.checks.len(), 1);
    }

    #[tokio::test]
    async fn critical_failure_means_not_ready() {
        let svc = HealthService::new(vec![
            Arc::new(FakeProbe::new("db", false, true)),
            Arc::new(FakeProbe::new("oidc", true, false)),
        ]);
        let r = svc.readiness().await;
        assert!(!r.ready);
        assert_eq!(r.checks[0].detail.as_deref(), Some("probe failed"));
    }

    #[tokio::test]
    async fn non_critical_failure_keeps_service_ready() {
        let svc = HealthService::new(vec![Arc::new(FakeProbe::new("oidc", false, false))]);
        let r = svc.readiness().await;
        assert!(r.ready);
        assert!(!r.checks[0].ok);
    }

    #[tokio::test]
    async fn slow_probe_times_out() {
        let svc = HealthService::new(vec![Arc::new(FakeProbe::slow("db"))])
            .with_timeout(Duration::from_millis(20));
        let r = svc.readiness().await;
        assert!(!r.ready);
        assert_eq!(r.checks[0].detail.as_deref(), Some("timed out"));
    }

    #[tokio::test]
    async fn no_probes_means_ready() {
        let svc = HealthService::new(vec![]);
        let r = svc.readiness().await;
        assert!(r.ready);
    }
}
