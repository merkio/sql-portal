//! `server:` and `ui:` sections → the HTTP router (`sql-portal-api`).

use std::sync::Arc;

use sql_portal_adapters::config::RuntimeConfig;
use sql_portal_api::{build_router, AppState, Router, RouterSettings, UiSettings};

use crate::{MetricsRender, Services};

pub fn ui_settings(rt: &RuntimeConfig) -> Arc<UiSettings> {
    Arc::new(UiSettings {
        title: rt.ui.title.clone(),
        chartjs_url: rt.ui.chartjs_url.clone(),
    })
}

pub fn router_settings(rt: &RuntimeConfig) -> RouterSettings {
    RouterSettings {
        metrics_enabled: rt.observability.metrics_enabled,
        metrics_path: rt.observability.metrics_path.clone(),
        cookie_name: rt.server.session.cookie_name.clone(),
        cookie_secure: rt.server.session.secure,
        session_ttl_minutes: rt.server.session.ttl_minutes,
    }
}

/// Builds the HTTP router from wired services.
pub fn router(services: Services, metrics: MetricsRender) -> Router {
    build_router(
        AppState {
            auth: services.auth,
            scripts: services.scripts,
            health: services.health,
            metrics,
            ui: services.ui,
        },
        &services.settings,
    )
}
