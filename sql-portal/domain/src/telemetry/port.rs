use std::time::Duration;

/// Business-level telemetry emitted by the application layer.
pub trait Telemetry: Send + Sync {
    fn auth_attempt(&self, provider: &str, outcome: &str);
    fn script_run(&self, script: &str, outcome: &str, duration: Duration, rows: usize);
}

/// Telemetry sink that discards everything (tests, tooling).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTelemetry;

impl Telemetry for NoopTelemetry {
    fn auth_attempt(&self, _provider: &str, _outcome: &str) {}
    fn script_run(&self, _script: &str, _outcome: &str, _duration: Duration, _rows: usize) {}
}
