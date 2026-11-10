//! ClickHouse adapter over the HTTP interface. The SQL rules (typed
//! parameters, result parsing, statement splitting, endpoint URLs) live in the
//! `dialect-clickhouse` crate; this file does the I/O and the server settings.

use std::time::Duration;

use anyhow::bail;
use async_trait::async_trait;
use clickhouse_dialect::{parse_result, split_statements, ClickHouse, Endpoint};
use reqwest::{Client, Url};
use sql_executor::Dialect;

use super::{cells_of, table_result, TlsOptions};
use sql_portal_domain::QueryExecutor;
use sql_portal_domain::{QueryError, QueryRequest, QueryResult};
use tls_config::apply_reqwest;

/// Runs script SQL against a ClickHouse server.
pub struct ClickHouseExecutor {
    client: Client,
    endpoint: Url,
    database: String,
    user: String,
    password: Option<String>,
}

impl ClickHouseExecutor {
    pub fn connect(
        url: &str,
        max_connections: u32,
        acquire_timeout: Duration,
        tls: &TlsOptions,
    ) -> anyhow::Result<Self> {
        let endpoint = Endpoint::parse(url)?;
        if tls.is_set() && !endpoint.is_https() {
            bail!("database.tls needs a clickhouses:// URL");
        }
        let builder = Client::builder()
            .pool_max_idle_per_host(max_connections as usize)
            .connect_timeout(acquire_timeout);
        let client = apply_reqwest(builder, tls)?.build()?;
        Ok(Self {
            client,
            endpoint: Url::parse(&endpoint.base)?,
            database: endpoint.database,
            user: endpoint.user,
            password: endpoint.password,
        })
    }

    /// Sends one statement and returns the response body.
    async fn send(
        &self,
        sql: String,
        mut query: Vec<(String, String)>,
    ) -> Result<String, QueryError> {
        query.push(("database".into(), self.database.clone()));
        let response = self
            .client
            .post(self.endpoint.clone())
            .basic_auth(&self.user, self.password.as_deref())
            .query(&query)
            .body(sql)
            .send()
            .await
            .map_err(map_error)?;
        let status = response.status();
        let text = response.text().await.map_err(map_error)?;
        if status.is_success() {
            Ok(text)
        } else if status.is_server_error() && status.as_u16() != 500 {
            Err(QueryError::Unavailable(text.trim().to_string()))
        } else {
            // ClickHouse reports query errors as HTTP 500 with the message as body.
            Err(QueryError::Database(text.trim().to_string()))
        }
    }

    async fn fetch(&self, request: &QueryRequest) -> Result<QueryResult, QueryError> {
        let statement = ClickHouse
            .prepare(&request.sql, &cells_of(&request.params), None)
            .map_err(|e| QueryError::Database(e.0))?;
        let limit = request.max_rows.saturating_add(1).to_string();
        let mut query = vec![
            ("default_format".to_string(), "JSONCompact".to_string()),
            // Read-only at the server, but settings may still be passed.
            ("readonly".to_string(), "2".to_string()),
            (
                "max_execution_time".to_string(),
                request.timeout.as_secs().max(1).to_string(),
            ),
            // One extra row tells us whether the result was cut short. Blocks
            // are sized to match so the server does not stream a full block.
            ("max_result_rows".to_string(), limit.clone()),
            ("max_block_size".to_string(), limit),
            ("result_overflow_mode".to_string(), "break".to_string()),
        ];
        query.extend(statement.query_params);

        let body = self.send(statement.sql, query).await?;
        let table = parse_result(&body).map_err(|e| QueryError::Database(e.0))?;
        Ok(table_result(table, request.max_rows))
    }
}

#[async_trait]
impl QueryExecutor for ClickHouseExecutor {
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, QueryError> {
        match tokio::time::timeout(request.timeout, self.fetch(&request)).await {
            Ok(result) => result,
            Err(_) => Err(QueryError::Timeout),
        }
    }

    async fn ping(&self) -> Result<(), QueryError> {
        self.execute(QueryRequest {
            sql: "SELECT 1".into(),
            params: vec![],
            max_rows: 1,
            timeout: Duration::from_secs(5),
        })
        .await
        .map(|_| ())
    }

    async fn run_script(&self, sql: &str) -> Result<(), QueryError> {
        // The HTTP interface accepts one statement per request. Setup scripts
        // are not read-only, so they are sent without the readonly setting.
        for statement in split_statements(sql) {
            self.send(statement, vec![]).await?;
        }
        Ok(())
    }
}

fn map_error(err: reqwest::Error) -> QueryError {
    if err.is_timeout() {
        QueryError::Timeout
    } else if err.is_connect() {
        QueryError::Unavailable(err.to_string())
    } else {
        QueryError::Database(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_other_schemes() {
        assert!(ClickHouseExecutor::connect(
            "postgres://x",
            1,
            Duration::from_secs(1),
            &TlsOptions::default()
        )
        .is_err());
    }

    #[test]
    fn tls_options_need_https() {
        let tls = TlsOptions {
            insecure: true,
            ..Default::default()
        };
        assert!(ClickHouseExecutor::connect(
            "clickhouse://ch.local",
            1,
            Duration::from_secs(1),
            &tls
        )
        .is_err());
        assert!(ClickHouseExecutor::connect(
            "clickhouses://ch.local",
            1,
            Duration::from_secs(1),
            &tls
        )
        .is_ok());
    }
}
