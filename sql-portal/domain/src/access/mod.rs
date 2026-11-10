//! Permissions: the vocabulary of what may be done. Pure value objects.
//! Identity: who a user is and their roles and permissions.
//! Authentication: how users prove their identity.
//! Grants: how users get permissions.

pub mod auth;
pub mod grants;
pub mod identity;
pub mod permission;
pub mod script;

pub use auth::{
    CredentialAuthenticator, GithubLogin, OidcAuthorization, OidcPending, OidcProvider,
};
pub use grants::{GrantSource, LocalGrants};
pub use identity::{AccessPolicy, AuthError, AuthProvider, Grants, Principal, User};
pub use permission::{Permission, PermissionSet};
pub use script::ScriptAccessRule;
