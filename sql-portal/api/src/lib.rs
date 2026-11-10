//! SQL Portal HTTP API and web UI on axum: routes, handlers, extractors,
//! middleware, HTML views and the server loop. The only crate that knows axum;
//! the composition root (`sql-portal`) configures and starts it.

pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod readiness;
pub mod render;
pub mod router;
pub mod server;
pub mod state;
pub mod views;

pub use axum::Router;
pub use middleware::describe_metrics;
pub use readiness::{CheckResult, Readiness, ReadinessReport};
pub use router::build_router;
pub use server::serve;
pub use state::{AppState, MetricsRender, RouterSettings, UiSettings};
