//! `access:` section → the `GrantSource` (roles and permissions per identity).

use std::sync::Arc;

use anyhow::Context;
use sql_portal_adapters::config::runtime::{GrantsRuntime, RuntimeConfig};
use sql_portal_adapters::grants::{CachedGrants, RemoteGrants};
use sql_portal_application::LocalGrants;
use sql_portal_domain::GrantSource;
use tracing::info;

/// The local role maps, or the external grants service behind a TTL cache.
pub fn grant_source(
    rt: &RuntimeConfig,
    supplied: Option<Arc<dyn GrantSource>>,
) -> anyhow::Result<Arc<dyn GrantSource>> {
    if let Some(grants) = supplied {
        return Ok(grants);
    }
    Ok(match &rt.grants {
        GrantsRuntime::Local => Arc::new(LocalGrants::new(rt.policy.clone())),
        GrantsRuntime::External(external) => {
            let remote = RemoteGrants::new(
                &external.settings.url,
                external.settings.bearer_token.clone(),
                external.settings.timeout,
                &external.tls,
            )
            .context("cannot configure the grants service")?;
            info!(url = %external.settings.url, "grants loaded from an external service");
            Arc::new(CachedGrants::new(
                Arc::new(remote),
                external.settings.cache_ttl,
            ))
        }
    })
}
