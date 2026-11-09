//! Scripts: definitions of SQL pages, their parameters, charts and the
//! placeholder rules that tie them together.

pub mod catalog;
pub mod chart;
pub mod error;
pub mod param;
pub mod repository;
pub mod script;
pub mod sql_template;

pub use catalog::ScriptCatalog;
pub use chart::{ChartData, ChartDefinition, ChartKind, Dataset, DatasetData};
pub use error::{FieldError, ScriptError};
pub use param::{ParamDefinition, ParamKind, SelectOption};
pub use repository::ConfigScriptCatalog;
pub use script::{ScriptDefinition, ScriptName, MAX_PAGE_SIZE};
