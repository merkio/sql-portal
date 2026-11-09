//! Common kernel: values and errors that every context uses.

pub mod error;
pub mod value;

pub use error::DomainError;
pub use value::Value;
