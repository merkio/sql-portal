use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// Accepts either the native YAML type or a string (as produced by `${ENV}`
/// substitution), e.g. `enabled: ${AUTH_ENABLED:-true}`.
pub fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + FromStr,
    T::Err: Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either<T> {
        Value(T),
        Text(String),
    }
    match Either::<T>::deserialize(deserializer)? {
        Either::Value(v) => Ok(v),
        Either::Text(s) => s.trim().parse().map_err(serde::de::Error::custom),
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Shared TLS settings for every outbound connection. A connection with its
    /// own `tls` block uses that block instead.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub access: AccessConfig,
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "ServerConfig::default_bind")]
    pub bind: String,
    /// Externally visible base URL, used to build the OIDC redirect URI.
    #[serde(default = "ServerConfig::default_public_url")]
    pub public_url: String,
    #[serde(default)]
    pub session: SessionConfig,
}

impl ServerConfig {
    fn default_bind() -> String {
        "0.0.0.0:8080".into()
    }
    fn default_public_url() -> String {
        "http://localhost:8080".into()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: Self::default_bind(),
            public_url: Self::default_public_url(),
            session: SessionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    #[serde(default = "SessionConfig::default_cookie")]
    pub cookie_name: String,
    /// Set the `Secure` cookie attribute (required behind HTTPS).
    #[serde(default, deserialize_with = "lenient")]
    pub secure: bool,
    #[serde(default = "SessionConfig::default_ttl", deserialize_with = "lenient")]
    pub ttl_minutes: i64,
}

impl SessionConfig {
    fn default_cookie() -> String {
        "sp_session".into()
    }
    fn default_ttl() -> i64 {
        60
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            cookie_name: Self::default_cookie(),
            secure: false,
            ttl_minutes: Self::default_ttl(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default = "UiConfig::default_title")]
    pub title: String,
    /// Chart.js bundle. Point this at a self-hosted copy to avoid the CDN.
    #[serde(default = "UiConfig::default_chartjs")]
    pub chartjs_url: String,
}

impl UiConfig {
    fn default_title() -> String {
        "Script Portal".into()
    }
    fn default_chartjs() -> String {
        "https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js".into()
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            title: Self::default_title(),
            chartjs_url: Self::default_chartjs(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// `tracing` env-filter directive, e.g. `info,script_portal=debug`.
    #[serde(default = "ObservabilityConfig::default_level")]
    pub log_level: String,
    #[serde(default)]
    pub log_format: LogFormat,
    #[serde(default = "yes", deserialize_with = "lenient")]
    pub metrics_enabled: bool,
    #[serde(default = "ObservabilityConfig::default_metrics_path")]
    pub metrics_path: String,
}

impl ObservabilityConfig {
    fn default_level() -> String {
        "info".into()
    }
    fn default_metrics_path() -> String {
        "/metrics".into()
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            log_level: Self::default_level(),
            log_format: LogFormat::default(),
            metrics_enabled: true,
            metrics_path: Self::default_metrics_path(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "DatabaseConfig::default_max", deserialize_with = "lenient")]
    pub max_connections: u32,
    #[serde(
        default = "DatabaseConfig::default_acquire",
        deserialize_with = "lenient"
    )]
    pub acquire_timeout_secs: u64,
    /// SQL files executed once at startup (schema / demo data). Paths are
    /// relative to the configuration file.
    #[serde(default)]
    pub init_sql_files: Vec<String>,
    /// Schema and data migrations. Off unless enabled explicitly.
    #[serde(default)]
    pub migrations: Option<MigrationsConfig>,
    /// Reject scripts that are not a single SELECT/WITH statement.
    #[serde(default = "yes", deserialize_with = "lenient")]
    pub read_only_scripts: bool,
    /// TLS for the connection (`postgres://` and `clickhouses://` only).
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

/// TLS settings for the database connection. Use `ca_file` for self-signed
/// certificates; the server's own self-signed certificate works as the file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    /// PEM file with the certificate(s) to trust. Relative paths resolve
    /// against the configuration file.
    #[serde(default)]
    pub ca_file: Option<String>,
    /// Accept any server certificate without verification. Development only.
    #[serde(default, deserialize_with = "lenient")]
    pub insecure: bool,
    /// Client certificate (PEM) for mutual TLS, e.g. `/etc/db-tls/tls.crt`
    /// from a Kubernetes `kubernetes.io/tls` secret. Needs `key_file`.
    #[serde(default)]
    pub cert_file: Option<String>,
    /// Private key (PEM) matching `cert_file`, e.g. `/etc/db-tls/tls.key`.
    #[serde(default)]
    pub key_file: Option<String>,
}

impl DatabaseConfig {
    fn default_max() -> u32 {
        5
    }
    fn default_acquire() -> u64 {
        5
    }
}

/// Migrations run once at startup, in file-name order, before `init_sql_files`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationsConfig {
    /// Explicit opt-in: nothing is applied unless this is `true`.
    #[serde(default, deserialize_with = "lenient")]
    pub enabled: bool,
    /// Directory of `*.sql` files, relative to the config file.
    /// Defaults to `migrations/sqlite`.
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Master switch: `false` disables **all** authorizers (LDAP and OIDC) and
    /// serves every request as an anonymous administrator.
    #[serde(default = "yes", deserialize_with = "lenient")]
    pub enabled: bool,
    #[serde(default)]
    pub ldap: Option<LdapConfig>,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    #[serde(default)]
    pub github: Option<GithubConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            github: None,
            ldap: None,
            oidc: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LdapConfig {
    #[serde(default = "yes", deserialize_with = "lenient")]
    pub enabled: bool,
    /// Overrides the shared `tls` block for this directory.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    /// `ldap://host:389` or `ldaps://host:636`.
    pub url: String,
    #[serde(default, deserialize_with = "lenient")]
    pub start_tls: bool,
    pub bind_dn: String,
    pub bind_password: String,
    pub base_dn: String,
    /// `{username}` is replaced by the escaped login name.
    #[serde(default = "LdapConfig::default_filter")]
    pub user_filter: String,
    #[serde(default = "LdapConfig::default_username_attr")]
    pub username_attribute: String,
    #[serde(default = "LdapConfig::default_display_attr")]
    pub display_name_attribute: String,
    #[serde(default = "LdapConfig::default_email_attr")]
    pub email_attribute: String,
    /// Attribute on the user entry listing group DNs. Empty = none.
    #[serde(default = "LdapConfig::default_group_attr")]
    pub group_attribute: String,
    /// Optional group search for servers without `memberOf`.
    #[serde(default)]
    pub group_search: Option<LdapGroupSearch>,
    #[serde(default = "LdapConfig::default_timeout", deserialize_with = "lenient")]
    pub timeout_secs: u64,
}

impl LdapConfig {
    fn default_filter() -> String {
        "(&(objectClass=person)(uid={username}))".into()
    }
    fn default_username_attr() -> String {
        "uid".into()
    }
    fn default_display_attr() -> String {
        "cn".into()
    }
    fn default_email_attr() -> String {
        "mail".into()
    }
    fn default_group_attr() -> String {
        "memberOf".into()
    }
    fn default_timeout() -> u64 {
        5
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LdapGroupSearch {
    pub base_dn: String,
    /// `{user_dn}` and `{username}` are replaced by escaped values.
    #[serde(default = "LdapGroupSearch::default_filter")]
    pub filter: String,
    #[serde(default = "LdapGroupSearch::default_name_attr")]
    pub name_attribute: String,
}

impl LdapGroupSearch {
    fn default_filter() -> String {
        "(&(objectClass=groupOfNames)(member={user_dn}))".into()
    }
    fn default_name_attr() -> String {
        "cn".into()
    }
}

/// GitHub OAuth sign-in. Create the OAuth app at
/// https://github.com/settings/developers; its callback URL is
/// `<server.public_url>/login/github/callback`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubConfig {
    #[serde(default = "yes", deserialize_with = "lenient")]
    pub enabled: bool,
    pub client_id: String,
    /// Keep it in the environment: `client_secret: ${GITHUB_CLIENT_SECRET}`.
    pub client_secret: String,
    /// When set, only members of at least one of these organisations may sign in.
    #[serde(default)]
    pub allowed_orgs: Vec<String>,
    /// GitHub web address; change only for GitHub Enterprise Server.
    #[serde(default = "GithubConfig::default_web")]
    pub web_url: String,
    /// GitHub API address; change only for GitHub Enterprise Server.
    #[serde(default = "GithubConfig::default_api")]
    pub api_url: String,
}

impl GithubConfig {
    fn default_web() -> String {
        "https://github.com".into()
    }
    fn default_api() -> String {
        "https://api.github.com".into()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    #[serde(default = "yes", deserialize_with = "lenient")]
    pub enabled: bool,
    /// Overrides the shared `tls` block for this provider.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    pub issuer_url: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default = "OidcConfig::default_scopes")]
    pub scopes: Vec<String>,
    #[serde(default = "OidcConfig::default_username_claim")]
    pub username_claim: String,
    #[serde(default = "OidcConfig::default_groups_claim")]
    pub groups_claim: String,
}

impl OidcConfig {
    fn default_scopes() -> Vec<String> {
        vec!["profile".into(), "email".into()]
    }
    fn default_username_claim() -> String {
        "preferred_username".into()
    }
    fn default_groups_claim() -> String {
        "groups".into()
    }
}

/// Role-based access: roles carry permissions; users and groups carry roles.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessConfig {
    /// Roles granted to every authenticated user.
    #[serde(default)]
    pub default_roles: Vec<String>,
    /// role -> permissions (`*` and `prefix.*` wildcards allowed).
    #[serde(default)]
    pub roles: BTreeMap<String, Vec<String>>,
    /// group name or LDAP DN -> roles.
    #[serde(default)]
    pub group_roles: BTreeMap<String, Vec<String>>,
    /// username / OIDC subject -> roles.
    #[serde(default)]
    pub user_roles: BTreeMap<String, Vec<String>>,
    /// External grants service. When set, it is the source of roles and
    /// permissions and the local maps above are not used for authorization.
    #[serde(default)]
    pub external: Option<ExternalGrantsConfig>,
}

/// Permissions loaded from an external service and cached locally.
///
/// The service receives `POST {url}` with JSON
/// `{"subject", "display_name", "email", "groups"}` and answers
/// `{"roles": [...], "permissions": [...]}`, or `404` for an unknown identity
/// (which grants nothing).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalGrantsConfig {
    /// `http://` or `https://` endpoint.
    pub url: String,
    #[serde(
        default = "ExternalGrantsConfig::default_timeout",
        deserialize_with = "lenient"
    )]
    pub timeout_secs: u64,
    /// How long a lookup is served from the cache. Stale entries are used
    /// only while the service is unreachable, for at most twice this long.
    #[serde(
        default = "ExternalGrantsConfig::default_ttl",
        deserialize_with = "lenient"
    )]
    pub cache_ttl_secs: u64,
    /// Sent as `Authorization: Bearer ...` when set.
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Overrides the shared `tls` block for this service.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

