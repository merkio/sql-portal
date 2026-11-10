//! Startup SQL. In-memory databases start empty, so their migrations run on
//! every start; file-backed databases are managed outside the application.

use std::path::{Path, PathBuf};

use anyhow::Context;
use tracing::info;

use super::executor::run_init_files;
use sql_portal_domain::QueryExecutor;

/// Applies every `*.sql` file of `dir` in lexical order. Returns how many ran.
pub async fn apply_migrations(executor: &dyn QueryExecutor, dir: &Path) -> anyhow::Result<usize> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read migrations directory {}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|p| p.extension().is_some_and(|e| e == "sql"));
    files.sort();
    run_init_files(executor, &files).await?;
    Ok(files.len())
}

/// Runs the migrations (when given), then the configured init files. The
/// caller decides whether migrations run; this function never guesses.
pub async fn prepare_database(
    executor: &dyn QueryExecutor,
    migrations: Option<&Path>,
    init_files: &[PathBuf],
) -> anyhow::Result<()> {
    if let Some(dir) = migrations {
        let applied = apply_migrations(executor, dir).await?;
        info!(applied, dir = %dir.display(), "migrations applied");
    }
    run_init_files(executor, init_files).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::SqlExecutor;
    use std::time::Duration;

    #[tokio::test]
    async fn nothing_runs_when_migrations_are_not_given() {
        let db = SqlExecutor::connect("sqlite::memory:", 1, Duration::from_secs(2))
            .await
            .unwrap();
        prepare_database(&db, None, &[]).await.unwrap();
        let tables = db
            .execute(sql_portal_domain::QueryRequest {
                sql: "SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table'".into(),
                params: vec![],
                max_rows: 1,
                timeout: Duration::from_secs(2),
            })
            .await
            .unwrap();
        assert_eq!(tables.rows[0][0], sql_portal_domain::Value::Int(0));
    }

    #[tokio::test]
    async fn applies_migrations_in_name_order_and_skips_other_files() {
        let dir = std::env::temp_dir().join("portal-migrations-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0002_rows.sql"), "INSERT INTO t VALUES (1), (2);").unwrap();
        std::fs::write(dir.join("0001_table.sql"), "CREATE TABLE t (id INTEGER);").unwrap();
        std::fs::write(dir.join("notes.txt"), "not SQL").unwrap();

        let db = SqlExecutor::connect("sqlite::memory:", 1, Duration::from_secs(2))
            .await
            .unwrap();
        let applied = apply_migrations(&db, &dir).await.unwrap();
        assert_eq!(applied, 2);

        let count = db
            .execute(sql_portal_domain::QueryRequest {
                sql: "SELECT COUNT(*) AS n FROM t".into(),
                params: vec![],
                max_rows: 1,
                timeout: Duration::from_secs(2),
            })
            .await
            .unwrap();
        assert_eq!(count.rows[0][0], sql_portal_domain::Value::Int(2));
        std::fs::remove_dir_all(dir).ok();
    }
}
