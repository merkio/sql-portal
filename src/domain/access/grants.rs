use async_trait::async_trait;

use crate::domain::access::identity::{AuthError, Grants, User};
use crate::domain::{AccessPolicy};

/// Where roles and permissions come from: the local access configuration or an
/// external service. Failures must be reported, never answered with a guess.
#[async_trait]
pub trait GrantSource: Send + Sync {
    async fn grants_for(&self, user: &User) -> Result<Grants, AuthError>;
}

/// Grants computed from the local access configuration.
pub struct LocalGrants {
    policy: AccessPolicy,
}

impl LocalGrants {
    pub fn new(policy: AccessPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl GrantSource for LocalGrants {
    async fn grants_for(&self, user: &User) -> Result<Grants, AuthError> {
        Ok(self.policy.grants(user))
    }
}
