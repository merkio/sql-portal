//! Maps the raw configuration model onto validated domain objects.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::loader::{ConfigError, LoadedConfig};
use super::model::*;
use super::runtime::{
    AuthRuntime, Connection, DatabaseRuntime, ExternalGrants, GrantsRuntime, RuntimeConfig,
};
use crate::tls::TlsOptions;
use sql_portal_domain::scripts::sql_template::is_read_only_select;
use sql_portal_domain::{
    AccessPolicy, ChartDefinition, ChartKind, ParamDefinition, ParamKind, Permission,
    ScriptDefinition, ScriptName, SelectOption,
};

fn invalid(msg: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(msg.into())
}

fn ldap_uses_tls(ldap: &LdapConfig) -> bool {
    ldap.url.starts_with("ldaps://") || ldap.start_tls
}

impl LoadedConfig {
    fn check_tls(&self, label: &str, tls: &TlsOptions) -> Result<(), ConfigError> {
        tls.check().map_err(|e| invalid(format!("{label}: {e}")))
    }

    /// Resolves a path relative to the configuration file.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        let p = Path::new(relative);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        }
    }

    /// Checks the configuration and returns the validated runtime value.
    pub fn into_runtime(&self) -> Result<RuntimeConfig, ConfigError> {
        self.check_rules()?;
        let app = &self.app;
        let auth = if app.auth.enabled {
            AuthRuntime::Enabled {
                ldap: app.auth.ldap.as_ref().filter(|l| l.enabled).map(|l| {
                    Box::new(Connection {
                        settings: l.clone(),
                        tls: self.ldap_tls(),
                    })
                }),
                oidc: app.auth.oidc.as_ref().filter(|o| o.enabled).map(|o| {
                    Box::new(Connection {
                        settings: o.clone(),
                        tls: self.oidc_tls(),
                    })
                }),
                github: app
                    .auth
                    .github
                    .as_ref()
                    .filter(|g| g.enabled)
                    .map(|g| Box::new(g.clone())),
            }
        } else {
            AuthRuntime::Disabled
        };
        let grants = match &app.access.external {
            Some(external) => GrantsRuntime::External(Connection {
                settings: ExternalGrants {
                    url: external.url.clone(),
                    timeout: Duration::from_secs(external.timeout_secs),
                    cache_ttl: Duration::from_secs(external.cache_ttl_secs),
                    bearer_token: external.bearer_token.clone(),
                },
                tls: self.grants_tls(),
            }),
            None => GrantsRuntime::Local,
        };
        Ok(RuntimeConfig {
            server: app.server.clone(),
            ui: app.ui.clone(),
            observability: app.observability.clone(),
            database: DatabaseRuntime {
                url: app.database.url.clone(),
                max_connections: app.database.max_connections,
                acquire_timeout: Duration::from_secs(app.database.acquire_timeout_secs),
                tls: self.database_tls(),
                init_files: app
                    .database
                    .init_sql_files
                    .iter()
                    .map(|f| self.resolve(f))
                    .collect(),
                migrations: self.database_migrations(),
            },
            auth,
            grants,
            policy: self.build_policy()?,
            scripts: self.build_scripts()?,
        })
    }

    /// Fails with the first configuration problem found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.into_runtime().map(|_| ())
    }

    /// Cross-cutting rules that do not need the domain objects built.
    fn check_rules(&self) -> Result<(), ConfigError> {
        let app = &self.app;

        if app.auth.enabled
            && !app.auth.ldap.as_ref().is_some_and(|l| l.enabled)
            && !app.auth.oidc.as_ref().is_some_and(|o| o.enabled)
            && !app.auth.github.as_ref().is_some_and(|g| g.enabled)
        {
            return Err(invalid(
                "auth.enabled is true but neither auth.ldap, auth.oidc nor auth.github is enabled; \
                 enable a provider or set auth.enabled to false",
            ));
        }
        // Shared and per-connection TLS. Files and combinations are checked on
        // the resolved options; scheme rules only apply to explicit blocks.
        self.check_tls("tls", &self.resolve_tls(app.tls.as_ref(), true))?;
        if app.database.tls.is_some() {
            let url = app.database.url.as_str();
            let supported = url.starts_with("postgres://")
                || url.starts_with("postgresql://")
                || url.starts_with("clickhouses://");
            if !supported {
                return Err(invalid(
                    "database.tls applies only to postgres:// and clickhouses:// URLs",
                ));
            }
        }
        self.check_tls("database.tls", &self.database_tls())?;
        if let Some(ldap) = &app.auth.ldap {
            if ldap.tls.is_some() && !ldap_uses_tls(ldap) {
                return Err(invalid(
                    "auth.ldap.tls needs an ldaps:// URL or start_tls: true",
                ));
            }
            self.check_tls("auth.ldap.tls", &self.ldap_tls())?;
        }
        if app.auth.oidc.is_some() {
            self.check_tls("auth.oidc.tls", &self.oidc_tls())?;
        }
        if let Some(github) = app.auth.github.as_ref().filter(|g| g.enabled) {
            if github.client_id.trim().is_empty() || github.client_secret.trim().is_empty() {
                return Err(invalid(
                    "auth.github: client_id and client_secret must be set",
                ));
            }
            for (name, url) in [("web_url", &github.web_url), ("api_url", &github.api_url)] {
                url::Url::parse(url)
                    .map_err(|e| invalid(format!("auth.github.{name} is not a valid URL: {e}")))?;
            }
            if github.allowed_orgs.iter().any(|o| o.trim().is_empty()) {
                return Err(invalid(
                    "auth.github.allowed_orgs must not contain empty names",
                ));
            }
        }
        if let Some(external) = &app.access.external {
            let scheme_ok =
                external.url.starts_with("https://") || external.url.starts_with("http://");
            if !scheme_ok {
                return Err(invalid(
                    "access.external.url must start with http:// or https://",
                ));
            }
            if external.tls.is_some() && !external.url.starts_with("https://") {
                return Err(invalid("access.external.tls needs an https:// URL"));
            }
            if external.timeout_secs == 0 || external.cache_ttl_secs == 0 {
                return Err(invalid(
                    "access.external: timeout_secs and cache_ttl_secs must be positive",
                ));
            }
            self.check_tls("access.external.tls", &self.grants_tls())?;
        }
        if let Some(dir) = self.database_migrations() {
            if !dir.is_dir() {
                return Err(invalid(format!(
                    "database.migrations.dir is not a directory: {}",
                    dir.display()
                )));
            }
        }
        if !app.observability.metrics_path.starts_with('/') {
            return Err(invalid("observability.metrics_path must start with '/'"));
        }
        if app.server.session.ttl_minutes <= 0 {
            return Err(invalid("server.session.ttl_minutes must be positive"));
        }
        if app.database.max_connections == 0 {
            return Err(invalid("database.max_connections must be at least 1"));
        }
        url::Url::parse(&app.server.public_url)
            .map_err(|e| invalid(format!("server.public_url is not a valid URL: {e}")))?;
        if !app.ui.chartjs_url.starts_with('/') {
            url::Url::parse(&app.ui.chartjs_url)
                .map_err(|e| invalid(format!("ui.chartjs_url is not a valid URL: {e}")))?;
        }

        Ok(())
    }

    /// TLS for one connection: its own `tls` block, or the shared block when the
    /// connection actually uses TLS. Paths resolve against the configuration file.
    fn resolve_tls(&self, own: Option<&TlsConfig>, uses_tls: bool) -> TlsOptions {
        let shared = if uses_tls {
            self.app.tls.as_ref()
        } else {
            None
        };
        let source = own.or(shared);
        match source {
            None => TlsOptions::default(),
            Some(tls) => TlsOptions {
                ca_file: tls.ca_file.as_deref().map(|f| self.resolve(f)),
                insecure: tls.insecure,
                cert_file: tls.cert_file.as_deref().map(|f| self.resolve(f)),
                key_file: tls.key_file.as_deref().map(|f| self.resolve(f)),
            },
        }
    }

    pub fn database_tls(&self) -> TlsOptions {
        let url = self.app.database.url.as_str();
        let uses_tls = url.starts_with("postgres://")
            || url.starts_with("postgresql://")
            || url.starts_with("clickhouses://");
        self.resolve_tls(self.app.database.tls.as_ref(), uses_tls)
    }

    pub fn ldap_tls(&self) -> TlsOptions {
        let ldap = self.app.auth.ldap.as_ref();
        let uses_tls = ldap.is_some_and(ldap_uses_tls);
        self.resolve_tls(ldap.and_then(|l| l.tls.as_ref()), uses_tls)
    }

    pub fn grants_tls(&self) -> TlsOptions {
        let external = self.app.access.external.as_ref();
        let uses_tls = external.is_some_and(|e| e.url.starts_with("https://"));
        self.resolve_tls(external.and_then(|e| e.tls.as_ref()), uses_tls)
    }

    pub fn oidc_tls(&self) -> TlsOptions {
        let own = self.app.auth.oidc.as_ref().and_then(|o| o.tls.as_ref());
        self.resolve_tls(own, true)
    }

    /// The migrations directory when migrations are enabled, `None` otherwise.
    pub fn database_migrations(&self) -> Option<PathBuf> {
        let migrations = self.app.database.migrations.as_ref()?;
        if !migrations.enabled {
            return None;
        }
        Some(self.resolve(migrations.dir.as_deref().unwrap_or("migrations/sqlite")))
    }

    pub fn build_policy(&self) -> Result<AccessPolicy, ConfigError> {
        let a = &self.app.access;
        AccessPolicy::new(
            a.roles.clone().into_iter().collect(),
            a.group_roles.clone().into_iter().collect(),
            a.user_roles.clone().into_iter().collect(),
            a.default_roles.clone(),
        )
        .map_err(|e| invalid(e.to_string()))
    }

    pub fn build_scripts(&self) -> Result<Vec<ScriptDefinition>, ConfigError> {
        let mut seen = HashSet::new();
        let mut scripts = Vec::with_capacity(self.app.scripts.len());
        for cfg in &self.app.scripts {
            if !seen.insert(cfg.name.clone()) {
                return Err(invalid(format!("duplicate script name '{}'", cfg.name)));
            }
            scripts.push(self.build_script(cfg)?);
        }
        Ok(scripts)
    }

    fn build_script(&self, cfg: &ScriptConfig) -> Result<ScriptDefinition, ConfigError> {
        let sql = match (&cfg.sql, &cfg.sql_file) {
            (Some(sql), None) => sql.clone(),
            (None, Some(file)) => {
                let path = self.resolve(file);
                std::fs::read_to_string(&path).map_err(|source| ConfigError::Io {
                    path: path.display().to_string(),
                    source,
                })?
            }
            _ => {
                return Err(invalid(format!(
                    "script '{}': set exactly one of 'sql' or 'sql_file'",
                    cfg.name
                )))
            }
        };

        if self.app.database.read_only_scripts && !is_read_only_select(&sql) {
            return Err(invalid(format!(
                "script '{}': only a single read-only SELECT/WITH statement is allowed \
                 (set database.read_only_scripts to false to opt out)",
                cfg.name
            )));
        }

        let name = ScriptName::new(&cfg.name).map_err(|e| invalid(e.to_string()))?;
        let mut script = ScriptDefinition::new_unchecked(name, cfg.title.clone(), sql);
        script.description = cfg.description.clone();
        script.group = cfg.group.as_ref().map(|g| g.trim().to_string());
        script.show_table = cfg.table;
        if let Some(n) = cfg.page_size {
            script.page_size = n;
        }
        if let Some(n) = cfg.max_rows {
            script.max_rows = n;
        }
        if let Some(t) = cfg.timeout_secs {
            script.timeout_secs = t;
        }
        script.required_permissions = cfg
            .permissions
            .iter()
            .map(|p| {
                Permission::new(p.clone())
                    .map_err(|e| invalid(format!("script '{}': {e}", cfg.name)))
            })
            .collect::<Result<_, _>>()?;
        script.params = cfg
            .params
            .iter()
            .map(|p| map_param(&cfg.name, p))
            .collect::<Result<_, _>>()?;
        script.charts = cfg
            .charts
            .iter()
            .map(|c| map_chart(&cfg.title, c))
            .collect();

        script.validate().map_err(|e| invalid(e.to_string()))?;
        Ok(script)
    }
}

