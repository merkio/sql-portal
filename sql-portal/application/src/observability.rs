//! What the use cases report about themselves. The composition root decides
//! where it goes (Prometheus in `sql-portal`'s `observability` module).

use std::time::Duration;

/// Business-level metrics recorded by the use cases.
pub trait Metrics: Send + Sync {
    /// One login attempt with `provider` (`ldap`, `oidc`, `github`) and its outcome.
    fn auth_attempt(&self, provider: &str, outcome: &str);
    /// One script request with its outcome, duration and rows returned.
    fn script_run(&self, script: &str, outcome: &str, duration: Duration, rows: usize);
}

/// Metrics sink that discards everything (tests, tooling).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMetrics;

impl Metrics for NoopMetrics {
    fn auth_attempt(&self, _provider: &str, _outcome: &str) {}
    fn script_run(&self, _script: &str, _outcome: &str, _duration: Duration, _rows: usize) {}
}

/// Records metric calls for assertions.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecordingMetrics {
    auth: std::sync::Mutex<Vec<(String, String)>>,
    scripts: std::sync::Mutex<Vec<(String, String, usize)>>,
}

#[cfg(test)]
impl RecordingMetrics {
    pub(crate) fn auth_events(&self) -> Vec<(String, String)> {
        self.auth.lock().unwrap().clone()
    }

    pub(crate) fn script_events(&self) -> Vec<(String, String, usize)> {
        self.scripts.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Metrics for RecordingMetrics {
    fn auth_attempt(&self, provider: &str, outcome: &str) {
        self.auth
            .lock()
            .unwrap()
            .push((provider.to_string(), outcome.to_string()));
    }

    fn script_run(&self, script: &str, outcome: &str, _duration: Duration, rows: usize) {
        self.scripts
            .lock()
            .unwrap()
            .push((script.to_string(), outcome.to_string(), rows));
    }
}
