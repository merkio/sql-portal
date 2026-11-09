//! Authentication adapters: LDAP, OIDC, and GitHub providers.

pub mod github;
pub mod ldap;
pub mod oidc;

pub use github::GithubAdapter;
pub use ldap::LdapAuthenticator;
pub use oidc::OidcAdapter;
