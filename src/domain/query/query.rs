use async_trait::async_trait;

use crate::domain::query::{QueryError, QueryRequest, QueryResult};

/// Executes SQL for a script.
#[async_trait]
pub trait QueryExecutor: Send + Sync {
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, QueryError>;
    async fn ping(&self) -> Result<(), QueryError>;
    /// Runs a setup script (schema, seed data). Not subject to script rules.
    async fn run_script(&self, sql: &str) -> Result<(), QueryError>;
}
