use async_trait::async_trait;

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
