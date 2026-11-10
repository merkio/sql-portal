//! Query execution: requests and results, pagination, and the errors a run can end with.

pub mod error;
pub mod executor;
pub mod page;
pub mod request;

pub use error::QueryError;
pub use executor::QueryExecutor;
pub use page::{PageInfo, PageRequest};
pub use request::{QueryRequest, QueryResult};
