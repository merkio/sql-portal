//! SQL Portal adapters: implementations of the domain ports, one module per
//! concern. Only `sql-portal` (deployment) and tests depend on this crate.

pub mod auth;
pub mod config;
pub mod grants;
pub mod sql;
