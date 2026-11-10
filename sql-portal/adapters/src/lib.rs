//! SQL Portal adapters: implementations of the domain ports, one module per
//! concern. Only `sql-portal` (deployment) and tests depend on this crate.

pub mod auth;
pub mod config;
pub mod grants;
pub mod health;
pub mod sql;
pub mod telemetry;
pub mod tls;
