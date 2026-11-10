use std::collections::HashMap;

use axum::extract::{Form, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tower_sessions::Session;

use super::extractors::{
    csrf_token, verify_csrf, ApiPrincipal, WebPrincipal, SESSION_CSRF, SESSION_GITHUB,
    SESSION_OIDC, SESSION_USER,
};
use super::state::AppState;
use super::views::{self, Nav};
use sql_portal_application::{OidcCallback, ScriptRun};
use sql_portal_domain::scripts::MAX_PAGE_SIZE;
use sql_portal_domain::OidcPending;
use sql_portal_domain::{
    AuthError, PageRequest, ParamKind, Principal, QueryError, ScriptDefinition, ScriptError, User,
};

type HtmlResponse = (StatusCode, Html<String>);

// ---------------------------------------------------------------- helpers

async fn nav_for(state: &AppState, session: &Session, principal: &Principal) -> Nav {
    let logout_csrf = if state.auth.is_enabled() {
        csrf_token(session).await.ok()
    } else {
        None
    };
    Nav {
        display_name: principal.user.display_name.clone(),
        logout_csrf,
    }
}

fn html_page(
    state: &AppState,
    status: StatusCode,
    title: &str,
    nav: Option<&Nav>,
    body: &str,
    with_charts: bool,
) -> HtmlResponse {
    (
        status,
        Html(views::page(&state.ui, title, nav, body, with_charts)),
    )
}

async fn store_login(session: &Session, user: &User) -> Result<(), StatusCode> {
    // New session id on privilege change prevents session fixation.
    session
        .cycle_id()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    session
        .insert(SESSION_USER, user)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

// ------------------------------------------------------------------ pages

pub async fn index(
    State(state): State<AppState>,
    session: Session,
    WebPrincipal(principal): WebPrincipal,
    Query(query): Query<HashMap<String, String>>,
) -> HtmlResponse {
    let nav = nav_for(&state, &session, &principal).await;
    let show_all = query.get("all").is_some_and(|v| v == "1");
    let all = state.scripts.list_all(&principal);
    let locked = all.iter().filter(|s| !s.accessible).count();
    let scripts: Vec<_> = if show_all {
        all
    } else {
        all.into_iter().filter(|s| s.accessible).collect()
    };
    html_page(
        &state,
        StatusCode::OK,
        "Scripts",
        Some(&nav),
        &views::index_body(&scripts, show_all, locked),
        false,
    )
}

pub async fn not_found(State(state): State<AppState>) -> HtmlResponse {
    html_page(
        &state,
        StatusCode::NOT_FOUND,
        "Not found",
        None,
        &views::message_body("Page not found", "The page you asked for does not exist."),
        false,
    )
}

// ------------------------------------------------------------------- auth

async fn render_login(
    state: &AppState,
    session: &Session,
    status: StatusCode,
    error: Option<&str>,
) -> Response {
    let Ok(csrf) = csrf_token(session).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let body = views::login_body(
        state.auth.ldap_enabled(),
        state.auth.oidc_enabled(),
        state.auth.github_enabled(),
        &csrf,
        error,
    );
    html_page(state, status, "Sign in", None, &body, false).into_response()
}

pub async fn login_page(State(state): State<AppState>, session: Session) -> Response {
    if !state.auth.is_enabled() {
        return Redirect::to("/").into_response();
    }
    if matches!(session.get::<User>(SESSION_USER).await, Ok(Some(_))) {
        return Redirect::to("/").into_response();
    }
    render_login(&state, &session, StatusCode::OK, None).await
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    csrf: String,
}

pub async fn login_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Response {
    if !state.auth.ldap_enabled() {
        return render_login(
            &state,
            &session,
            StatusCode::BAD_REQUEST,
            Some("Directory sign-in is not enabled."),
        )
        .await;
    }
    if !verify_csrf(&session, &form.csrf).await {
        return render_login(
            &state,
            &session,
            StatusCode::FORBIDDEN,
            Some("Your session expired. Please try again."),
        )
        .await;
    }
    match state.auth.login_ldap(&form.username, &form.password).await {
        Ok(user) => match store_login(&session, &user).await {
            Ok(()) => Redirect::to("/").into_response(),
            Err(status) => status.into_response(),
        },
        Err(e) => {
            let (status, message) = auth_error_message(&e);
            render_login(&state, &session, status, Some(message)).await
        }
    }
}

fn auth_error_message(e: &AuthError) -> (StatusCode, &'static str) {
    match e {
        AuthError::InvalidCredentials => {
            (StatusCode::UNAUTHORIZED, "Invalid username or password.")
        }
        AuthError::ProviderUnavailable(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The sign-in service is temporarily unavailable. Please try again later.",
        ),
        AuthError::Disabled | AuthError::ProviderNotEnabled => (
            StatusCode::BAD_REQUEST,
            "This sign-in method is not enabled.",
        ),
        AuthError::Rejected(_) => (StatusCode::UNAUTHORIZED, "Sign-in was rejected."),
    }
}

pub async fn oidc_login(State(state): State<AppState>, session: Session) -> Response {
    match state.auth.oidc_begin().await {
        Ok(auth) => {
            if session.insert(SESSION_OIDC, &auth.pending).await.is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Redirect::to(&auth.redirect_url).into_response()
        }
        Err(e) => {
            let (status, message) = auth_error_message(&e);
            render_login(&state, &session, status, Some(message)).await
        }
    }
}

/// Starts a GitHub sign-in: remembers the state and sends the user to GitHub.
pub async fn github_login(State(state): State<AppState>, session: Session) -> Response {
    match state.auth.github_begin().await {
        Ok(auth) => {
            if session.insert(SESSION_GITHUB, &auth.state).await.is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Redirect::to(&auth.redirect_url).into_response()
        }
        Err(e) => {
            let (status, message) = auth_error_message(&e);
            render_login(&state, &session, status, Some(message)).await
        }
    }
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// GitHub returns the user here with a code and the state we sent.
pub async fn github_callback(
    State(state): State<AppState>,
    session: Session,
    Query(params): Query<CallbackParams>,
) -> Response {
    if params.error.is_some() {
        return render_login(
            &state,
            &session,
            StatusCode::UNAUTHORIZED,
            Some("GitHub denied the sign-in."),
        )
        .await;
    }
    let expected: Option<String> = session.remove(SESSION_GITHUB).await.ok().flatten();
    let (Some(expected), Some(code), Some(returned)) = (expected, params.code, params.state) else {
        return render_login(
            &state,
            &session,
            StatusCode::BAD_REQUEST,
            Some("The sign-in request is invalid or has expired."),
        )
        .await;
    };
    match state
        .auth
        .github_complete(&code, &expected, &returned)
        .await
    {
        Ok(user) => match store_login(&session, &user).await {
            Ok(()) => Redirect::to("/").into_response(),
            Err(status) => status.into_response(),
        },
        Err(e) => {
            let (status, message) = auth_error_message(&e);
            render_login(&state, &session, status, Some(message)).await
        }
    }
}

pub async fn oidc_callback(
    State(state): State<AppState>,
    session: Session,
    Query(params): Query<CallbackParams>,
) -> Response {
    if params.error.is_some() {
        return render_login(
            &state,
            &session,
            StatusCode::UNAUTHORIZED,
            Some("The identity provider denied the sign-in."),
        )
        .await;
    }
    let pending: Option<OidcPending> = session.remove(SESSION_OIDC).await.ok().flatten();
    let (Some(pending), Some(code), Some(returned_state)) = (pending, params.code, params.state)
    else {
        return render_login(
            &state,
            &session,
            StatusCode::BAD_REQUEST,
            Some("The sign-in request is invalid or has expired."),
        )
        .await;
    };

    let callback = OidcCallback {
        code,
        state: returned_state,
    };
    match state.auth.oidc_complete(&callback, &pending).await {
        Ok(user) => match store_login(&session, &user).await {
            Ok(()) => Redirect::to("/").into_response(),
            Err(status) => status.into_response(),
        },
        Err(e) => {
            let (status, message) = auth_error_message(&e);
            render_login(&state, &session, status, Some(message)).await
        }
    }
}

#[derive(Deserialize)]
pub struct LogoutForm {
    csrf: String,
}

pub async fn logout(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<LogoutForm>,
) -> Response {
    if !verify_csrf(&session, &form.csrf).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    let _ = session.remove::<String>(SESSION_CSRF).await;
    if session.flush().await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if state.auth.is_enabled() {
        Redirect::to("/login").into_response()
    } else {
        Redirect::to("/").into_response()
    }
}

// ---------------------------------------------------------------- scripts

/// HTML checkboxes are absent from submissions when unchecked; once the form
/// was submitted, absence therefore means `false`.
fn normalize_checkboxes(script: &ScriptDefinition, params: &mut HashMap<String, String>) {
    if !params.contains_key("_run") {
        return;
    }
    for p in script
        .params
        .iter()
        .filter(|p| p.kind == ParamKind::Boolean)
    {
        params
            .entry(p.name.clone())
            .or_insert_with(|| "false".into());
    }
}

fn query_error_message(e: &QueryError, timeout_secs: u64) -> (StatusCode, String) {
    match e {
        QueryError::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            format!("The query did not finish within {timeout_secs} seconds."),
        ),
        QueryError::Unavailable(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The database is currently unavailable.".into(),
        ),
        QueryError::Database(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "The query could not be completed. The error was logged.".into(),
        ),
    }
}

pub async fn script_page(
    State(state): State<AppState>,
    session: Session,
    WebPrincipal(principal): WebPrincipal,
    Path(name): Path<String>,
    Query(mut params): Query<HashMap<String, String>>,
) -> HtmlResponse {
    let nav = nav_for(&state, &session, &principal).await;
    let script = match state.scripts.authorize(&principal, &name) {
        Ok(s) => s,
        Err(ScriptError::Forbidden) => {
            return html_page(
                &state,
                StatusCode::FORBIDDEN,
                "Forbidden",
                Some(&nav),
                &views::message_body(
                    "Access denied",
                    "You do not have permission to run this script.",
                ),
                false,
            )
        }
        Err(_) => {
            return html_page(
                &state,
                StatusCode::NOT_FOUND,
                "Not found",
                Some(&nav),
                &views::message_body("Script not found", "There is no script with that name."),
                false,
            )
        }
    };

    let should_run = params.contains_key("_run") || script.runs_without_input();
    if !should_run {
        let body = views::script_body(&script, &params, &[], None, None);
        return html_page(
            &state,
            StatusCode::OK,
            &script.title,
            Some(&nav),
            &body,
            false,
        );
    }
    normalize_checkboxes(&script, &mut params);

    let page = PageRequest {
        number: params
            .get("_page")
            .and_then(|p| p.parse::<usize>().ok())
            .unwrap_or(1),
        size: params
            .get("_size")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|s| (1..=MAX_PAGE_SIZE).contains(s))
            .unwrap_or(script.page_size),
    };
    match state.scripts.run(script.clone(), &params, Some(page)).await {
        Ok(run) => {
            let with_charts = run.charts.iter().any(Result::is_ok);
            let body = views::script_body(&script, &params, &[], Some(&run), None);
            html_page(
                &state,
                StatusCode::OK,
                &script.title,
                Some(&nav),
                &body,
                with_charts,
            )
        }
        Err(ScriptError::InvalidParams(errors)) => {
            let body = views::script_body(
                &script,
                &params,
                &errors,
                None,
                Some("Please correct the highlighted fields."),
            );
            html_page(
                &state,
                StatusCode::UNPROCESSABLE_ENTITY,
                &script.title,
                Some(&nav),
                &body,
                false,
            )
        }
        Err(ScriptError::Query(e)) => {
            let (status, message) = query_error_message(&e, script.timeout_secs);
            let body = views::script_body(&script, &params, &[], None, Some(&message));
            html_page(&state, status, &script.title, Some(&nav), &body, false)
        }
        Err(ScriptError::Forbidden) | Err(ScriptError::NotFound) => html_page(
            &state,
            StatusCode::NOT_FOUND,
            "Not found",
            Some(&nav),
            &views::message_body("Script not found", "There is no script with that name."),
            false,
        ),
    }
}

// -------------------------------------------------------------------- API

pub async fn api_list(
    State(state): State<AppState>,
    ApiPrincipal(principal): ApiPrincipal,
    Query(query): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let listed = if query.get("all").is_some_and(|v| v == "1") {
        state.scripts.list_all(&principal)
    } else {
        state.scripts.list_available(&principal)
    };
    let scripts: Vec<_> = listed
        .into_iter()
        .map(|s| {
            json!({
                "name": s.name,
                "title": s.title,
                "description": s.description,
                "group": s.group,
                "accessible": s.accessible,
                "required_permissions": s.required_permissions,
            })
        })
        .collect();
    Json(json!({ "scripts": scripts }))
}

fn run_json(run: &ScriptRun) -> serde_json::Value {
    json!({
        "script": run.script.name.as_str(),
        "title": run.script.title,
        "columns": run.result.columns,
        "rows": run.result.rows,
        "truncated": run.result.truncated,
        "duration_ms": run.duration.as_millis() as u64,
    })
}

pub async fn api_run(
    State(state): State<AppState>,
    ApiPrincipal(principal): ApiPrincipal,
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let script = match state.scripts.authorize(&principal, &name) {
        Ok(s) => s,
        Err(ScriptError::NotFound) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "script not found"})),
            )
                .into_response()
        }
        Err(_) => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "permission denied"})),
            )
                .into_response()
        }
    };
    match state.scripts.run(script.clone(), &params, None).await {
        Ok(run) => Json(run_json(&run)).into_response(),
        Err(ScriptError::NotFound | ScriptError::Forbidden) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": "script not found"}))).into_response()
        }
        Err(ScriptError::InvalidParams(errors)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "invalid parameters",
                "fields": errors.iter().map(|e| json!({"param": e.param, "message": e.message})).collect::<Vec<_>>()
            })),
        )
            .into_response(),
        Err(ScriptError::Query(e)) => {
            let (status, message) = query_error_message(&e, script.timeout_secs);
            (status, Json(json!({"error": message}))).into_response()
        }
    }
}

// ------------------------------------------------------------- operations

pub async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

pub async fn readyz(State(state): State<AppState>) -> Response {
    let report = state.health.readiness().await;
    let status = if report.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let checks: Vec<_> = report
        .checks
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "status": if c.ok { "up" } else { "down" },
                "critical": c.critical,
                "detail": c.detail,
            })
        })
        .collect();
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"status": if report.ready {"ready"} else {"not_ready"}, "checks": checks})),
    )
        .into_response()
}

pub async fn metrics(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        (state.metrics)(),
    )
        .into_response()
}

pub async fn app_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        include_str!("assets/app.css"),
    )
        .into_response()
}

pub async fn app_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        include_str!("assets/app.js"),
    )
        .into_response()
}
