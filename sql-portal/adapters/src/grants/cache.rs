//! Local cache in front of a permission source.
//!
//! A lookup is answered from the cache while the entry is younger than the
//! TTL. Otherwise the inner source is asked. If it fails, an entry younger than
//! twice the TTL is used, so a short outage does not lock everyone out, but
//! revocations still take effect within two TTLs. Failures are never cached.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::warn;

use sql_portal_domain::GrantSource;
use sql_portal_domain::{AuthError, Grants, User};

/// Upper bound on cached identities; the cache is cleared when it is full.
const MAX_ENTRIES: usize = 10_000;

pub struct CachedGrants {
    inner: Arc<dyn GrantSource>,
    ttl: Duration,
    entries: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    grants: Grants,
    fetched: Instant,
}

impl CachedGrants {
    pub fn new(inner: Arc<dyn GrantSource>, ttl: Duration) -> Self {
        Self {
            inner,
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Identity and groups decide the grants, so both are part of the key.
    fn key(user: &User) -> String {
        let mut groups: Vec<String> = user
            .groups
            .iter()
            .map(|g| g.trim().to_lowercase())
            .collect();
        groups.sort();
        groups.dedup();
        format!(
            "{}|{}",
            user.subject.trim().to_lowercase(),
            groups.join(",")
        )
    }

    fn stale_limit(&self) -> Duration {
        self.ttl * 2
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        // A panic elsewhere must not disable authorization; the map stays usable.
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn store(&self, key: String, grants: Grants) {
        let limit = self.stale_limit();
        let mut entries = self.entries();
        if entries.len() >= MAX_ENTRIES {
            entries.retain(|_, e| e.fetched.elapsed() < limit);
            if entries.len() >= MAX_ENTRIES {
                entries.clear();
            }
        }
        entries.insert(
            key,
            Entry {
                grants,
                fetched: Instant::now(),
            },
        );
    }
}

#[async_trait]
impl GrantSource for CachedGrants {
    async fn grants_for(&self, user: &User) -> Result<Grants, AuthError> {
        let key = Self::key(user);
        let cached = self
            .entries()
            .get(&key)
            .map(|e| (e.grants.clone(), e.fetched.elapsed()));

        if let Some((grants, age)) = &cached {
            if *age < self.ttl {
                return Ok(grants.clone());
            }
        }

        match self.inner.grants_for(user).await {
            Ok(grants) => {
                self.store(key, grants.clone());
                Ok(grants)
            }
            Err(e) => match cached {
                Some((grants, age)) if age < self.stale_limit() => {
                    warn!(error = %e, subject = %user.subject, "permission source unavailable; using cached grants");
                    Ok(grants)
                }
                _ => Err(e),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sql_portal_domain::{Permission, PermissionSet};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Counts lookups and can be switched into failure.
    struct Source {
        calls: AtomicUsize,
        fail: AtomicBool,
    }

    #[async_trait]
    impl GrantSource for Source {
        async fn grants_for(&self, _: &User) -> Result<Grants, AuthError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(AuthError::ProviderUnavailable("down".into()));
            }
            Ok(Grants {
                roles: Default::default(),
                permissions: PermissionSet::new(["scripts.sales.read".to_string()]),
            })
        }
    }

    fn user(subject: &str, groups: &[&str]) -> User {
        User {
            subject: subject.into(),
            display_name: subject.into(),
            email: None,
            groups: groups.iter().map(|g| g.to_string()).collect(),
            provider: sql_portal_domain::AuthProvider::Oidc,
        }
    }

    fn cached(ttl: Duration) -> (CachedGrants, Arc<Source>) {
        let source = Arc::new(Source {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        });
        (CachedGrants::new(source.clone(), ttl), source)
    }

    #[tokio::test]
    async fn fresh_entries_do_not_reach_the_source() {
        let (cache, source) = cached(Duration::from_secs(60));
        let alice = user("alice", &["Analysts"]);
        let first = cache.grants_for(&alice).await.unwrap();
        let second = cache.grants_for(&alice).await.unwrap();
        assert_eq!(first, second);
        assert!(first
            .permissions
            .grants(&Permission::new("scripts.sales.read").unwrap()));
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn group_membership_is_part_of_the_key() {
        let (cache, source) = cached(Duration::from_secs(60));
        cache.grants_for(&user("alice", &["a"])).await.unwrap();
        cache.grants_for(&user("ALICE", &["A"])).await.unwrap();
        cache.grants_for(&user("alice", &["a", "b"])).await.unwrap();
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn expired_entries_are_refreshed_from_the_source() {
        let (cache, source) = cached(Duration::from_millis(20));
        cache.grants_for(&user("bob", &[])).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        cache.grants_for(&user("bob", &[])).await.unwrap();
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn outage_is_bridged_by_recent_entries_only() {
        let (cache, source) = cached(Duration::from_millis(50));
        cache.grants_for(&user("carol", &[])).await.unwrap();
        source.fail.store(true, Ordering::SeqCst);

        // Past the TTL but within twice the TTL: the cached grants are used.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(cache.grants_for(&user("carol", &[])).await.is_ok());

        // Beyond twice the TTL: the entry is too old to trust.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(cache.grants_for(&user("carol", &[])).await.is_err());
    }

    #[tokio::test]
    async fn failures_with_nothing_cached_are_errors_and_not_cached() {
        let (cache, source) = cached(Duration::from_secs(60));
        source.fail.store(true, Ordering::SeqCst);
        assert!(cache.grants_for(&user("dave", &[])).await.is_err());
        source.fail.store(false, Ordering::SeqCst);
        assert!(cache.grants_for(&user("dave", &[])).await.is_ok());
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    }
}
