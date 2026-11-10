//! Telemetry: Prometheus metrics and tracing setup.

use std::time::Duration;

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub use crate::config::model::LogFormat;

use sql_portal_domain::telemetry::Telemetry;

/// Latency buckets (seconds) shared by HTTP and script histograms.
pub const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// Builds a recorder with histogram buckets configured. Install it globally
/// with `metrics::set_global_recorder`, or just use its handle in tests.
pub fn build_prometheus() -> metrics_exporter_prometheus::PrometheusRecorder {
    PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Suffix("_duration_seconds".into()), LATENCY_BUCKETS)
        .expect("valid buckets")
        .build_recorder()
}

/// Installs the global recorder and returns the render handle.
pub fn install_prometheus() -> anyhow::Result<PrometheusHandle> {
    let recorder = build_prometheus();
    let handle = recorder.handle();
    metrics::set_global_recorder(recorder)
        .map_err(|e| anyhow::anyhow!("metrics recorder already installed: {e}"))?;
    Ok(handle)
}

/// Static, process-level metrics.
pub fn describe_and_init(version: &str, auth_enabled: bool) {
    metrics::describe_counter!("http_requests_total", "Total HTTP requests handled");
    metrics::describe_histogram!(
        "http_request_duration_seconds",
        metrics::Unit::Seconds,
        "HTTP request latency"
    );
    metrics::describe_gauge!(
        "http_requests_in_flight",
        "HTTP requests currently being served"
    );
    metrics::describe_counter!(
        "auth_attempts_total",
        "Authentication attempts by provider and outcome"
    );
    metrics::describe_counter!(
        "script_runs_total",
        "Script executions by script and outcome"
    );
    metrics::describe_histogram!(
        "script_run_duration_seconds",
        metrics::Unit::Seconds,
        "Script execution latency"
    );
    metrics::describe_counter!("script_rows_returned_total", "Rows returned by scripts");
    metrics::describe_gauge!("build_info", "Build information (always 1)");
    metrics::describe_gauge!(
        "auth_enabled",
        "1 when authentication is enabled, 0 when disabled"
    );

    gauge!("build_info", "version" => version.to_string()).set(1.0);
    gauge!("auth_enabled").set(if auth_enabled { 1.0 } else { 0.0 });
}

/// Initialises `tracing` with an env-filter; `RUST_LOG` overrides the config.
pub fn init_tracing(level: &str, format: LogFormat) {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let result = match format {
        LogFormat::Json => registry.with(fmt::layer().json()).try_init(),
        LogFormat::Text => registry.with(fmt::layer().with_ansi(ansi)).try_init(),
    };
    // A subscriber may already be installed (tests); that is fine.
    let _ = result;
}

/// `Telemetry` port implementation backed by the `metrics` facade.
#[derive(Debug, Default, Clone, Copy)]
pub struct MetricsTelemetry;

impl Telemetry for MetricsTelemetry {
    fn auth_attempt(&self, provider: &str, outcome: &str) {
        counter!(
            "auth_attempts_total",
            "provider" => provider.to_string(),
            "outcome" => outcome.to_string()
        )
        .increment(1);
    }

    fn script_run(&self, script: &str, outcome: &str, duration: Duration, rows: usize) {
        counter!(
            "script_runs_total",
            "script" => script.to_string(),
            "outcome" => outcome.to_string()
        )
        .increment(1);
        histogram!(
            "script_run_duration_seconds",
            "script" => script.to_string()
        )
        .record(duration.as_secs_f64());
        if rows > 0 {
            counter!("script_rows_returned_total", "script" => script.to_string())
                .increment(rows as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_emits_expected_series() {
        let recorder = build_prometheus();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            MetricsTelemetry.auth_attempt("ldap", "success");
            MetricsTelemetry.script_run("sales", "success", Duration::from_millis(30), 4);
            describe_and_init("1.2.3", true);
        });
        let text = handle.render();
        assert!(text.contains(r#"auth_attempts_total{provider="ldap",outcome="success"} 1"#));
        assert!(text.contains(r#"script_runs_total{script="sales",outcome="success"} 1"#));
        assert!(text.contains("script_run_duration_seconds_bucket"));
        assert!(text.contains(r#"script_rows_returned_total{script="sales"} 4"#));
        assert!(text.contains(r#"build_info{version="1.2.3"} 1"#));
        assert!(text.contains("auth_enabled 1"));
    }
}
