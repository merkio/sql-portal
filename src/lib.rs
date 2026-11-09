//! Script Portal: a config-driven portal that runs SQL scripts behind
//! LDAP / OIDC authorization and renders forms, tables and charts.
//!
//! Layers (dependencies point inwards):
//!
//! ```text
//! presentation  ─┐
//! infrastructure ┼─►  application ─►  domain
//! bootstrap     ─┘
//! ```

pub mod apapter;
pub mod application;
pub mod bootstrap;
pub mod domain;
pub mod infrastructure;
pub mod ui;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