fn scalar_to_string(v: &serde_yaml_ng::Value) -> Option<String> {
    match v {
        serde_yaml_ng::Value::String(s) => Some(s.clone()),
        serde_yaml_ng::Value::Number(n) => Some(n.to_string()),
        serde_yaml_ng::Value::Bool(b) => Some(b.to_string()),
        serde_yaml_ng::Value::Null => None,
        _ => None,
    }
}

fn map_param(script: &str, cfg: &ParamConfig) -> Result<ParamDefinition, ConfigError> {
    let kind = match cfg.kind {
        ParamTypeConfig::Text => ParamKind::Text,
        ParamTypeConfig::Integer => ParamKind::Integer,
        ParamTypeConfig::Number => ParamKind::Number,
        ParamTypeConfig::Boolean => ParamKind::Boolean,
        ParamTypeConfig::Date => ParamKind::Date,
        ParamTypeConfig::Select => ParamKind::Select,
    };
    let mut p = ParamDefinition::new(cfg.name.clone(), kind);
    if let Some(label) = &cfg.label {
        p.label = label.clone();
    }
    p.required = cfg.required;
    p.help = cfg.help.clone();
    p.min = cfg.min;
    p.max = cfg.max;
    if let Some(n) = cfg.max_length {
        p.max_length = n;
    }
    p.pattern = cfg.pattern.clone();
    p.default = match &cfg.default {
        None => None,
        Some(v) => Some(scalar_to_string(v).ok_or_else(|| {
            invalid(format!(
                "script '{script}': default of parameter '{}' must be a scalar",
                cfg.name
            ))
        })?),
    };
    for option in &cfg.options {
        p.options.push(match option {
            OptionConfig::Full { value, label } => {
                let value = scalar_to_string(value).unwrap_or_default();
                SelectOption {
                    label: label
                        .as_ref()
                        .and_then(scalar_to_string)
                        .unwrap_or_else(|| value.clone()),
                    value,
                }
            }
            OptionConfig::Scalar(v) => {
                let value = scalar_to_string(v).unwrap_or_default();
                SelectOption {
                    label: value.clone(),
                    value,
                }
            }
        });
    }
    Ok(p)
}

