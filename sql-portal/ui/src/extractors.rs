use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde_json::json;
use tower_sessions::Session;

use super::state::AppState;
use sql_portal_domain::{Principal, User};

pub const SESSION_USER: &str = "user";
pub const SESSION_OIDC: &str = "oidc_pending";
pub const SESSION_GITHUB: &str = "github_state";
pub const SESSION_CSRF: &str = "csrf";

// The error is the HTTP response sent to the client as-is; boxing it buys nothing.
#[allow(clippy::result_large_err)]
async fn principal_from_session(
    parts: &mut Parts,
    state: &AppState,
) -> Result<Option<Principal>, Response> {
    let session = Session::from_request_parts(parts, state)
        .await
        .map_err(IntoResponse::into_response)?;
    let user: Option<User> = session
        .get(SESSION_USER)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    Ok(state.auth.principal(user).await)
}

/// Authenticated principal for HTML pages; anonymous requests are redirected
/// to the login page.
pub struct WebPrincipal(pub Principal);

impl FromRequestParts<AppState> for WebPrincipal {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match principal_from_session(parts, state).await? {
            Some(p) => Ok(WebPrincipal(p)),
            None => Err(Redirect::to("/login").into_response()),
        }
    }
}

/// Authenticated principal for the JSON API; anonymous requests get a 401.
pub struct ApiPrincipal(pub Principal);

impl FromRequestParts<AppState> for ApiPrincipal {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match principal_from_session(parts, state).await? {
            Some(p) => Ok(ApiPrincipal(p)),
            None => Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "authentication required"})),
            )
                .into_response()),
        }
    }
}

/// Returns the session's CSRF token, creating it on first use.
pub async fn csrf_token(session: &Session) -> Result<String, StatusCode> {
    if let Some(token) = session
        .get::<String>(SESSION_CSRF)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Ok(token);
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    session
        .insert(SESSION_CSRF, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(token)
}

/// Verifies a submitted CSRF token in constant time.
pub async fn verify_csrf(session: &Session, submitted: &str) -> bool {
    match session.get::<String>(SESSION_CSRF).await {
        Ok(Some(expected)) => {
            let (a, b) = (expected.as_bytes(), submitted.as_bytes());
            a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
        }
        _ => false,
    }
}
