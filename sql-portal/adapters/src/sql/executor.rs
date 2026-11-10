use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::TryStreamExt;
use sqlx::any::{AnyPoolOptions, AnyRow, AnyTypeInfoKind};
use sqlx::{AnyPool, Column, Row, ValueRef};
use tracing::debug;

use sql_executor::{Cell, Dialect};
use sqlite_dialect::Sqlite;

use super::cells_of;
use sql_portal_domain::QueryExecutor;
use sql_portal_domain::{QueryError, QueryRequest, QueryResult, Value};

/// `sqlite::memory:` and `sqlite://...?mode=memory` URLs.
fn is_in_memory(url: &str) -> bool {
    url.starts_with("sqlite:") && (url.contains(":memory:") || url.contains("mode=memory"))
}

/// Executes script SQL through a `sqlx` connection pool.
pub struct SqlExecutor {
    pool: AnyPool,
}

impl SqlExecutor {
    pub async fn connect(
        url: &str,
        max_connections: u32,
        acquire_timeout: Duration,
    ) -> anyhow::Result<Self> {
        if !url.starts_with("sqlite:") {
            anyhow::bail!("SqlExecutor serves sqlite: URLs only");
        }
        let mut options = AnyPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(acquire_timeout);
        if is_in_memory(url) {
            // Every connection to an in-memory SQLite database is a separate,
            // empty database, and closing the last one deletes it. So the pool
            // keeps exactly one connection, never closes it, and never recycles it.
            if max_connections != 1 {
                tracing::warn!(
                    configured = max_connections,
                    "in-memory SQLite uses a single connection; ignoring max_connections"
                );
            }
            options = options
                .max_connections(1)
                .idle_timeout(None)
                .max_lifetime(None);
        }
        sqlx::any::install_default_drivers();
        let pool = options.connect(url).await?;
        Ok(Self { pool })
    }

