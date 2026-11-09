use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum QueryError {
    #[error("query timed out")]
    Timeout,
    #[error("database error: {0}")]
    Database(String),
    #[error("database unavailable: {0}")]
    Unavailable(String),
}
