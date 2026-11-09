//! Query execution: requests and results, pagination, and the errors a run can end with.

pub mod error;
pub mod page;
pub mod query;
pub mod request;

pub use error::QueryError;
pub use page::{PageInfo, PageRequest};
pub use query::QueryExecutor;
pub use request::{QueryRequest, QueryResult};
