//! Telemetry infrastructure: Prometheus metrics and tracing.

pub mod telemetry;

pub use telemetry::{
    build_prometheus, describe_and_init, init_tracing, install_prometheus, MetricsTelemetry,
    LATENCY_BUCKETS,
};
