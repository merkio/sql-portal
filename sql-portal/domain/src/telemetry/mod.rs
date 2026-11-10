//! Business-level telemetry traits and implementations.

pub mod port;

pub use port::{NoopTelemetry, Telemetry};
