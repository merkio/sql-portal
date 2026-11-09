//! Live PostgreSQL tests. Ignored by default:
//!
//! ```sh
//! docker run -d --name pg -p 5432:5432 -e POSTGRES_PASSWORD=secret postgres:16
//! TEST_PG_URL=postgres://postgres:secret@localhost:5432/postgres \
//!     cargo test --test postgres -- --ignored
//! ```

use std::time::Duration;

use std::sync::Arc;
use script_portal::domain::ports::QueryExecutor;
use script_portal::domain::{QueryError, QueryRequest, QueryResult, Value};
use script_portal::apapter::sql::connect_database;
use script_portal::domain::tls::TlsOptions;

async fn executor() -> std::sync::Arc<dyn QueryExecutor> {
    let url = std::env::var("TEST_PG_URL").expect("set TEST_PG_URL to run the PostgreSQL tests");
    connect_database(&url, 2, Duration::from_secs(5), &TlsOptions::default())
        .await
        .expect("connects to PostgreSQL")
}

async fn run(sql: &str, params: Vec<(&str, Value)>) -> Result<QueryResult, QueryError> {
    executor()
        .await
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

async fn single_row(sql: &str, params: Vec<(&str, Value)>) -> Vec<Value> {
    let result = run(sql, params)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    assert_eq!(result.rows.len(), 1, "{sql}");
    result.rows.into_iter().next().unwrap()
}

#[tokio::test]
#[ignore = "needs TEST_PG_URL"]
async fn date_and_time_types_arrive_as_text() {
    let row = single_row(
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
#[ignore = "needs TEST_PG_URL"]
async fn numeric_types_keep_integers_and_floats() {
    let row = single_row(
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
#[ignore = "needs TEST_PG_URL"]
async fn uuid_json_array_bytea_and_interval_arrive_as_text() {
    let row = single_row(
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
#[ignore = "needs TEST_PG_URL"]
async fn booleans_text_and_nulls() {
    let row = single_row(
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
#[ignore = "needs TEST_PG_URL"]
async fn parameters_are_cast_to_the_type_the_server_expects() {
    // A text value compares with a date column.
    let by_date = single_row(
        "SELECT DATE '2024-03-01' >= :from",
        vec![("from", Value::Text("2024-02-01".into()))],
    )
    .await;
    assert_eq!(by_date, vec![Value::Bool(true)]);

    // A number parameter takes part in numeric arithmetic.
    let arithmetic = single_row("SELECT NUMERIC '1.5' * :n", vec![("n", Value::Int(2))]).await;
    assert_eq!(arithmetic, vec![Value::Float(3.0)]);

    // A text value cast to uuid by the query itself.
    let uuid = single_row(
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
#[ignore = "needs TEST_PG_URL"]
async fn text_parameters_keep_backslashes_and_limit_binds_integers() {
    let text = single_row("SELECT :t", vec![("t", Value::Text("C:\\x\ty".into()))]).await;
    assert_eq!(text, vec![Value::Text("C:\\x\ty".into())]);

    let limited = run(
        "SELECT g FROM generate_series(1, 10) g ORDER BY g LIMIT :n",
        vec![("n", Value::Int(3))],
    )
    .await
    .unwrap();
    assert_eq!(limited.rows.len(), 3);
    assert_eq!(limited.rows[0], vec![Value::Int(1)]);
}

#[tokio::test]
#[ignore = "needs TEST_PG_URL"]
async fn trailing_semicolons_and_errors_are_handled() {
    let row = single_row("SELECT 1 AS a; ", vec![]).await;
    assert_eq!(row, vec![Value::Int(1)]);

    let missing = run("SELECT * FROM portal_no_such_table", vec![])
        .await
        .unwrap_err();
    assert!(matches!(missing, QueryError::Database(m) if m.contains("portal_no_such_table")));

    let slow = run("SELECT pg_sleep(3)", vec![]).await;
    // The request timeout is 5s here, so this only checks that a long sleep completes.
    assert!(slow.is_ok());
}
