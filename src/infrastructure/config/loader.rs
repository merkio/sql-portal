use std::path::{Path, PathBuf};

use regex::Regex;
use serde_yaml_ng::Value;
use thiserror::Error;

use super::model::AppConfig;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid YAML: {0}")]
    Yaml(String),
    #[error("environment variable '{0}' is not set and has no default")]
    MissingEnv(String),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// A parsed configuration plus the directory used to resolve relative paths.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub app: AppConfig,
    pub base_dir: PathBuf,
}

pub fn load_from_path(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let base_dir = path
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    load_from_str_with_env(&text, base_dir, &|k| std::env::var(k).ok())
}

pub fn load_from_str(yaml: &str, base_dir: &Path) -> Result<LoadedConfig, ConfigError> {
    load_from_str_with_env(yaml, base_dir.to_path_buf(), &|k| std::env::var(k).ok())
}

pub fn load_from_str_with_env(
    yaml: &str,
    base_dir: PathBuf,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<LoadedConfig, ConfigError> {
    let mut value: Value =
        serde_yaml_ng::from_str(yaml).map_err(|e| ConfigError::Yaml(e.to_string()))?;
    expand_value(&mut value, env)?;
    let app: AppConfig =
        serde_yaml_ng::from_value(value).map_err(|e| ConfigError::Yaml(e.to_string()))?;
    Ok(LoadedConfig { app, base_dir })
}

/// Expands `${VAR}` and `${VAR:-default}` in every string scalar, except
/// values of `sql` keys (SQL text is never touched).
fn expand_value(
    value: &mut Value,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(), ConfigError> {
    match value {
        Value::String(s) => *s = expand_env(s, env)?,
        Value::Sequence(items) => {
            for item in items {
                expand_value(item, env)?;
            }
        }
        Value::Mapping(map) => {
            for (key, val) in map.iter_mut() {
                if key.as_str() == Some("sql") {
                    continue;
                }
                expand_value(val, env)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn expand_env(
    input: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<String, ConfigError> {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}").expect("static regex");
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    for caps in re.captures_iter(input) {
        let whole = caps.get(0).expect("match");
        out.push_str(&input[last..whole.start()]);
        let name = &caps[1];
        match env(name).filter(|v| !v.is_empty()) {
            Some(v) => out.push_str(&v),
            None => match caps.get(2) {
                Some(default) => out.push_str(default.as_str()),
                None => return Err(ConfigError::MissingEnv(name.to_string())),
            },
        }
        last = whole.end();
    }
    out.push_str(&input[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<String> {
        match k {
            "A" => Some("alpha".into()),
            "EMPTY" => Some(String::new()),
            "FLAG" => Some("false".into()),
            "N" => Some("7".into()),
            _ => None,
        }
    }

    #[test]
    fn expands_variables_and_defaults() {
        assert_eq!(expand_env("x-${A}-y", &env).unwrap(), "x-alpha-y");
        assert_eq!(
            expand_env("${MISSING:-fallback}", &env).unwrap(),
            "fallback"
        );
        assert_eq!(expand_env("${EMPTY:-d}", &env).unwrap(), "d");
        assert_eq!(expand_env("no vars", &env).unwrap(), "no vars");
        assert_eq!(expand_env("${A}${A}", &env).unwrap(), "alphaalpha");
    }

    #[test]
    fn missing_variable_without_default_is_an_error() {
        assert!(matches!(
            expand_env("${NOPE}", &env),
            Err(ConfigError::MissingEnv(n)) if n == "NOPE"
        ));
    }

    const MINIMAL: &str = "database:\n  url: \"sqlite::memory:\"\n";

    #[test]
    fn minimal_config_uses_defaults() {
        let c = load_from_str_with_env(MINIMAL, ".".into(), &env)
            .unwrap()
            .app;
        assert!(c.auth.enabled);
        assert_eq!(c.server.bind, "0.0.0.0:8080");
        assert_eq!(c.observability.metrics_path, "/metrics");
        assert!(c.database.read_only_scripts);
        assert!(c.scripts.is_empty());
    }

    #[test]
    fn env_substitution_feeds_typed_fields() {
        let yaml = "database:\n  url: ${DB:-sqlite::memory:}\n  max_connections: ${N}\nauth:\n  enabled: ${FLAG}\n";
        let c = load_from_str_with_env(yaml, ".".into(), &env).unwrap().app;
        assert_eq!(c.database.url, "sqlite::memory:");
        assert_eq!(c.database.max_connections, 7);
        assert!(!c.auth.enabled);
    }

    #[test]
    fn sql_values_are_not_expanded() {
        let yaml = "database:\n  url: \"sqlite::memory:\"\nscripts:\n  - name: a\n    title: A\n    sql: \"SELECT '${NOT_EXPANDED}'\"\n";
        let c = load_from_str_with_env(yaml, ".".into(), &env).unwrap().app;
        assert_eq!(
            c.scripts[0].sql.as_deref(),
            Some("SELECT '${NOT_EXPANDED}'")
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let yaml = "database:\n  url: x\nbogus: 1\n";
        assert!(matches!(
            load_from_str_with_env(yaml, ".".into(), &env),
            Err(ConfigError::Yaml(_))
        ));
    }

    #[test]
    fn invalid_yaml_is_reported() {
        assert!(matches!(
            load_from_str_with_env("database: [", ".".into(), &env),
            Err(ConfigError::Yaml(_))
        ));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        assert!(matches!(
            load_from_path(Path::new("/definitely/not/here.yaml")),
            Err(ConfigError::Io { .. })
        ));
    }
}