    /// Runs a (possibly multi-statement) setup script.
    pub async fn run_init_script(&self, sql: &str) -> Result<(), QueryError> {
        sqlx::raw_sql(sql)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    async fn fetch(&self, request: &QueryRequest) -> Result<QueryResult, QueryError> {
        let statement = Sqlite
            .prepare(&request.sql, &cells_of(&request.params), None)
            .map_err(|e| QueryError::Database(e.0))?;
        debug!(sql = %statement.sql, params = statement.bind.len(), "executing script");

        let mut query = sqlx::query(&statement.sql);
        for cell in &statement.bind {
            query = match cell {
                Cell::Null => query.bind(Option::<String>::None),
                Cell::Bool(b) => query.bind(*b),
                Cell::Int(i) => query.bind(*i),
                Cell::Float(f) => query.bind(*f),
                Cell::Text(s) => query.bind(s.clone()),
            };
        }

        let mut stream = query.fetch(&self.pool);
        let mut result = QueryResult::default();
        while let Some(row) = stream.try_next().await.map_err(map_error)? {
            if result.columns.is_empty() {
                result.columns = row.columns().iter().map(|c| c.name().to_string()).collect();
            }
            if result.rows.len() >= request.max_rows {
                result.truncated = true;
                break;
            }
            result.rows.push(decode_row(&row));
        }
        Ok(result)
    }
}

#[async_trait]
impl QueryExecutor for SqlExecutor {
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

fn map_error(err: sqlx::Error) -> QueryError {
    match err {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
            QueryError::Unavailable(err.to_string())
        }
        other => QueryError::Database(other.to_string()),
    }
}

fn decode_row(row: &AnyRow) -> Vec<Value> {
    (0..row.columns().len())
        .map(|i| decode_value(row, i))
        .collect()
}

fn decode_value(row: &AnyRow, i: usize) -> Value {
    match row.try_get_raw(i) {
        Ok(raw) if raw.is_null() => return Value::Null,
        Err(_) => return Value::Null,
        Ok(_) => {}
    }
    let kind = row.column(i).type_info().kind();
    let decoded = match kind {
        AnyTypeInfoKind::Bool => row.try_get::<bool, _>(i).ok().map(Value::Bool),
        AnyTypeInfoKind::SmallInt => row.try_get::<i16, _>(i).ok().map(|v| Value::Int(v.into())),
        AnyTypeInfoKind::Integer => row.try_get::<i32, _>(i).ok().map(|v| Value::Int(v.into())),
        AnyTypeInfoKind::BigInt => row.try_get::<i64, _>(i).ok().map(Value::Int),
        AnyTypeInfoKind::Real => row
            .try_get::<f32, _>(i)
            .ok()
            .map(|v| Value::Float(v.into())),
        AnyTypeInfoKind::Double => row.try_get::<f64, _>(i).ok().map(Value::Float),
        AnyTypeInfoKind::Text => row.try_get::<String, _>(i).ok().map(Value::Text),
        AnyTypeInfoKind::Blob => row
            .try_get::<Vec<u8>, _>(i)
            .ok()
            .map(|b| Value::Text(format!("<{} bytes>", b.len()))),
        AnyTypeInfoKind::Null => None,
    };
    decoded.unwrap_or_else(|| fallback_decode(row, i))
}

/// Dynamically typed columns (SQLite expressions) may report no static type.
fn fallback_decode(row: &AnyRow, i: usize) -> Value {
    if let Ok(v) = row.try_get_unchecked::<i64, _>(i) {
        return Value::Int(v);
    }
    if let Ok(v) = row.try_get_unchecked::<f64, _>(i) {
        return Value::Float(v);
    }
    if let Ok(v) = row.try_get_unchecked::<String, _>(i) {
        return Value::Text(v);
    }
    Value::Null
}

/// Executes the configured init SQL files in order.
pub async fn run_init_files(executor: &dyn QueryExecutor, files: &[PathBuf]) -> anyhow::Result<()> {
    for file in files {
        let sql = std::fs::read_to_string(file)
            .map_err(|e| anyhow::anyhow!("cannot read init file {}: {e}", file.display()))?;
        executor
            .run_script(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("init file {} failed: {e}", file.display()))?;
        tracing::info!(file = %file.display(), "init script executed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn exec() -> SqlExecutor {
        let e = SqlExecutor::connect("sqlite::memory:", 1, Duration::from_secs(2))
            .await
            .unwrap();
        e.run_init_script(
            "CREATE TABLE t (id INTEGER, name TEXT, price REAL, flag INTEGER, note TEXT);
             CREATE TABLE flags (id INTEGER, active BOOLEAN);
             INSERT INTO flags VALUES (1, 1);
             INSERT INTO t VALUES (1, 'a', 1.5, 1, NULL), (2, 'b', 2.5, 0, 'x'), (3, 'c', 3.5, 1, NULL);",
        )
        .await
        .unwrap();
        e
    }

    fn req(sql: &str, params: Vec<(&str, Value)>, max_rows: usize) -> QueryRequest {
        QueryRequest {
            sql: sql.into(),
            params: params
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect(),
            max_rows,
            timeout: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn selects_typed_rows() {
        let e = exec().await;
        let r = e
            .execute(req(
                "SELECT id, name, price, note FROM t ORDER BY id",
                vec![],
                10,
            ))
            .await
            .unwrap();
        assert_eq!(r.columns, vec!["id", "name", "price", "note"]);
        assert_eq!(r.rows.len(), 3);
        assert_eq!(r.rows[0][0], Value::Int(1));
        assert_eq!(r.rows[0][1], Value::Text("a".into()));
        assert_eq!(r.rows[0][2], Value::Float(1.5));
        assert_eq!(r.rows[0][3], Value::Null);
        assert_eq!(r.rows[1][3], Value::Text("x".into()));
        assert!(!r.truncated);
    }

    #[tokio::test]
    async fn computed_columns_decode() {
        let e = exec().await;
        let r = e
            .execute(req(
                "SELECT COUNT(*) AS n, SUM(price) AS s, 'lit' AS l, 1 + 1 AS two FROM t",
                vec![],
                10,
            ))
            .await
            .unwrap();
        assert_eq!(r.rows[0][0], Value::Int(3));
        assert_eq!(r.rows[0][1], Value::Float(7.5));
        assert_eq!(r.rows[0][2], Value::Text("lit".into()));
        assert_eq!(r.rows[0][3], Value::Int(2));
    }

    #[tokio::test]
    async fn binds_parameters_by_name_including_repeats() {
        let e = exec().await;
        let r = e
            .execute(req(
                "SELECT id FROM t WHERE id >= :lo AND id <= :hi AND name <> :skip ORDER BY id",
                vec![
                    ("lo", Value::Int(1)),
                    ("hi", Value::Int(3)),
                    ("skip", Value::Text("b".into())),
                ],
                10,
            ))
            .await
            .unwrap();
        let ids: Vec<_> = r.rows.iter().map(|row| row[0].clone()).collect();
        assert_eq!(ids, vec![Value::Int(1), Value::Int(3)]);
    }

    #[tokio::test]
    async fn null_and_float_and_bool_params() {
        let e = exec().await;
        let r = e
            .execute(req(
                "SELECT :n IS NULL AS is_null, :f * 2 AS doubled",
                vec![("n", Value::Null), ("f", Value::Float(1.25))],
                10,
            ))
            .await
            .unwrap();
        assert_eq!(r.rows[0][0], Value::Int(1));
        assert_eq!(r.rows[0][1], Value::Float(2.5));
    }

    #[tokio::test]
    async fn sql_injection_attempts_are_just_data() {
        let e = exec().await;
        let r = e
            .execute(req(
                "SELECT COUNT(*) FROM t WHERE name = :name",
                vec![("name", Value::Text("a' OR '1'='1".into()))],
                10,
            ))
            .await
            .unwrap();
        assert_eq!(r.rows[0][0], Value::Int(0));
        // The table is still there.
        let r = e.execute(req("SELECT * FROM t", vec![], 10)).await;
        assert!(r.is_ok(), "{r:?}");
    }

    #[tokio::test]
    async fn truncates_to_max_rows() {
        let e = exec().await;
        let r = e
            .execute(req("SELECT id FROM t ORDER BY id", vec![], 2))
            .await
            .unwrap();
        assert_eq!(r.rows.len(), 2);
        assert!(r.truncated);
    }

    #[tokio::test]
    async fn empty_result_has_no_rows() {
        let e = exec().await;
        let r = e
            .execute(req("SELECT id FROM t WHERE id > 100", vec![], 10))
            .await
            .unwrap();
        assert!(r.rows.is_empty());
        assert!(!r.truncated);
    }

    #[tokio::test]
    async fn database_errors_are_mapped() {
        let e = exec().await;
        let err = e
            .execute(req("SELECT * FROM missing_table", vec![], 10))
            .await
            .unwrap_err();
        assert!(matches!(err, QueryError::Database(_)));
    }

    #[tokio::test]
    async fn missing_parameter_value_is_an_error() {
        let e = exec().await;
        let err = e.execute(req("SELECT :x", vec![], 10)).await.unwrap_err();
        assert!(matches!(err, QueryError::Database(m) if m.contains(":x")));
    }

    #[tokio::test]
    async fn columns_of_unsupported_types_need_a_cast() {
        let e = exec().await;
        // sqlx's Any driver cannot map SQLite BOOLEAN columns (documented limitation)...
        let err = e
            .execute(req("SELECT active FROM flags", vec![], 10))
            .await
            .unwrap_err();
        assert!(matches!(err, QueryError::Database(m) if m.contains("active")));
        // ...but an explicit CAST in the script works.
        let r = e
            .execute(req(
                "SELECT CAST(active AS INTEGER) AS active FROM flags",
                vec![],
                10,
            ))
            .await
            .unwrap();
        assert_eq!(r.rows[0][0], Value::Int(1));
    }

    #[tokio::test]
    async fn ping_works() {
        assert!(exec().await.ping().await.is_ok());
    }

    #[tokio::test]
    async fn rejects_unsupported_scheme() {
        assert!(SqlExecutor::connect("mysql://x", 1, Duration::from_secs(1))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn in_memory_database_keeps_one_connection_whatever_is_configured() {
        let db = SqlExecutor::connect("sqlite::memory:", 5, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(db.pool.options().get_max_connections(), 1);
        assert_eq!(db.pool.options().get_idle_timeout(), None);
        assert_eq!(db.pool.options().get_max_lifetime(), None);
    }

    #[tokio::test]
    async fn statements_see_each_others_tables_with_a_larger_configured_pool() {
        // Regression: with several connections, a table created by one statement
        // was missing for the next, which failed with "no such table".
        let db = SqlExecutor::connect("sqlite::memory:", 5, Duration::from_secs(2))
            .await
            .unwrap();
        db.run_init_script("CREATE TABLE sales (x INTEGER);")
            .await
            .unwrap();
        db.run_init_script("INSERT INTO sales VALUES (1), (2);")
            .await
            .unwrap();
        let rows = db
            .execute(QueryRequest {
                sql: "SELECT COUNT(*) AS n FROM sales".into(),
                params: vec![],
                max_rows: 1,
                timeout: Duration::from_secs(2),
            })
            .await
            .unwrap();
        assert_eq!(rows.rows[0][0], Value::Int(2));
    }
}
