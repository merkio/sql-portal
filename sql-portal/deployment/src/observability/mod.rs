//! `observability:` section → tracing, the Prometheus recorder and the `Metrics` port.

use std::sync::Arc;
use std::time::Duration;

use metrics::{counter, gauge, histogram};
use prometheus_metrics::{Buckets, Handle, PrometheusRecorder, DEFAULT_LATENCY_BUCKETS};
use sql_portal_adapters::config::{AppConfig, LogFormat};
use sql_portal_application::Metrics;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::MetricsRender;

/// Every `*_duration_seconds` histogram (HTTP and scripts) uses latency buckets.
const BUCKETS: &[Buckets<'static>] = &[Buckets {
    suffix: "_duration_seconds",
    values: DEFAULT_LATENCY_BUCKETS,
}];

/// Installs the global tracing subscriber and Prometheus recorder. Call once
/// per process; returns the renderer served at the metrics path.
pub fn install(app: &AppConfig) -> anyhow::Result<MetricsRender> {
    init_tracing(&app.observability.log_level, app.observability.log_format);
    let handle = install_metrics()?;
    describe_and_init(env!("CARGO_PKG_VERSION"), app.auth.enabled);
    Ok(Arc::new(move || handle.render()))
}

/// Installs the process-global Prometheus recorder.
pub fn install_metrics() -> anyhow::Result<Handle> {
    prometheus_metrics::install(BUCKETS)
}

/// A recorder that is not installed globally (tests).
pub fn build_recorder() -> (PrometheusRecorder, Handle) {
    prometheus_metrics::build_recorder(BUCKETS).expect("valid buckets")
}

/// Declares every metric and sets the process-level gauges.
pub fn describe_and_init(version: &str, auth_enabled: bool) {
    sql_portal_api::describe_metrics();
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

/// The supplied sink, or Prometheus through the `metrics` facade.
pub fn metrics(supplied: Option<Arc<dyn Metrics>>) -> Arc<dyn Metrics> {
    supplied.unwrap_or_else(|| Arc::new(PrometheusMetrics))
}

/// `Metrics` port implementation backed by the `metrics` facade, exported
/// by the Prometheus recorder.
#[derive(Debug, Default, Clone, Copy)]
pub struct PrometheusMetrics;

impl Metrics for PrometheusMetrics {
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
    fn prometheus_metrics_emit_expected_series() {
        let (recorder, handle) = build_recorder();
        metrics::with_local_recorder(&recorder, || {
            PrometheusMetrics.auth_attempt("ldap", "success");
            PrometheusMetrics.script_run("sales", "success", Duration::from_millis(30), 4);
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
