use thiserror::Error;

use crate::query::QueryError;

/// A validation problem tied to one form field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub param: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("script not found")]
    NotFound,
    #[error("permission denied")]
    Forbidden,
    #[error("invalid parameters")]
    InvalidParams(Vec<FieldError>),
    #[error(transparent)]
    Query(#[from] QueryError),
}

impl ScriptError {
    pub fn outcome(&self) -> &'static str {
        match self {
            ScriptError::NotFound => "not_found",
            ScriptError::Forbidden => "forbidden",
            ScriptError::InvalidParams(_) => "invalid_params",
            ScriptError::Query(QueryError::Timeout) => "timeout",
            ScriptError::Query(_) => "error",
        }
    }
}
