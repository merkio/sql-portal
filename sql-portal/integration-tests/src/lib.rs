//! Integration and end-to-end tests of the SQL Portal.
//!
//! Two kinds of test binaries live in `tests/`:
//!
//! * **In-process** tests drive the real router through [`harness`] with fake
//!   identity providers and in-memory SQLite. They need no Docker and run
//!   with a plain `cargo test`.
//! * **Container** tests (`--features docker`) start real services through
//!   [testcontainers](https://docs.rs/testcontainers): PostgreSQL and
//!   ClickHouse for the datasource adapters, and the portal's own Docker image
//!   for end-to-end checks. Every container is stopped and removed when the
//!   test that uses it ends, including when the test panics. See
//!   [`containers`].

pub mod harness;

#[cfg(feature = "docker")]
pub mod containers;

/// The repository root, where `config/` lives.
pub const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
