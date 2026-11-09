//! Application configuration: YAML model, loader and mapping to the domain.

pub mod loader;
pub mod mapping;
pub mod model;
pub mod runtime;

pub use loader::{load_from_path, load_from_str, ConfigError};
pub use model::AppConfig;