fn map_chart(script_title: &str, cfg: &ChartConfig) -> ChartDefinition {
    ChartDefinition {
        title: cfg
            .title
            .clone()
            .unwrap_or_else(|| script_title.to_string()),
        kind: match cfg.kind {
            ChartTypeConfig::Line => ChartKind::Line,
            ChartTypeConfig::Bar => ChartKind::Bar,
            ChartTypeConfig::Pie => ChartKind::Pie,
            ChartTypeConfig::Doughnut => ChartKind::Doughnut,
            ChartTypeConfig::Scatter => ChartKind::Scatter,
        },
        x: cfg.x.clone(),
        y: cfg.y.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::loader::load_from_str_with_env;

    fn load(yaml: &str) -> LoadedConfig {
        load_from_str_with_env(yaml, std::env::temp_dir(), &|_| None).unwrap()
    }

    const FULL: &str = r#"
database:
  url: "sqlite::memory:"
auth:
  enabled: false
access:
  default_roles: [viewer]
  roles:
    viewer: ["scripts.public.read"]
    admin: ["*"]
  group_roles:
    "cn=admins,ou=groups": [admin]
  user_roles:
    alice: [admin]
scripts:
  - name: sales
    title: Sales
    description: Sales per region
    permissions: [scripts.sales.read]
    max_rows: 50
    timeout_secs: 5
    sql: "SELECT region, SUM(amount) AS total FROM sales WHERE sold_on >= :from_date AND region = :region GROUP BY region"
    params:
      - name: from_date
        label: From
        type: date
        required: true
        default: "2024-01-01"
      - name: region
        type: select
        default: ""
        options: ["", EU, {value: US, label: "United States"}]
    charts:
      - { type: bar, x: region, y: [total] }
"#;

    #[test]
    fn maps_full_config_to_domain() {
        let cfg = load(FULL);
        cfg.validate().unwrap();
        let scripts = cfg.build_scripts().unwrap();
        let s = &scripts[0];
        assert_eq!(s.name.as_str(), "sales");
        assert_eq!(s.max_rows, 50);
        assert_eq!(s.timeout_secs, 5);
        assert_eq!(s.required_permissions[0].as_str(), "scripts.sales.read");
        assert_eq!(s.params[0].label, "From");
        assert_eq!(s.params[1].options.len(), 3);
        assert_eq!(s.params[1].options[2].label, "United States");
        assert_eq!(s.charts[0].title, "Sales");
        assert!(s.runs_without_input());
    }

    #[test]
    fn builds_access_policy() {
        let policy = load(FULL).build_policy().unwrap();
        let p = policy.resolve(sql_portal_domain::User {
            subject: "alice".into(),
            display_name: "a".into(),
            email: None,
            groups: vec![],
            provider: sql_portal_domain::AuthProvider::Ldap,
        });
        assert!(p.roles.contains("admin"));
    }

    #[test]
    fn enabled_auth_requires_a_provider() {
        let cfg = load("database:\n  url: x\n");
        assert!(cfg.validate().unwrap_err().to_string().contains("neither"));
        // Disabled providers do not count.
        let cfg = load("database:\n  url: x\nauth:\n  ldap:\n    enabled: false\n    url: l\n    bind_dn: b\n    bind_password: p\n    base_dn: d\n");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn disabling_auth_needs_no_provider() {
        load("database:\n  url: x\nauth:\n  enabled: false\n")
            .validate()
            .unwrap();
    }

    #[test]
    fn rejects_scripts_with_both_or_no_sql_source() {
        let cfg = load(
            "database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n",
        );
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("exactly one"));
    }

    #[test]
    fn rejects_write_statements_when_read_only() {
        let cfg = load("database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n    sql: DELETE FROM t\n");
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("read-only"));
        let cfg = load("database:\n  url: x\n  read_only_scripts: false\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n    table: true\n    sql: DELETE FROM t\n");
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_duplicate_script_names() {
        let yaml = "database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - {name: a, title: A, sql: SELECT 1}\n  - {name: a, title: B, sql: SELECT 2}\n";
        assert!(load(yaml)
            .validate()
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }

    #[test]
    fn reports_undeclared_placeholders() {
        let yaml = "database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - {name: a, title: A, sql: 'SELECT :x'}\n";
        assert!(load(yaml)
            .validate()
            .unwrap_err()
            .to_string()
            .contains(":x"));
    }

    #[test]
    fn reads_sql_from_file_relative_to_config() {
        let dir = std::env::temp_dir().join(format!("sp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("q.sql"), "SELECT 42 AS answer").unwrap();
        let yaml = "database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - {name: a, title: A, sql_file: q.sql}\n";
        let cfg = load_from_str_with_env(yaml, dir.clone(), &|_| None).unwrap();
        assert_eq!(cfg.build_scripts().unwrap()[0].sql, "SELECT 42 AS answer");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_sql_file_is_an_error() {
        let yaml = "database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - {name: a, title: A, sql_file: nope.sql}\n";
        assert!(matches!(
            load(yaml).build_scripts(),
            Err(ConfigError::Io { .. })
        ));
    }

    #[test]
    fn rejects_bad_metrics_path_and_urls() {
        let base = "database:\n  url: x\nauth:\n  enabled: false\n";
        assert!(
            load(&format!("{base}observability:\n  metrics_path: metrics\n"))
                .validate()
                .is_err()
        );
        assert!(load(&format!("{base}server:\n  public_url: nope\n"))
            .validate()
            .is_err());
    }

    #[test]
    fn rejects_invalid_param_default() {
        let yaml = "database:\n  url: x\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n    sql: 'SELECT :n'\n    params:\n      - {name: n, type: integer, default: abc}\n";
        assert!(load(yaml)
            .validate()
            .unwrap_err()
            .to_string()
            .contains("default"));
    }
}

#[cfg(test)]
mod tls_tests {
    use super::*;
    use crate::config::loader::load_from_str_with_env;

    fn load(yaml: &str) -> LoadedConfig {
        load_from_str_with_env(yaml, std::env::temp_dir(), &|_| None).unwrap()
    }

    #[test]
    fn tls_is_rejected_for_sqlite() {
        let cfg = load(
            "database:\n  url: \"sqlite::memory:\"\n  tls:\n    insecure: true\nauth:\n  enabled: false\n",
        );
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("postgres://"));
    }

    #[test]
    fn missing_ca_file_is_reported() {
        let cfg = load(
            "database:\n  url: postgres://db/app\n  tls:\n    ca_file: no-such-ca.pem\nauth:\n  enabled: false\n",
        );
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("not found"));
    }

    #[test]
    fn ca_file_and_insecure_conflict() {
        let cfg = load(
            "database:\n  url: postgres://db/app\n  tls:\n    ca_file: x.pem\n    insecure: true\nauth:\n  enabled: false\n",
        );
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("cannot be combined"));
    }

    #[test]
    fn client_certificate_needs_its_key_and_a_server_check() {
        let only_cert = load(
            "database:\n  url: postgres://db/app\n  tls:\n    ca_file: \"\"\n    cert_file: tls.crt\nauth:\n  enabled: false\n",
        );
        assert!(only_cert
            .validate()
            .unwrap_err()
            .to_string()
            .contains("set together"));

        let no_server_check = load(
            "database:\n  url: postgres://db/app\n  tls:\n    cert_file: tls.crt\n    key_file: tls.key\nauth:\n  enabled: false\n",
        );
        assert!(no_server_check
            .validate()
            .unwrap_err()
            .to_string()
            .contains("needs ca_file"));
    }

    #[test]
    fn ca_file_resolves_against_the_config_directory() {
        let dir = std::env::temp_dir();
        let file = dir.join("portal-tls-test-ca.pem");
        std::fs::write(&file, "dummy").unwrap();
        let cfg = load(
            "database:\n  url: clickhouses://ch.internal/analytics\n  tls:\n    ca_file: portal-tls-test-ca.pem\nauth:\n  enabled: false\n",
        );
        cfg.validate().unwrap();
        assert_eq!(cfg.database_tls().ca_file, Some(file.clone()));
        assert!(!cfg.database_tls().insecure);
        std::fs::remove_file(file).ok();
    }
}

#[cfg(test)]
mod shared_tls_tests {
    use super::*;
    use crate::config::loader::load_from_str_with_env;

    fn load(yaml: &str) -> LoadedConfig {
        load_from_str_with_env(yaml, std::env::temp_dir(), &|_| None).unwrap()
    }

    /// Creates a CA file in the config directory. Each test uses its own name:
    /// tests run in parallel.
    fn with_ca<T>(name: &str, f: impl FnOnce(&str, PathBuf) -> T) -> T {
        let file = format!("portal-shared-ca-{name}.pem");
        let path = std::env::temp_dir().join(&file);
        std::fs::write(&path, "dummy").unwrap();
        let out = f(&file, path.clone());
        std::fs::remove_file(path).ok();
        out
    }

    #[test]
    fn shared_block_applies_to_every_tls_capable_connection() {
        with_ca("applies", |ca, path| {
            let cfg = load(&format!(
                "tls:\n  ca_file: {ca}\ndatabase:\n  url: postgres://db/app\nauth:\n  enabled: true\n  ldap:\n    url: ldaps://ldap.internal\n    bind_dn: cn=svc\n    bind_password: p\n    base_dn: dc=x\n  oidc:\n    issuer_url: https://idp.internal\n    client_id: portal\n"
            ));
            cfg.validate().unwrap();
            assert_eq!(cfg.database_tls().ca_file, Some(path.clone()));
            assert_eq!(cfg.ldap_tls().ca_file, Some(path.clone()));
            assert_eq!(cfg.oidc_tls().ca_file, Some(path));
        });
    }

    #[test]
    fn shared_block_is_ignored_by_plain_connections() {
        with_ca("plain", |ca, _| {
            let cfg = load(&format!(
                "tls:\n  ca_file: {ca}\ndatabase:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: true\n  ldap:\n    url: ldap://ldap.internal\n    bind_dn: cn=svc\n    bind_password: p\n    base_dn: dc=x\n  oidc:\n    issuer_url: https://idp.internal\n    client_id: portal\n"
            ));
            cfg.validate().unwrap();
            assert!(!cfg.database_tls().is_set());
            assert!(!cfg.ldap_tls().is_set());
            assert!(cfg.oidc_tls().is_set());
        });
    }

    #[test]
    fn per_connection_block_replaces_the_shared_one() {
        with_ca("override", |ca, path| {
            let cfg = load(&format!(
                "tls:\n  insecure: true\ndatabase:\n  url: postgres://db/app\n  tls:\n    ca_file: {ca}\nauth:\n  enabled: false\n"
            ));
            cfg.validate().unwrap();
            let db = cfg.database_tls();
            assert_eq!(db.ca_file, Some(path));
            assert!(!db.insecure, "database block must not inherit insecure");
        });
    }

    #[test]
    fn explicit_ldap_tls_on_plain_ldap_is_rejected() {
        with_ca("plain-ldap", |ca, _| {
            let cfg = load(&format!(
                "database:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: true\n  ldap:\n    url: ldap://ldap.internal\n    tls:\n      ca_file: {ca}\n    bind_dn: cn=svc\n    bind_password: p\n    base_dn: dc=x\n"
            ));
            assert!(cfg.validate().unwrap_err().to_string().contains("ldaps://"));
        });
    }
}

#[cfg(test)]
mod external_access_tests {
    use super::*;
    use crate::config::loader::load_from_str_with_env;

    fn load(access: &str) -> LoadedConfig {
        let yaml = format!(
            "database:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: false\naccess:\n  external:\n{access}"
        );
        load_from_str_with_env(&yaml, std::env::temp_dir(), &|_| None).unwrap()
    }

    #[test]
    fn external_service_is_accepted_with_defaults() {
        let cfg = load("    url: https://perms.internal/permissions\n");
        cfg.validate().unwrap();
        let ext = cfg.app.access.external.as_ref().unwrap();
        assert_eq!((ext.timeout_secs, ext.cache_ttl_secs), (3, 300));
    }

    #[test]
    fn url_must_be_http_or_https() {
        let cfg = load("    url: ftp://perms.internal/permissions\n");
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("http:// or https://"));
    }

    #[test]
    fn explicit_tls_needs_https() {
        let cfg =
            load("    url: http://perms.internal/permissions\n    tls:\n      insecure: true\n");
        assert!(cfg.validate().unwrap_err().to_string().contains("https://"));
    }

    #[test]
    fn shared_tls_applies_only_to_https_permission_services() {
        let yaml = "tls:\n  insecure: true\ndatabase:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: false\naccess:\n  external:\n    url: http://perms.internal/permissions\n";
        let cfg = load_from_str_with_env(yaml, std::env::temp_dir(), &|_| None).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.grants_tls().is_set());
    }

    #[test]
    fn zero_timeouts_are_rejected() {
        let cfg = load("    url: https://perms.internal/permissions\n    cache_ttl_secs: 0\n");
        assert!(cfg.validate().is_err());
    }
}

