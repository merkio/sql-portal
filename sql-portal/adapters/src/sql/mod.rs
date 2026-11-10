//! SQL execution adapters: `sqlx` (SQLite and PostgreSQL) and ClickHouse over HTTP.

use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use url::Url;

use sql_executor::{Cell, Table};
use sql_portal_domain::QueryExecutor;
use sql_portal_domain::Value;

pub mod clickhouse;
pub mod executor;
pub mod migrations;
pub mod postgres;

pub use clickhouse::ClickHouseExecutor;
pub use executor::{run_init_files, SqlExecutor};
pub use migrations::{apply_migrations, prepare_database};
pub use postgres::PostgresExecutor;

/// Domain value to the dialect's neutral cell.
pub(crate) fn cell_of(value: &Value) -> Cell {
    match value {
        Value::Null => Cell::Null,
        Value::Bool(b) => Cell::Bool(*b),
        Value::Int(i) => Cell::Int(*i),
        Value::Float(f) => Cell::Float(*f),
        Value::Text(s) => Cell::Text(s.clone()),
    }
}

/// Neutral cell back to the domain value.
pub(crate) fn value_of(cell: Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Bool(b) => Value::Bool(b),
        Cell::Int(i) => Value::Int(i),
        Cell::Float(f) => Value::Float(f),
        Cell::Text(s) => Value::Text(s),
    }
}

/// Named parameters as neutral cells, for a dialect's `prepare`.
pub(crate) fn cells_of(params: &[(String, Value)]) -> Vec<(String, Cell)> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), cell_of(value)))
        .collect()
}

/// A dialect table as a query result, truncated to `max_rows`.
pub(crate) fn table_result(table: Table, max_rows: usize) -> sql_portal_domain::QueryResult {
    let truncated = table.rows.len() > max_rows;
    sql_portal_domain::QueryResult {
        columns: table.columns,
        rows: table
            .rows
            .into_iter()
            .take(max_rows)
            .map(|row| row.into_iter().map(value_of).collect())
            .collect(),
        truncated,
    }
}

pub use tls_config::TlsOptions;

/// Picks the adapter from the URL scheme: `clickhouse://` and
/// `clickhouses://` use the ClickHouse HTTP interface, `postgres://` and
/// `sqlite:` use `sqlx`.
pub async fn connect_database(
    url: &str,
    max_connections: u32,
    acquire_timeout: Duration,
    tls: &TlsOptions,
) -> anyhow::Result<Arc<dyn QueryExecutor>> {
    if url.starts_with("clickhouse://") || url.starts_with("clickhouses://") {
        return Ok(Arc::new(ClickHouseExecutor::connect(
            url,
            max_connections,
            acquire_timeout,
            tls,
        )?));
    }
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        let url = postgres_url_with_tls(url, tls)?;
        return Ok(Arc::new(
            PostgresExecutor::connect(&url, max_connections, acquire_timeout).await?,
        ));
    }
    if tls.is_set() {
        bail!("database.tls applies only to postgres:// and clickhouses:// URLs");
    }
    Ok(Arc::new(
        SqlExecutor::connect(url, max_connections, acquire_timeout).await?,
    ))
}

/// Expresses the TLS options as libpq-style URL parameters, which sqlx reads.
/// `ca_file` verifies the chain and the host name; `insecure` disables verification.
pub(crate) fn postgres_url_with_tls(url: &str, tls: &TlsOptions) -> anyhow::Result<String> {
    if !tls.is_set() {
        return Ok(url.to_string());
    }
    if tls.insecure && tls.ca_file.is_some() {
        bail!("database.tls: ca_file and insecure cannot be combined");
    }
    let mut parsed = Url::parse(url)?;
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !matches!(k.as_ref(), "sslmode" | "sslrootcert" | "sslcert" | "sslkey"))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut query = parsed.query_pairs_mut();
        query.clear();
        for (k, v) in &kept {
            query.append_pair(k, v);
        }
        if tls.insecure {
            query.append_pair("sslmode", "require");
        } else if let Some(ca) = &tls.ca_file {
            query
                .append_pair("sslmode", "verify-full")
                .append_pair("sslrootcert", &ca.to_string_lossy());
        }
        if let Some(cert) = &tls.cert_file {
            query.append_pair("sslcert", &cert.to_string_lossy());
        }
        if let Some(key) = &tls.key_file {
            query.append_pair("sslkey", &key.to_string_lossy());
        }
    }
    Ok(parsed.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn no_tls_options_leave_the_url_alone() {
        let url = "postgres://u:p@db/app?sslmode=prefer";
        assert_eq!(
            postgres_url_with_tls(url, &TlsOptions::default()).unwrap(),
            url
        );
    }

    #[test]
    fn ca_file_requests_verified_tls_and_replaces_existing_ssl_params() {
        let tls = TlsOptions {
            ca_file: Some(PathBuf::from("/etc/certs/ca one.pem")),
            ..Default::default()
        };
        let out = postgres_url_with_tls(
            "postgres://u:p@db:5432/app?sslmode=disable&application_name=portal",
            &tls,
        )
        .unwrap();
        let parsed = Url::parse(&out).unwrap();
        let params: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert!(params.contains(&("application_name".into(), "portal".into())));
        assert!(params.contains(&("sslmode".into(), "verify-full".into())));
        assert!(params.contains(&("sslrootcert".into(), "/etc/certs/ca one.pem".into())));
        assert_eq!(params.iter().filter(|(k, _)| k == "sslmode").count(), 1);
    }

    #[test]
    fn insecure_requires_ssl_without_verification() {
        let tls = TlsOptions {
            insecure: true,
            ..Default::default()
        };
        let out = postgres_url_with_tls("postgres://u:p@db/app", &tls).unwrap();
        assert!(out.contains("sslmode=require"));
        assert!(!out.contains("sslrootcert"));
    }

    #[test]
    fn client_identity_is_passed_as_ssl_cert_and_key() {
        let tls = TlsOptions {
            ca_file: Some(PathBuf::from("/tls/ca.crt")),
            cert_file: Some(PathBuf::from("/tls/tls.crt")),
            key_file: Some(PathBuf::from("/tls/tls.key")),
            ..Default::default()
        };
        let out = postgres_url_with_tls("postgres://u@db/app?sslcert=old", &tls).unwrap();
        let parsed = Url::parse(&out).unwrap();
        let get = |name: &str| {
            parsed
                .query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        assert_eq!(get("sslcert").as_deref(), Some("/tls/tls.crt"));
        assert_eq!(get("sslkey").as_deref(), Some("/tls/tls.key"));
        assert_eq!(get("sslmode").as_deref(), Some("verify-full"));
        assert_eq!(
            parsed.query_pairs().filter(|(k, _)| k == "sslcert").count(),
            1
        );
    }

    #[test]
    fn ca_file_and_insecure_conflict() {
        let tls = TlsOptions {
            ca_file: Some(PathBuf::from("ca.pem")),
            insecure: true,
            ..Default::default()
        };
        assert!(postgres_url_with_tls("postgres://db/app", &tls).is_err());
    }
}
