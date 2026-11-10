use std::collections::{BTreeMap, BTreeSet};

use super::grants::Grants;
use super::principal::Principal;
use super::user::User;
use crate::access::PermissionSet;
use crate::common::DomainError;

/// Maps users and their groups to roles, and roles to permissions.
///
/// Matching of group and user names is case-insensitive so that LDAP DNs and
/// IdP group names behave predictably.
#[derive(Debug, Clone, Default)]
pub struct AccessPolicy {
    roles: BTreeMap<String, Vec<String>>,
    group_roles: BTreeMap<String, Vec<String>>,
    user_roles: BTreeMap<String, Vec<String>>,
    default_roles: Vec<String>,
}

impl AccessPolicy {
    /// Builds a policy and verifies that every referenced role is defined.
    pub fn new(
        roles: BTreeMap<String, Vec<String>>,
        group_roles: BTreeMap<String, Vec<String>>,
        user_roles: BTreeMap<String, Vec<String>>,
        default_roles: Vec<String>,
    ) -> Result<Self, DomainError> {
        let normalize = |m: BTreeMap<String, Vec<String>>| {
            m.into_iter()
                .map(|(k, v)| (k.trim().to_lowercase(), v))
                .collect::<BTreeMap<_, _>>()
        };
        let policy = Self {
            roles,
            group_roles: normalize(group_roles),
            user_roles: normalize(user_roles),
            default_roles,
        };
        policy.verify_role_references()?;
        Ok(policy)
    }

    fn verify_role_references(&self) -> Result<(), DomainError> {
        let referenced = self
            .group_roles
            .values()
            .chain(self.user_roles.values())
            .flatten()
            .chain(self.default_roles.iter());
        for role in referenced {
            if !self.roles.contains_key(role) {
                return Err(DomainError::invalid(format!(
                    "role '{role}' is referenced in the access configuration but not defined under 'roles'"
                )));
            }
        }
        Ok(())
    }

    /// Roles and permissions this policy grants to a user.
    pub fn grants(&self, user: &User) -> Grants {
        let mut roles: BTreeSet<String> = self.default_roles.iter().cloned().collect();

        if let Some(assigned) = self.user_roles.get(&user.subject.trim().to_lowercase()) {
            roles.extend(assigned.iter().cloned());
        }
        for group in &user.groups {
            if let Some(assigned) = self.group_roles.get(&group.trim().to_lowercase()) {
                roles.extend(assigned.iter().cloned());
            }
        }

        let permissions = PermissionSet::new(
            roles
                .iter()
                .filter_map(|r| self.roles.get(r))
                .flatten()
                .cloned(),
        );
        Grants { roles, permissions }
    }

    /// The principal for an authenticated user, with the roles and permissions
    /// this policy grants them.
    pub fn resolve(&self, user: User) -> Principal {
        let grants = self.grants(&user);
        Principal {
            user,
            roles: grants.roles,
            permissions: grants.permissions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::identity::AuthProvider;
    use crate::Permission;

    fn user(subject: &str, groups: &[&str]) -> User {
        User {
            subject: subject.into(),
            display_name: subject.into(),
            email: None,
            groups: groups.iter().map(|g| g.to_string()).collect(),
            provider: AuthProvider::Ldap,
        }
    }

    fn policy() -> AccessPolicy {
        let roles = BTreeMap::from([
            ("admin".to_string(), vec!["*".to_string()]),
            ("analyst".to_string(), vec!["scripts.sales.*".to_string()]),
            (
                "viewer".to_string(),
                vec!["scripts.public.read".to_string()],
            ),
        ]);
        let group_roles = BTreeMap::from([(
            "CN=Analysts,OU=Groups".to_string(),
            vec!["analyst".to_string()],
        )]);
        let user_roles = BTreeMap::from([("Alice".to_string(), vec!["admin".to_string()])]);
        AccessPolicy::new(roles, group_roles, user_roles, vec!["viewer".into()]).unwrap()
    }

    #[test]
    fn default_roles_apply_to_everyone() {
        let p = policy().resolve(user("bob", &[]));
        assert!(p.roles.contains("viewer"));
        assert!(p.can(&Permission::new("scripts.public.read").unwrap()));
        assert!(!p.can(&Permission::new("scripts.sales.read").unwrap()));
    }

    #[test]
    fn group_mapping_is_case_insensitive() {
        let p = policy().resolve(user("bob", &["cn=analysts,ou=groups"]));
        assert!(p.roles.contains("analyst"));
        assert!(p.can(&Permission::new("scripts.sales.read").unwrap()));
    }

    #[test]
    fn user_mapping_is_case_insensitive() {
        let p = policy().resolve(user("ALICE", &[]));
        assert!(p.roles.contains("admin"));
        assert!(p.can(&Permission::new("whatever").unwrap()));
    }

    #[test]
    fn unknown_group_grants_nothing_extra() {
        let p = policy().resolve(user("carol", &["cn=other"]));
        assert_eq!(p.roles.len(), 1);
    }

    #[test]
    fn rejects_undefined_role_references() {
        let err = AccessPolicy::new(
            BTreeMap::new(),
            BTreeMap::from([("g".to_string(), vec!["ghost".to_string()])]),
            BTreeMap::new(),
            vec![],
        )
        .unwrap_err();
        assert!(err.to_string().contains("ghost"));

        let err = AccessPolicy::new(
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            vec!["x".into()],
        )
        .unwrap_err();
        assert!(err.to_string().contains("'x'"));
    }
}
