use std::time::Duration;

use crate::common::Value;

/// What the application asks the database adapter to run.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryRequest {
    /// SQL text with `:name` placeholders.
    pub sql: String,
    /// Bound parameters by name.
    pub params: Vec<(String, Value)>,
    pub max_rows: usize,
    pub timeout: Duration,
}

/// Tabular result of a script.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// `true` when more rows existed than `max_rows`.
    pub truncated: bool,
}

impl QueryResult {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}
