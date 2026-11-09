use std::collections::BTreeMap;
use std::sync::Arc;

use crate::domain::scripts::ScriptCatalog;
use crate::domain::ScriptDefinition;

/// Catalog holding the scripts loaded from the configuration.
pub struct ConfigScriptCatalog {
    scripts: BTreeMap<String, Arc<ScriptDefinition>>,
}

impl ConfigScriptCatalog {
    pub fn new(scripts: Vec<ScriptDefinition>) -> Self {
        Self {
            scripts: scripts
                .into_iter()
                .map(|s| (s.name.to_string(), Arc::new(s)))
                .collect(),
        }
    }
}

impl ScriptCatalog for ConfigScriptCatalog {
    fn all(&self) -> Vec<Arc<ScriptDefinition>> {
        self.scripts.values().cloned().collect()
    }

    fn get(&self, name: &str) -> Option<Arc<ScriptDefinition>> {
        self.scripts.get(name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ScriptName;

    #[test]
    fn stores_and_finds_scripts() {
        let s = ScriptDefinition::new(ScriptName::new("a").unwrap(), "A", "SELECT 1").unwrap();
        let c = ConfigScriptCatalog::new(vec![s]);
        assert_eq!(c.all().len(), 1);
        assert!(c.get("a").is_some());
        assert!(c.get("b").is_none());
    }
}
