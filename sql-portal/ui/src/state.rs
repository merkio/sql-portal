use std::sync::Arc;

use sql_portal_application::HealthService;
use sql_portal_application::{AuthService, ScriptService};

#[derive(Debug, Clone)]
pub struct UiSettings {
    pub title: String,
    pub chartjs_url: String,
}

/// Dependencies shared by all handlers (cheap to clone).
#[derive(Clone)]
pub struct AppState {
    pub auth: Arc<AuthService>,
    pub scripts: Arc<ScriptService>,
    pub health: Arc<HealthService>,
    /// Renders the Prometheus exposition text.
    pub metrics: Arc<dyn Fn() -> String + Send + Sync>,
    pub ui: Arc<UiSettings>,
}

/// Router-level settings that are not part of the shared state.
#[derive(Debug, Clone)]
pub struct RouterSettings {
    pub metrics_enabled: bool,
    pub metrics_path: String,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub session_ttl_minutes: i64,
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self {
            metrics_enabled: true,
            metrics_path: "/metrics".into(),
            cookie_name: "sp_session".into(),
            cookie_secure: false,
            session_ttl_minutes: 60,
        }
    }
}
