//! `observability:` section → tracing, the Prometheus recorder and the `Telemetry` port.

use std::sync::Arc;

use sql_portal_adapters::config::AppConfig;
use sql_portal_adapters::telemetry::{
    describe_and_init, init_tracing, install_prometheus, MetricsTelemetry,
};
use sql_portal_domain::Telemetry;

use crate::MetricsRender;

/// Installs the global tracing subscriber and Prometheus recorder. Call once
/// per process; returns the renderer served at the metrics path.
pub fn install(app: &AppConfig) -> anyhow::Result<MetricsRender> {
    init_tracing(&app.observability.log_level, app.observability.log_format);
    let handle = install_prometheus()?;
    describe_and_init(env!("CARGO_PKG_VERSION"), app.auth.enabled);
    Ok(Arc::new(move || handle.render()))
}

/// The supplied telemetry, or metrics recorded through the `metrics` facade.
pub fn telemetry(supplied: Option<Arc<dyn Telemetry>>) -> Arc<dyn Telemetry> {
    supplied.unwrap_or_else(|| Arc::new(MetricsTelemetry))
}
