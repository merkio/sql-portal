//! ClickHouse adapter against a real server (`--features docker`).
//!
//! The tests share one ClickHouse container (see
//! `sql_portal_integration_tests::containers`); each test works in its own
//! database, so tables never collide. The container is removed when the last
//! test of this binary finishes.

#![cfg(feature = "docker")]

use std::sync::Arc;
use std::time::Duration;

use sql_portal_adapters::sql::connect_database;
use sql_portal_domain::{QueryError, QueryExecutor, QueryRequest, Value};
use sql_portal_integration_tests::containers::{unique_name, Lease, CLICKHOUSE};
use tls_config::TlsOptions;

/// An executor bound to a fresh database. Holding it keeps the container alive.
struct Ch {
    executor: Arc<dyn QueryExecutor>,
    _lease: Arc<Lease>,
}

async fn connect(url: &str) -> Arc<dyn QueryExecutor> {
    connect_database(url, 2, Duration::from_secs(5), &TlsOptions::default())
        .await
        .expect("connects to ClickHouse")
}

async fn clickhouse() -> Ch {
    let lease = CLICKHOUSE.lease().await;
    let database = unique_name("it").replace('-', "_");
    connect(&lease.url)
        .await
        .run_script(&format!("CREATE DATABASE {database}"))
        .await
        .expect("creates the test database");
    let url = format!("{}/{database}", lease.url.rsplit_once('/').unwrap().0);
    Ch {
        executor: connect(&url).await,
        _lease: lease,
    }
}

fn request(
    sql: &str,
    params: Vec<(&str, Value)>,
    max_rows: usize,
    timeout: Duration,
) -> QueryRequest {
    QueryRequest {
        sql: sql.into(),
        params: params
            .into_iter()
            .map(|(n, v)| (n.to_string(), v))
            .collect(),
        max_rows,
        timeout,
    }
}

#[tokio::test]
async fn typed_parameters_and_nulls_round_trip() {
    let db = clickhouse().await;
    let ex = &db.executor;
    ex.ping().await.unwrap();

    let result = ex
        .execute(request(
            "SELECT :n AS n, :text AS text, :flag AS flag, :gone AS gone, :f AS f",
            vec![
                ("n", Value::Int(7)),
                ("text", Value::Text("it's \"quoted\"".into())),
                ("flag", Value::Bool(true)),
                ("gone", Value::Null),
                ("f", Value::Float(2.5)),
            ],
            10,
            Duration::from_secs(5),
        ))
        .await
        .unwrap();

    assert_eq!(result.columns, vec!["n", "text", "flag", "gone", "f"]);
    assert_eq!(
        result.rows,
        vec![vec![
            Value::Int(7),
            Value::Text("it's \"quoted\"".into()),
            Value::Bool(true),
            Value::Null,
            Value::Float(2.5),
        ]]
    );
}

#[tokio::test]
async fn setup_script_then_query_with_truncation() {
    let db = clickhouse().await;
    let ex = &db.executor;
    ex.run_script(
        "DROP TABLE IF EXISTS portal_it;\n\
         -- seed data; the semicolon in this comment must not split anything\n\
         CREATE TABLE portal_it (id Int64, label String) ENGINE = Memory;\n\
         INSERT INTO portal_it SELECT number, concat('row;', toString(number)) FROM numbers(50);",
    )
    .await
    .unwrap();

    let result = ex
        .execute(request(
            "SELECT id, label FROM portal_it WHERE id >= :min ORDER BY id",
            vec![("min", Value::Int(10))],
            5,
            Duration::from_secs(5),
        ))
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 5);
    assert!(result.truncated);
    assert_eq!(result.rows[0][1], Value::Text("row;10".into()));

    let all = ex
        .execute(request(
            "SELECT id FROM portal_it",
            vec![],
            100,
            Duration::from_secs(5),
        ))
        .await
        .unwrap();
    assert_eq!(all.rows.len(), 50);
    assert!(!all.truncated);

    ex.run_script("DROP TABLE portal_it").await.unwrap();
}

#[tokio::test]
async fn scripts_are_read_only_at_the_server() {
    let db = clickhouse().await;
    let ex = &db.executor;
    let err = ex
        .execute(request(
            "CREATE TABLE portal_should_not_exist (a Int32) ENGINE = Memory",
            vec![],
            10,
            Duration::from_secs(5),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, QueryError::Database(_)), "{err:?}");
}

#[tokio::test]
async fn slow_queries_time_out() {
    let db = clickhouse().await;
    let ex = &db.executor;
    let err = ex
        .execute(request(
            "SELECT sleep(3)",
            vec![],
            10,
            Duration::from_secs(1),
        ))
        .await
        .unwrap_err();
    assert_eq!(err, QueryError::Timeout);
}

#[tokio::test]
async fn missing_table_is_a_database_error() {
    let db = clickhouse().await;
    let ex = &db.executor;
    let err = ex
        .execute(request(
            "SELECT * FROM portal_no_such_table",
            vec![],
            10,
            Duration::from_secs(5),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, QueryError::Database(m) if m.contains("portal_no_such_table")));
}

