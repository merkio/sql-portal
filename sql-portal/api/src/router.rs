use axum::extract::Request;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{get, post};
use axum::Router;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tower_sessions::cookie::time::Duration;
use tower_sessions::cookie::SameSite;
use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};
use tracing::info_span;

use super::handlers;
use super::middleware::{content_security_policy, security_headers, track_metrics};
use super::state::{AppState, RouterSettings};

/// Assembles all routes and cross-cutting layers.
pub fn build_router(state: AppState, settings: &RouterSettings) -> Router {
    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_name(settings.cookie_name.clone())
        .with_secure(settings.cookie_secure)
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::minutes(
            settings.session_ttl_minutes,
        )));
    let csp = content_security_policy(&state.ui.chartjs_url);

    let mut app = Router::new()
        .route("/", get(handlers::index))
        .route(
            "/login",
            get(handlers::login_page).post(handlers::login_submit),
        )
        .route("/login/oidc", get(handlers::oidc_login))
        .route("/login/github", get(handlers::github_login))
        .route("/login/github/callback", get(handlers::github_callback))
        .route("/auth/callback", get(handlers::oidc_callback))
        .route("/logout", post(handlers::logout))
        .route("/scripts/{name}", get(handlers::script_page))
        .route("/api/scripts", get(handlers::api_list))
        .route("/api/scripts/{name}", get(handlers::api_run))
        .route("/healthz", get(handlers::healthz))
        .route("/readyz", get(handlers::readyz))
        .route("/static/app.css", get(handlers::app_css))
        .route("/static/app.js", get(handlers::app_js));

    if settings.metrics_enabled {
        app = app.route(&settings.metrics_path, get(handlers::metrics));
    }

    app.fallback(handlers::not_found)
        .route_layer(from_fn(track_metrics))
        .layer(session_layer)
        .layer(from_fn_with_state(csp, security_headers))
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request| {
            let request_id = req
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-");
            // Only the path is logged: query strings may carry parameters.
            info_span!(
                "http_request",
                method = %req.method(),
                path = %req.uri().path(),
                request_id = %request_id
            )
        }))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .with_state(state)
}
