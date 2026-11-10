//! PostgreSQL adapter against a real server (`--features docker`).
//!
//! The tests share one PostgreSQL container (see
//! `sql_portal_integration_tests::containers`); it is removed when the last
//! test of this binary finishes. The queries here are read-only.

#![cfg(feature = "docker")]

use std::sync::Arc;
use std::time::Duration;

use sql_portal_adapters::sql::connect_database;
use sql_portal_domain::{QueryError, QueryExecutor, QueryRequest, QueryResult, Value};
use sql_portal_integration_tests::containers::{Lease, POSTGRES};
use tls_config::TlsOptions;

/// A connection to the shared server. Holding it keeps the container alive.
struct Pg {
    executor: Arc<dyn QueryExecutor>,
    _lease: Arc<Lease>,
}

async fn pg() -> Pg {
    let lease = POSTGRES.lease().await;
    let executor = connect_database(
        &lease.url,
        2,
        Duration::from_secs(5),
        &TlsOptions::default(),
    )
    .await
    .expect("connects to PostgreSQL");
    Pg {
        executor,
        _lease: lease,
    }
}

impl Pg {
    async fn run(&self, sql: &str, params: Vec<(&str, Value)>) -> Result<QueryResult, QueryError> {
        self.executor
            .execute(QueryRequest {
                sql: sql.into(),
                params: params
                    .into_iter()
                    .map(|(n, v)| (n.to_string(), v))
                    .collect(),
                max_rows: 100,
                timeout: Duration::from_secs(5),
            })
            .await
    }

    async fn single_row(&self, sql: &str, params: Vec<(&str, Value)>) -> Vec<Value> {
        let result = self
            .run(sql, params)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
        assert_eq!(result.rows.len(), 1, "{sql}");
        result.rows.into_iter().next().unwrap()
    }
}

#[tokio::test]
async fn date_and_time_types_arrive_as_text() {
    let db = pg().await;
    let row = db.single_row(
        "SELECT DATE '2024-01-02', TIMESTAMP '2024-01-02 03:04:05', TIMESTAMPTZ '2024-01-02 03:04:05+00', TIME '03:04:05'",
        vec![],
    )
    .await;
    assert_eq!(
        row,
        vec![
            Value::Text("2024-01-02".into()),
            Value::Text("2024-01-02 03:04:05".into()),
            Value::Text("2024-01-02 03:04:05+00".into()),
            Value::Text("03:04:05".into()),
        ]
    );
}

#[tokio::test]
async fn numeric_types_keep_integers_and_floats() {
    let db = pg().await;
    let row = db.single_row(
        "SELECT NUMERIC '12.50', NUMERIC '42', 1.5::float4, 2.5::float8, 7::int2, 8::int4, 9::int8",
        vec![],
    )
    .await;
    assert_eq!(
        row,
        vec![
            Value::Float(12.5),
            Value::Int(42),
            Value::Float(1.5),
            Value::Float(2.5),
            Value::Int(7),
            Value::Int(8),
            Value::Int(9),
        ]
    );
}

#[tokio::test]
async fn uuid_json_array_bytea_and_interval_arrive_as_text() {
    let db = pg().await;
    let row = db.single_row(
        "SELECT UUID 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '{\"a\":1}'::json, '[1,2]'::jsonb, ARRAY[1,2,3], '\\x0102'::bytea, INTERVAL '1 day'",
        vec![],
    )
    .await;
    assert_eq!(
        row,
        vec![
            Value::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
            Value::Text("{\"a\":1}".into()),
            Value::Text("[1, 2]".into()),
            Value::Text("{1,2,3}".into()),
            Value::Text("\\x0102".into()),
            Value::Text("1 day".into()),
        ]
    );
}

#[tokio::test]
async fn booleans_text_and_nulls() {
    let db = pg().await;
    let row = db
        .single_row(
            "SELECT true, 'x'::varchar(5), NULL::int, NULL::numeric",
            vec![],
        )
        .await;
    assert_eq!(
        row,
        vec![
            Value::Bool(true),
            Value::Text("x".into()),
            Value::Null,
            Value::Null
        ]
    );
}

#[tokio::test]
async fn parameters_are_cast_to_the_type_the_server_expects() {
    let db = pg().await;
    // A text value compares with a date column.
    let by_date = db
        .single_row(
            "SELECT DATE '2024-03-01' >= :from",
            vec![("from", Value::Text("2024-02-01".into()))],
        )
        .await;
    assert_eq!(by_date, vec![Value::Bool(true)]);

    // A number parameter takes part in numeric arithmetic.
    let arithmetic = db
        .single_row("SELECT NUMERIC '1.5' * :n", vec![("n", Value::Int(2))])
        .await;
    assert_eq!(arithmetic, vec![Value::Float(3.0)]);

    // A text value cast to uuid by the query itself.
    let uuid = db
        .single_row(
            "SELECT :u::uuid",
            vec![(
                "u",
                Value::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
            )],
        )
        .await;
    assert_eq!(
        uuid,
        vec![Value::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())]
    );
}

#[tokio::test]
async fn text_parameters_keep_backslashes_and_limit_binds_integers() {
    let db = pg().await;
    let text = db
        .single_row("SELECT :t", vec![("t", Value::Text("C:\\x\ty".into()))])
        .await;
    assert_eq!(text, vec![Value::Text("C:\\x\ty".into())]);

    let limited = db
        .run(
            "SELECT g FROM generate_series(1, 10) g ORDER BY g LIMIT :n",
            vec![("n", Value::Int(3))],
        )
        .await
        .unwrap();
    assert_eq!(limited.rows.len(), 3);
    assert_eq!(limited.rows[0], vec![Value::Int(1)]);
}

#[tokio::test]
async fn trailing_semicolons_and_errors_are_handled() {
    let db = pg().await;
    let row = db.single_row("SELECT 1 AS a; ", vec![]).await;
    assert_eq!(row, vec![Value::Int(1)]);

    let missing = db
        .run("SELECT * FROM portal_no_such_table", vec![])
        .await
        .unwrap_err();
    assert!(matches!(missing, QueryError::Database(m) if m.contains("portal_no_such_table")));

    let slow = db.run("SELECT pg_sleep(3)", vec![]).await;
    // The request timeout is 5s here, so this only checks that a long sleep completes.
    assert!(slow.is_ok());
}
