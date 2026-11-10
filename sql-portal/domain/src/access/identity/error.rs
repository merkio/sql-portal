use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("authentication is disabled")]
    Disabled,
    #[error("authentication provider is not enabled")]
    ProviderNotEnabled,
    #[error("authentication request rejected: {0}")]
    Rejected(String),
    #[error("authentication provider unavailable: {0}")]
    ProviderUnavailable(String),
}

impl AuthError {
    /// Short, low-cardinality label used for metrics.
    pub fn outcome(&self) -> &'static str {
        match self {
            AuthError::InvalidCredentials => "invalid_credentials",
            AuthError::Disabled | AuthError::ProviderNotEnabled => "not_enabled",
            AuthError::Rejected(_) => "rejected",
            AuthError::ProviderUnavailable(_) => "unavailable",
        }
    }
}
