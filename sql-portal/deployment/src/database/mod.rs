//! `database:` section → the `QueryExecutor`, with migrations and init files applied.

use std::sync::Arc;

use anyhow::Context;
use sql_portal_adapters::config::RuntimeConfig;
use sql_portal_adapters::sql::{connect_database, prepare_database};
use sql_portal_domain::QueryExecutor;

/// Connects (unless an executor is supplied) and prepares the database.
pub async fn executor(
    rt: &RuntimeConfig,
    supplied: Option<Arc<dyn QueryExecutor>>,
) -> anyhow::Result<Arc<dyn QueryExecutor>> {
    let executor = match supplied {
        Some(executor) => executor,
        None => connect_database(
            &rt.database.url,
            rt.database.max_connections,
            rt.database.acquire_timeout,
            &rt.database.tls,
        )
        .await
        .context("cannot connect to the database")?,
    };
    prepare_database(
        executor.as_ref(),
        rt.database.migrations.as_deref(),
        &rt.database.init_files,
    )
    .await?;
    Ok(executor)
}
