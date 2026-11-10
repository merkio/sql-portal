use std::sync::Arc;

use crate::scripts::ScriptDefinition;

/// Read access to the configured scripts.
pub trait ScriptCatalog: Send + Sync {
    fn all(&self) -> Vec<Arc<ScriptDefinition>>;
    fn get(&self, name: &str) -> Option<Arc<ScriptDefinition>>;
}
