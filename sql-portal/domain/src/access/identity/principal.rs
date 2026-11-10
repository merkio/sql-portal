use std::collections::BTreeSet;

use crate::access::{Permission, PermissionSet};

use super::user::{AuthProvider, User};

/// An authenticated user together with the roles and permissions that the
/// access policy resolved for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub user: User,
    pub roles: BTreeSet<String>,
    pub permissions: PermissionSet,
}

impl Principal {
    pub fn can(&self, permission: &Permission) -> bool {
        self.permissions.grants(permission)
    }

    /// `true` when every permission is granted (an empty slice is always allowed).
    pub fn can_all(&self, permissions: &[Permission]) -> bool {
        permissions.iter().all(|p| self.can(p))
    }

    /// The principal used when authentication is disabled: full access.
    pub fn anonymous_admin() -> Self {
        Self {
            user: User {
                subject: "anonymous".into(),
                display_name: "Anonymous".into(),
                email: None,
                groups: vec![],
                provider: AuthProvider::Disabled,
            },
            roles: BTreeSet::new(),
            permissions: PermissionSet::all(),
        }
    }
}
