//! The readiness port behind `/readyz`. The composition root implements it.

use async_trait::async_trait;

/// Outcome of one dependency check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: &'static str,
    pub ok: bool,
    pub critical: bool,
    pub detail: Option<String>,
}

/// Whether the service should receive traffic, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessReport {
    pub ready: bool,
    pub checks: Vec<CheckResult>,
}

#[async_trait]
pub trait Readiness: Send + Sync {
    async fn readiness(&self) -> ReadinessReport;
}
