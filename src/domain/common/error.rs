use thiserror::Error;

/// Violations of domain invariants (invalid configuration, malformed names...).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("{0}")]
    Invalid(String),
}

impl DomainError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        DomainError::Invalid(msg.into())
    }
}
