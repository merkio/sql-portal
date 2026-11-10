use std::collections::BTreeSet;

use crate::common::DomainError;

/// A permission required by a script, e.g. `scripts.sales.read`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Permission(String);

impl Permission {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::invalid("permission must not be empty"));
        }
        if trimmed.chars().any(char::is_whitespace) || trimmed.contains('*') {
            return Err(DomainError::invalid(format!(
                "permission '{trimmed}' must not contain whitespace or wildcards"
            )));
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Permissions granted to a principal. Entries may be exact permissions, a
/// prefix wildcard (`scripts.sales.*`) or the global wildcard (`*`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet(BTreeSet<String>);

impl PermissionSet {
    pub fn new<I, S>(grants: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(
            grants
                .into_iter()
                .map(|g| g.into().trim().to_string())
                .filter(|g| !g.is_empty())
                .collect(),
        )
    }

    pub fn all() -> Self {
        Self::new(["*"])
    }

    pub fn grants(&self, required: &Permission) -> bool {
        self.0.iter().any(|grant| {
            if grant == "*" {
                true
            } else if let Some(prefix) = grant.strip_suffix('*') {
                required.as_str().starts_with(prefix)
            } else {
                grant == required.as_str()
            }
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Permission {
        Permission::new(s).unwrap()
    }

    #[test]
    fn rejects_blank_whitespace_and_wildcards() {
        assert!(Permission::new("").is_err());
        assert!(Permission::new("  ").is_err());
        assert!(Permission::new("a b").is_err());
        assert!(Permission::new("a.*").is_err());
        assert_eq!(Permission::new(" a.b ").unwrap().as_str(), "a.b");
    }

    #[test]
    fn exact_grant() {
        let set = PermissionSet::new(["scripts.sales.read"]);
        assert!(set.grants(&p("scripts.sales.read")));
        assert!(!set.grants(&p("scripts.sales.write")));
    }

    #[test]
    fn prefix_wildcard() {
        let set = PermissionSet::new(["scripts.sales.*"]);
        assert!(set.grants(&p("scripts.sales.read")));
        assert!(set.grants(&p("scripts.sales.deep.read")));
        assert!(!set.grants(&p("scripts.admin.read")));
        assert!(!set.grants(&p("scripts.sales")));
    }

    #[test]
    fn global_wildcard() {
        assert!(PermissionSet::all().grants(&p("anything.at.all")));
    }

    #[test]
    fn empty_set_grants_nothing() {
        let set = PermissionSet::default();
        assert!(set.is_empty());
        assert!(!set.grants(&p("x")));
    }

    #[test]
    fn blank_grants_are_dropped() {
        let set = PermissionSet::new(["", "  ", "a"]);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec!["a"]);
    }
}
