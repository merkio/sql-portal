//! SQL Portal Presentation Layer
//!
//! Axum routes, handlers, extractors, views, and middleware.

pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod readiness;
pub mod render;
pub mod router;
pub mod state;
pub mod views;

pub use middleware::describe_metrics;
pub use readiness::{CheckResult, Readiness, ReadinessReport};
pub use router::build_router;
pub use state::{AppState, MetricsRender, RouterSettings, UiSettings};
