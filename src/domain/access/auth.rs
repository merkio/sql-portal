use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::access::identity::{AuthError, User};

/// Verifies a username/password pair against a directory (LDAP).
#[async_trait]
pub trait CredentialAuthenticator: Send + Sync {
    async fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError>;
}

/// Transient values that must survive the redirect to the identity provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidcPending {
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcAuthorization {
    pub redirect_url: String,
    pub pending: OidcPending,
}

/// An OpenID Connect relying-party flow (authorization code + PKCE).
#[async_trait]
pub trait OidcProvider: Send + Sync {
    async fn begin(&self) -> Result<OidcAuthorization, AuthError>;
    async fn complete(&self, code: &str, pending: &OidcPending) -> Result<User, AuthError>;
    /// Can the provider currently be reached (discovery succeeded)?
    async fn probe(&self) -> Result<(), String>;
}

/// GitHub OAuth 2.0 sign-in. GitHub does not publish OpenID Connect discovery
/// or ID tokens for sign-in, so the identity and the groups (organisations and
/// teams) come from the GitHub API after the code exchange.
#[async_trait]
pub trait GithubLogin: Send + Sync {
    /// The GitHub page that starts the sign-in. `state` protects the callback.
    fn authorize_url(&self, state: &str) -> String;
    /// Exchanges the callback code and loads the signed-in user.
    async fn complete(&self, code: &str) -> Result<User, AuthError>;
}