impl ExternalGrantsConfig {
    fn default_timeout() -> u64 {
        3
    }
    fn default_ttl() -> u64 {
        300
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptConfig {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Section of the script list, e.g. `group: Sales`. Scripts without a group are listed last.
    #[serde(default)]
    pub group: Option<String>,
    /// Inline SQL. Exactly one of `sql` / `sql_file` is required.
    #[serde(default)]
    pub sql: Option<String>,
    /// SQL file path relative to the configuration file.
    #[serde(default)]
    pub sql_file: Option<String>,
    /// Permissions the caller needs (all of them).
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub params: Vec<ParamConfig>,
    #[serde(default)]
    pub charts: Vec<ChartConfig>,
    #[serde(default = "yes")]
    pub table: bool,
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Rows per page of the table; charts always use `max_rows`.
    #[serde(default)]
    pub page_size: Option<usize>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParamTypeConfig {
    Text,
    Integer,
    Number,
    Boolean,
    Date,
    Select,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(rename = "type")]
    pub kind: ParamTypeConfig,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub max_length: Option<usize>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub options: Vec<OptionConfig>,
}

/// A select option: either a bare scalar or `{value, label}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OptionConfig {
    /// `value` and `label` may be any YAML scalar, e.g. `label: 2024`.
    Full {
        value: serde_yaml_ng::Value,
        label: Option<serde_yaml_ng::Value>,
    },
    Scalar(serde_yaml_ng::Value),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChartTypeConfig {
    Line,
    Bar,
    Pie,
    Doughnut,
    Scatter,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartConfig {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub kind: ChartTypeConfig,
    pub x: String,
    pub y: Vec<String>,
}
