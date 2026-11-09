use std::collections::{HashMap, HashSet};
use std::fmt;

use super::chart::ChartDefinition;
use super::error::FieldError;
use super::param::ParamDefinition;
use super::sql_template::scan_placeholders;
use crate::domain::access::Permission;
use crate::domain::common::{DomainError, Value};

pub const DEFAULT_MAX_ROWS: usize = 1000;
/// Rows per table page when a script does not say otherwise.
pub const DEFAULT_PAGE_SIZE: usize = 25;
/// Largest page a script may ask for.
pub const MAX_PAGE_SIZE: usize = 500;
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// URL-safe identifier of a script (`[a-z0-9_-]+`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptName(String);

impl ScriptName {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let ok = !value.is_empty()
            && value.len() <= 64
            && value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if ok {
            Ok(Self(value))
        } else {
            Err(DomainError::invalid(format!(
                "script name '{value}' must be 1-64 chars of [a-z0-9_-]"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScriptName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An aggregate describing one launchable SQL script: its SQL, the form
/// parameters, the permissions required to run it and how to present results.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptDefinition {
    pub name: ScriptName,
    pub title: String,
    pub description: String,
    /// Section of the script list this script belongs to; `None` = ungrouped.
    pub group: Option<String>,
    pub sql: String,
    /// The caller needs **all** of these permissions. Empty = any authenticated user.
    pub required_permissions: Vec<Permission>,
    pub params: Vec<ParamDefinition>,
    pub charts: Vec<ChartDefinition>,
    pub show_table: bool,
    pub max_rows: usize,
    /// Rows per page of the table (1..=MAX_PAGE_SIZE).
    pub page_size: usize,
    pub timeout_secs: u64,
}

impl ScriptDefinition {
    /// Creates a script and verifies its invariants.
    pub fn new(
        name: ScriptName,
        title: impl Into<String>,
        sql: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let script = Self::new_unchecked(name, title, sql);
        script.validate()?;
        Ok(script)
    }

    /// Creates a script with defaults without validating it. Callers must
    /// call [`ScriptDefinition::validate`] once they finished configuring it.
    pub fn new_unchecked(
        name: ScriptName,
        title: impl Into<String>,
        sql: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            group: None,
            sql: sql.into(),
            required_permissions: vec![],
            params: vec![],
            charts: vec![],
            show_table: true,
            max_rows: DEFAULT_MAX_ROWS,
            page_size: DEFAULT_PAGE_SIZE,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            name,
        }
    }

    /// Re-checks invariants; call after mutating public fields.
    pub fn validate(&self) -> Result<(), DomainError> {
        let ctx = |msg: String| DomainError::invalid(format!("script '{}': {msg}", self.name));

        if self.sql.trim().is_empty() {
            return Err(ctx("sql must not be empty".into()));
        }
        if self.title.trim().is_empty() {
            return Err(ctx("title must not be empty".into()));
        }
        if let Some(group) = &self.group {
            if group.trim().is_empty() || group.chars().count() > 64 {
                return Err(ctx("group must be 1 to 64 characters".into()));
            }
        }
        if self.max_rows == 0 {
            return Err(ctx("max_rows must be greater than zero".into()));
        }
        if self.timeout_secs == 0 {
            return Err(ctx("timeout_secs must be greater than zero".into()));
        }
        if !(1..=MAX_PAGE_SIZE).contains(&self.page_size) {
            return Err(ctx(format!(
                "page_size must be between 1 and {MAX_PAGE_SIZE}"
            )));
        }
        if !self.show_table && self.charts.is_empty() {
            return Err(ctx(
                "a script must show a table or at least one chart".into()
            ));
        }

        let mut seen = HashSet::new();
        for p in &self.params {
            p.validate().map_err(|e| ctx(e.to_string()))?;
            if !seen.insert(p.name.as_str()) {
                return Err(ctx(format!("duplicate parameter '{}'", p.name)));
            }
        }
        for placeholder in scan_placeholders(&self.sql) {
            if !seen.contains(placeholder.name.as_str()) {
                return Err(ctx(format!(
                    "sql uses ':{}' but no parameter with that name is defined",
                    placeholder.name
                )));
            }
        }
        Ok(())
    }

    /// Validates the submitted values and returns typed, named parameters.
    pub fn bind_params(
        &self,
        raw: &HashMap<String, String>,
    ) -> Result<Vec<(String, Value)>, Vec<FieldError>> {
        let mut bound = Vec::with_capacity(self.params.len());
        let mut errors = Vec::new();
        for p in &self.params {
            match p.parse(raw.get(&p.name).map(String::as_str)) {
                Ok(v) => bound.push((p.name.clone(), v)),
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            Ok(bound)
        } else {
            Err(errors)
        }
    }

    /// `true` if the script can be executed immediately on page load.
    pub fn runs_without_input(&self) -> bool {
        self.params
            .iter()
            .all(ParamDefinition::is_satisfied_without_input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::scripts::ParamKind;

    fn name(s: &str) -> ScriptName {
        ScriptName::new(s).unwrap()
    }

    #[test]
    fn script_name_rules() {
        assert!(ScriptName::new("sales_by-region2").is_ok());
        assert!(ScriptName::new("").is_err());
        assert!(ScriptName::new("Upper").is_err());
        assert!(ScriptName::new("has space").is_err());
        assert!(ScriptName::new("a/b").is_err());
        assert!(ScriptName::new("x".repeat(65)).is_err());
    }

    #[test]
    fn rejects_empty_sql_title_and_limits() {
        assert!(ScriptDefinition::new(name("a"), "T", "  ").is_err());
        assert!(ScriptDefinition::new(name("a"), " ", "SELECT 1").is_err());
        let mut s = ScriptDefinition::new(name("a"), "T", "SELECT 1").unwrap();
        s.max_rows = 0;
        assert!(s.validate().is_err());
        s.max_rows = 1;
        s.timeout_secs = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn requires_table_or_chart() {
        let mut s = ScriptDefinition::new(name("a"), "T", "SELECT 1").unwrap();
        s.show_table = false;
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_undeclared_placeholders() {
        let err = ScriptDefinition::new(name("a"), "T", "SELECT :x").unwrap_err();
        assert!(err.to_string().contains(":x"));
    }

    #[test]
    fn rejects_duplicate_params() {
        let mut s = ScriptDefinition::new(name("a"), "T", "SELECT 1").unwrap();
        s.params = vec![
            ParamDefinition::new("p", ParamKind::Text),
            ParamDefinition::new("p", ParamKind::Text),
        ];
        assert!(s.validate().unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn binds_params_and_collects_all_errors() {
        let mut s = ScriptDefinition::new_unchecked(name("a"), "T", "SELECT :n, :t");
        let mut n = ParamDefinition::new("n", ParamKind::Integer);
        n.required = true;
        let mut t = ParamDefinition::new("t", ParamKind::Text);
        t.required = true;
        s.params = vec![n, t];
        s.validate().unwrap();

        let ok = HashMap::from([("n".to_string(), "4".to_string()), ("t".into(), "x".into())]);
        assert_eq!(
            s.bind_params(&ok).unwrap(),
            vec![
                ("n".into(), Value::Int(4)),
                ("t".into(), Value::Text("x".into()))
            ]
        );

        let bad = HashMap::from([("n".to_string(), "abc".to_string())]);
        let errs = s.bind_params(&bad).unwrap_err();
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn runs_without_input_depends_on_required_params() {
        let mut s = ScriptDefinition::new_unchecked(name("a"), "T", "SELECT :n");
        let mut n = ParamDefinition::new("n", ParamKind::Integer);
        n.required = true;
        s.params = vec![n];
        assert!(!s.runs_without_input());
        s.params[0].default = Some("1".into());
        assert!(s.runs_without_input());
    }
}