#[tokio::test]
async fn text_parameters_keep_backslashes_tabs_and_null_markers() {
    let db = clickhouse().await;
    let ex = &db.executor;
    for text in [
        "C:\\temp\\new\tline\nend",
        "\\N",
        "it's \"quoted\"",
        "",
        "naïve ✓",
    ] {
        let result = ex
            .execute(request(
                "SELECT :t AS v",
                vec![("t", Value::Text(text.into()))],
                10,
                Duration::from_secs(5),
            ))
            .await
            .unwrap();
        assert_eq!(
            result.rows,
            vec![vec![Value::Text(text.into())]],
            "{text:?}"
        );
    }
}

#[tokio::test]
async fn named_parameters_work_in_common_sql_positions() {
    let db = clickhouse().await;
    let ex = &db.executor;
    let check = |sql: &'static str, params: Vec<(&'static str, Value)>, expected: Value| {
        let ex = ex.clone();
        async move {
            let result = ex
                .execute(request(sql, params, 10, Duration::from_secs(5)))
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
            assert_eq!(result.rows, vec![vec![expected]], "{sql}");
        }
    };

    check("SELECT :n + 1", vec![("n", Value::Int(41))], Value::Int(42)).await;
    check(
        "SELECT :f * 2",
        vec![("f", Value::Float(1.25))],
        Value::Float(2.5),
    )
    .await;
    check(
        "SELECT :b",
        vec![("b", Value::Bool(false))],
        Value::Bool(false),
    )
    .await;
    check(
        "SELECT isNaN(:f)",
        vec![("f", Value::Float(f64::NAN))],
        Value::Int(1),
    )
    .await; // UInt8 in ClickHouse
    check("SELECT :x", vec![("x", Value::Null)], Value::Null).await;
    check(
        "SELECT CAST(:p AS Int32)",
        vec![("p", Value::Int(7))],
        Value::Int(7),
    )
    .await;
    check(
        "SELECT :p::String",
        vec![("p", Value::Int(7))],
        Value::Text("7".into()),
    )
    .await;
    // Repeated names reuse one value; placeholders in literals and comments are not parameters.
    check(
        "SELECT :p + :p -- :ignored",
        vec![("p", Value::Int(4))],
        Value::Int(8),
    )
    .await;
    check(
        "SELECT concat(':p', toString(:p))",
        vec![("p", Value::Int(1))],
        Value::Text(":p1".into()),
    )
    .await;
}

#[tokio::test]
async fn named_parameters_filter_limit_and_dates() {
    let db = clickhouse().await;
    let ex = &db.executor;
    ex.run_script(
        "DROP TABLE IF EXISTS portal_params;\n\
         CREATE TABLE portal_params (d Date, n Int64) ENGINE = Memory;\n\
         INSERT INTO portal_params VALUES ('2024-01-01', 1), ('2024-02-15', 2), ('2024-03-01', 3), ('2024-04-01', 4);",
    )
    .await
    .unwrap();

    // A date given as text compares against a Date column.
    let by_date = ex
        .execute(request(
            "SELECT n FROM portal_params WHERE d >= :from ORDER BY n",
            vec![("from", Value::Text("2024-02-01".into()))],
            10,
            Duration::from_secs(5),
        ))
        .await
        .unwrap();
    assert_eq!(by_date.rows.len(), 3);

    let limited = ex
        .execute(request(
            "SELECT n FROM portal_params ORDER BY n LIMIT :limit",
            vec![("limit", Value::Int(2))],
            10,
            Duration::from_secs(5),
        ))
        .await
        .unwrap();
    assert_eq!(limited.rows, vec![vec![Value::Int(1)], vec![Value::Int(2)]]);

    ex.run_script("DROP TABLE portal_params").await.unwrap();
}

#[tokio::test]
async fn common_column_types_decode_to_portal_values() {
    let db = clickhouse().await;
    let ex = &db.executor;
    let result = ex
        .execute(request(
            "SELECT toDate('2024-01-02') AS d, toDateTime('2024-01-02 03:04:05') AS dt, \
             toDateTime64('2024-01-02 03:04:05.123', 3) AS dt64, toDecimal64(12.5, 2) AS dec, \
             toUUID('a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11') AS u, [1, 2, 3] AS arr, \
             CAST(NULL AS Nullable(String)) AS nn, CAST('a' AS Enum8('a' = 1)) AS e, \
             toFloat32(1.5) AS f32, map('k', 1) AS m, tuple(1, 'x') AS t, toIPv4('10.0.0.1') AS ip, \
             toLowCardinality('lc') AS lc, toInt128(5) AS i128, toUInt64(18446744073709551615) AS big, \
             toInt64(-7) AS i64, 2.0 AS f64, toNullable(3) AS ni, toFixedString('ab', 4) AS fs",
            vec![],
            5,
            Duration::from_secs(5),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.rows[0],
        vec![
            Value::Text("2024-01-02".into()),
            Value::Text("2024-01-02 03:04:05".into()),
            Value::Text("2024-01-02 03:04:05.123".into()),
            Value::Float(12.5),
            Value::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
            Value::Text("[1,2,3]".into()),
            Value::Null,
            Value::Text("a".into()),
            Value::Float(1.5),
            Value::Text("{\"k\":1}".into()),
            Value::Text("[1,\"x\"]".into()),
            Value::Text("10.0.0.1".into()),
            Value::Text("lc".into()),
            Value::Int(5),
            Value::Text("18446744073709551615".into()),
            Value::Int(-7),
            Value::Float(2.0),
            Value::Int(3),
            Value::Text("ab".into()),
        ]
    );
}
