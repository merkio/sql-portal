use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Ldap,
    Oidc,
    /// GitHub OAuth 2.0 sign-in (not OpenID Connect).
    Github,
    /// Authentication is switched off in the configuration.
    Disabled,
}

impl AuthProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthProvider::Ldap => "ldap",
            AuthProvider::Oidc => "oidc",
            AuthProvider::Github => "github",
            AuthProvider::Disabled => "disabled",
        }
    }
}

/// An identity asserted by an authentication provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub subject: String,
    pub display_name: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
    pub provider: AuthProvider,
}
