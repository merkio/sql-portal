use std::collections::BTreeSet;

use crate::domain::access::PermissionSet;

/// What an identity is granted: its roles and the permissions they carry.
/// Produced by a permission source, local or external.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    pub roles: BTreeSet<String>,
    pub permissions: PermissionSet,
}
