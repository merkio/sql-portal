//! Presentation layer: HTTP routing, handlers and HTML views (axum).

pub mod extractors;
pub mod handlers;
pub mod middleware;
pub mod render;
pub mod router;
pub mod state;
pub mod views;

pub use router::build_router;
pub use state::{AppState, RouterSettings, UiSettings};
