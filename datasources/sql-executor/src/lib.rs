//! Database-independent contract for SQL dialects.
//!
//! A dialect turns a script written with `:name` placeholders into the syntax
//! of one database and decides how values are bound. Implementations live in
//! the `dialect-*` crates and do no I/O, so they can be tested without a server
//! and reused by any application that talks to that database.
//!
//! To support another database: implement [`Dialect`] in a new crate and
//! execute the [`Statement`] it produces with the driver you use.

pub mod scanner;

pub use scanner::{scan_placeholders, Placeholder};

use std::fmt;

/// A value a dialect binds or a driver returns, independent of the driver.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// A problem found while preparing a statement, e.g. a placeholder without a
/// value. Drivers report these as query errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectError(pub String);

impl fmt::Display for DialectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DialectError {}

/// A statement ready for the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    /// SQL in the dialect's syntax.
    pub sql: String,
    /// Values for the driver's positional binds, in bind order.
    pub bind: Vec<Cell>,
    /// Parameters the server substitutes itself (ClickHouse `param_*`).
    pub query_params: Vec<(String, String)>,
}

/// A rectangular result in neutral cells.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

/// SQL dialect of one database.
pub trait Dialect: Send + Sync {
    /// Short stable name, e.g. `postgres`.
    fn name(&self) -> &'static str;

    /// Rewrites `:name` placeholders for the database and prepares the bound
    /// values. `params` holds the value of each named parameter.
    /// `param_types` are the server-declared parameter types in placeholder
    /// order, when the driver can report them.
    fn prepare(
        &self,
        sql: &str,
        params: &[(String, Cell)],
        param_types: Option<&[String]>,
    ) -> Result<Statement, DialectError>;
}

/// Finds the value of a named parameter.
pub fn lookup<'a>(params: &'a [(String, Cell)], name: &str) -> Result<&'a Cell, DialectError> {
    params
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .ok_or_else(|| DialectError(format!("missing value for ':{name}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_reports_missing_values() {
        let params = vec![("a".to_string(), Cell::Int(1))];
        assert_eq!(lookup(&params, "a").unwrap(), &Cell::Int(1));
        assert!(lookup(&params, "b").unwrap_err().0.contains(":b"));
    }
}
