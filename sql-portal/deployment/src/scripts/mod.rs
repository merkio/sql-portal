//! `scripts:` section → the `ScriptService` with its catalog and access rule.

use std::sync::Arc;

use sql_portal_adapters::config::RuntimeConfig;
use sql_portal_application::{Metrics, RequiredPermissions, ScriptService};
use sql_portal_domain::scripts::repository::ConfigScriptCatalog;
use sql_portal_domain::{QueryExecutor, ScriptAccessRule};
use tracing::info;

/// The fully wired scripts use case. The access rule defaults to
/// "the caller holds every permission the script lists".
pub fn service(
    rt: &RuntimeConfig,
    executor: Arc<dyn QueryExecutor>,
    metrics: Arc<dyn Metrics>,
    access: Option<Arc<dyn ScriptAccessRule>>,
) -> Arc<ScriptService> {
    info!(count = rt.scripts.len(), "scripts loaded");
    let catalog = Arc::new(ConfigScriptCatalog::new(rt.scripts.clone()));
    let access = access.unwrap_or_else(|| Arc::new(RequiredPermissions));
    Arc::new(ScriptService::new(catalog, executor, metrics, access))
}