#[cfg(test)]
mod page_size_tests {
    use super::*;
    use crate::config::loader::load_from_str_with_env;

    fn build(page_size: &str) -> Result<Vec<ScriptDefinition>, ConfigError> {
        let yaml = format!(
            "database:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n    sql: SELECT 1\n    page_size: {page_size}\n"
        );
        load_from_str_with_env(&yaml, std::env::temp_dir(), &|_| None)
            .unwrap()
            .build_scripts()
    }

    #[test]
    fn page_size_is_read_and_bounded() {
        assert_eq!(build("40").unwrap()[0].page_size, 40);
        assert!(build("0").is_err());
        assert!(build("501").is_err());
    }
}

#[cfg(test)]
mod select_option_tests {
    use crate::config::loader::load_from_str_with_env;

    #[test]
    fn numeric_values_and_labels_are_read_as_text() {
        let yaml = "database:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n    sql: SELECT 1\n    params:\n      - name: year\n        type: select\n        options:\n          - { value: \"2024\", label: 2024 }\n          - { value: 2023 }\n          - ALL\n";
        let cfg = load_from_str_with_env(yaml, std::env::temp_dir(), &|_| None).unwrap();
        let options = &cfg.build_scripts().unwrap()[0].params[0].options;
        let pairs: Vec<(&str, &str)> = options
            .iter()
            .map(|o| (o.value.as_str(), o.label.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![("2024", "2024"), ("2023", "2023"), ("ALL", "ALL")]
        );
    }
}

#[cfg(test)]
mod group_tests {
    use crate::config::loader::{load_from_str_with_env, ConfigError};

    fn build(group: &str) -> Result<(), ConfigError> {
        let yaml = format!(
            "database:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: false\nscripts:\n  - name: a\n    title: A\n    sql: SELECT 1\n    group: \"{group}\"\n"
        );
        load_from_str_with_env(&yaml, std::env::temp_dir(), &|_| None)
            .unwrap()
            .build_scripts()
            .map(|_| ())
    }

    #[test]
    fn group_is_trimmed_and_must_not_be_empty() {
        assert!(build("  Sales  ").is_ok());
        assert!(build("   ").is_err());
    }
}
