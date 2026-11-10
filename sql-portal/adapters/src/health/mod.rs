//! Health probe adapters: concrete checks registered with the application's
//! `HealthService`.

pub mod probes;

pub use probes::{DatabaseProbe, OidcProbe};
