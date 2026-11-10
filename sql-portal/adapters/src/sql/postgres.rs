//! PostgreSQL adapter on the native driver (not sqlx `Any`). The SQL rules
//! live in the `dialect-postgres` crate; this file does the I/O: describe the
//! statement, bind the prepared values, decode the rows.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::TryStreamExt;
use postgres_dialect::{numeric_cell, wrap_result, Kind, Postgres};
use sql_executor::{Cell, Dialect};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Column, Either, Executor, Row, TypeInfo, ValueRef};

use super::{cells_of, value_of};
use sql_portal_domain::QueryExecutor;
use sql_portal_domain::{QueryError, QueryRequest, QueryResult};

/// Executes script SQL through a native PostgreSQL pool.
pub struct PostgresExecutor {
    pool: PgPool,
}

impl PostgresExecutor {
    pub async fn connect(
        url: &str,
        max_connections: u32,
        acquire_timeout: Duration,
    ) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(acquire_timeout)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// Runs a setup script. The simple protocol accepts several statements.
    pub async fn run_init_script(&self, sql: &str) -> Result<(), QueryError> {
        sqlx::raw_sql(sql)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    async fn fetch(&self, request: &QueryRequest) -> Result<QueryResult, QueryError> {
        let params = cells_of(&request.params);

        // 1. Describe the statement with untyped parameters.
        let plain = Postgres
            .prepare(&request.sql, &params, None)
            .map_err(|e| QueryError::Database(e.0))?;
        let described = (&self.pool).describe(&plain.sql).await.map_err(map_error)?;
        let param_types: Vec<String> = match described.parameters() {
            Some(Either::Left(types)) => types.iter().map(|t| t.name().to_string()).collect(),
            _ => Vec::new(),
        };

        // 2. Prepare again with the declared parameter types (casts).
        let typed = Postgres
            .prepare(&request.sql, &params, Some(&param_types))
            .map_err(|e| QueryError::Database(e.0))?;

        // 3. Wrap the statement so every column decodes.
        let columns: Vec<(String, String)> = described
            .columns()
            .iter()
            .map(|c| (c.name().to_string(), c.type_info().name().to_string()))
            .collect();
        let (statement, kinds) = match wrap_result(&typed.sql, &columns) {
            Some(wrapped) => (wrapped.sql, wrapped.kinds),
            None => (typed.sql.clone(), Vec::new()),
        };

        let mut query = sqlx::query(&statement);
        for cell in &typed.bind {
            query = bind(query, cell);
        }

        let mut result = QueryResult {
            columns: columns.into_iter().map(|(name, _)| name).collect(),
            ..Default::default()
        };
        let mut stream = query.fetch(&self.pool);
        while let Some(row) = stream.try_next().await.map_err(map_error)? {
            if result.rows.len() >= request.max_rows {
                result.truncated = true;
                break;
            }
            result.rows.push(
                kinds
                    .iter()
                    .enumerate()
                    .map(|(i, kind)| decode(&row, i, *kind))
                    .collect(),
            );
        }
        Ok(result)
    }
}

#[async_trait]
impl QueryExecutor for PostgresExecutor {
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, QueryError> {
        match tokio::time::timeout(request.timeout, self.fetch(&request)).await {
            Ok(result) => result,
            Err(_) => Err(QueryError::Timeout),
        }
    }

    async fn ping(&self) -> Result<(), QueryError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    async fn run_script(&self, sql: &str) -> Result<(), QueryError> {
        self.run_init_script(sql).await
    }
}

type PgQuery<'q> = sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>;

fn bind<'q>(query: PgQuery<'q>, cell: &Cell) -> PgQuery<'q> {
    match cell {
        Cell::Null => query.bind(Option::<String>::None),
        Cell::Bool(b) => query.bind(*b),
        Cell::Int(i) => query.bind(*i),
        Cell::Float(f) => query.bind(*f),
        Cell::Text(s) => query.bind(s.clone()),
    }
}

fn decode(row: &PgRow, index: usize, kind: Kind) -> sql_portal_domain::Value {
    let cell = match row.try_get_raw(index) {
        Ok(raw) if !raw.is_null() => match kind {
            Kind::Bool => row.try_get::<bool, _>(index).map(Cell::Bool).ok(),
            Kind::Int => row.try_get::<i64, _>(index).map(Cell::Int).ok(),
            Kind::Float => row.try_get::<f64, _>(index).map(Cell::Float).ok(),
            Kind::Numeric => row
                .try_get::<String, _>(index)
                .ok()
                .map(|s| numeric_cell(&s)),
            Kind::Text => row.try_get::<String, _>(index).map(Cell::Text).ok(),
        },
        _ => None,
    };
    value_of(cell.unwrap_or(Cell::Null))
}

fn map_error(err: sqlx::Error) -> QueryError {
    match err {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
            QueryError::Unavailable(err.to_string())
        }
        other => QueryError::Database(other.to_string()),
    }
}
