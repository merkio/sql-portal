//! GitHub OAuth sign-in: the adapter against a mock GitHub, and the sign-in
//! over HTTP with the fake provider.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::Form;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::{harness, Options};
use script_portal::domain::ports::GithubLogin;
use script_portal::domain::{AuthError, AuthProvider};
use script_portal::infrastructure::config::model::GithubConfig;
use script_portal::infrastructure::github::GithubAdapter;
use serde_json::json;

/// A mock GitHub: the web and API endpoints on one address. The code `good`
/// yields the token `tok`, which opens the user `octocat`.
async fn spawn_github() -> String {
    let app = Router::new()
        .route(
            "/login/oauth/access_token",
            post(|Form(form): Form<HashMap<String, String>>| async move {
                if form.get("code").map(String::as_str) == Some("good") {
                    Json(json!({"access_token": "tok"}))
                } else {
                    Json(json!({"error": "bad_verification_code"}))
                }
            }),
        )
        .route(
            "/user",
            get(|headers: HeaderMap| async move {
                if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer tok")
                {
                    return (StatusCode::UNAUTHORIZED, Json(json!({})));
                }
                // No public email: the adapter must look it up.
                (
                    StatusCode::OK,
                    Json(json!({"login": "octocat", "name": null, "email": null})),
                )
            }),
        )
        .route(
            "/user/emails",
            get(|| async {
                Json(json!([
                    {"email": "old@example.org", "primary": false, "verified": true},
                    {"email": "octo@example.org", "primary": true, "verified": true}
                ]))
            }),
        )
        .route(
            "/user/orgs",
            get(|| async { Json(json!([{"login": "acme"}, {"login": "other"}])) }),
        )
        .route(
            "/user/teams",
            get(|| async {
                Json(json!([{"slug": "analysts", "organization": {"login": "acme"}}]))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn adapter(base: &str, allowed_orgs: &[&str]) -> GithubAdapter {
    let cfg: GithubConfig = serde_yaml_ng::from_str(&format!(
        "client_id: app-id\nclient_secret: app-secret\nweb_url: {base}\napi_url: {base}\nallowed_orgs: [{}]\n",
        allowed_orgs.join(", ")
    ))
    .unwrap();
    GithubAdapter::new(&cfg, "https://portal.example.org/").unwrap()
}

#[tokio::test]
async fn sign_in_loads_the_user_with_organisations_and_teams_as_groups() {
    let base = spawn_github().await;
    let user = adapter(&base, &[]).complete("good").await.unwrap();
    assert_eq!(user.subject, "octocat");
    assert_eq!(user.display_name, "octocat", "no name: the login is shown");
    assert_eq!(
        user.email.as_deref(),
        Some("octo@example.org"),
        "primary verified email"
    );
    assert_eq!(user.provider, AuthProvider::Github);
    assert!(user.groups.contains(&"acme".to_string()));
    assert!(user.groups.contains(&"other".to_string()));
    assert!(
        user.groups.contains(&"acme/analysts".to_string()),
        "teams are org/slug"
    );
}

#[tokio::test]
async fn a_rejected_code_is_rejected_not_unavailable() {
    let base = spawn_github().await;
    let err = adapter(&base, &[]).complete("forged").await.unwrap_err();
    assert!(matches!(err, AuthError::Rejected(m) if m.contains("bad_verification_code")));
}

#[tokio::test]
async fn only_members_of_allowed_organisations_may_sign_in() {
    let base = spawn_github().await;
    assert!(
        adapter(&base, &["ACME"]).complete("good").await.is_ok(),
        "case-insensitive"
    );
    let err = adapter(&base, &["nope"])
        .complete("good")
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::Rejected(m) if m.contains("allowed organisation")));
}

#[tokio::test]
async fn the_authorization_url_asks_for_the_needed_scopes() {
    let url = url::Url::parse(&adapter("https://github.com", &[]).authorize_url("st4te")).unwrap();
    assert_eq!(url.host_str(), Some("github.com"));
    assert_eq!(url.path(), "/login/oauth/authorize");
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "app-id");
    assert_eq!(query["state"], "st4te");
    assert_eq!(
        query["redirect_uri"],
        "https://portal.example.org/login/github/callback"
    );
    assert!(query["scope"].contains("read:org"), "teams need read:org");
}

#[tokio::test]
async fn sign_in_over_http_checks_the_state_and_creates_the_session() {
    let h = harness(Options {
        github: true,
        ..Options::default()
    })
    .await;

    // The login page offers GitHub.
    let login = h.get("/login", None).await;
    assert!(login.body.contains("href=\"/login/github\""));

    // Starting the sign-in redirects to GitHub with a fresh state.
    let start = h.get("/login/github", None).await;
    assert_eq!(start.status, StatusCode::SEE_OTHER);
    let location = start.headers["location"].to_str().unwrap().to_string();
    assert!(location.starts_with("https://github.example.org/login/oauth/authorize?state="));
    let state = location.split("state=").nth(1).unwrap().to_string();
    let cookie = start
        .session_cookie()
        .expect("the state is kept in the session");

    // A callback with a different state is refused.
    let forged = h
        .get(
            "/login/github/callback?code=good-code&state=other",
            Some(&cookie),
        )
        .await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    // The state was consumed by that attempt, so the genuine one needs a new start.
    let start = h.get("/login/github", None).await;
    let cookie = start.session_cookie().unwrap();
    let state2 = start.headers["location"]
        .to_str()
        .unwrap()
        .split("state=")
        .nth(1)
        .unwrap()
        .to_string();
    assert_ne!(state, state2, "every sign-in gets a new state");

    // A rejected code is refused.
    let bad = h
        .get(
            &format!("/login/github/callback?code=bad&state={state2}"),
            Some(&cookie),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);

    // The genuine callback signs the user in.
    let start = h.get("/login/github", None).await;
    let cookie = start.session_cookie().unwrap();
    let state3 = start.headers["location"]
        .to_str()
        .unwrap()
        .split("state=")
        .nth(1)
        .unwrap()
        .to_string();
    let done = h
        .get(
            &format!("/login/github/callback?code=good-code&state={state3}"),
            Some(&cookie),
        )
        .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    assert_eq!(done.headers["location"], "/");
    // The session ID changes on sign-in, so the callback sets the new cookie.
    let signed_in = done.session_cookie().expect("a new session after sign-in");
    let home = h.get("/", Some(&signed_in)).await;
    assert!(
        home.body.contains("The Octocat"),
        "the signed-in name is shown"
    );
}

#[tokio::test]
async fn callback_without_a_started_sign_in_is_refused() {
    let h = harness(Options {
        github: true,
        ..Options::default()
    })
    .await;
    let r = h
        .get("/login/github/callback?code=good-code&state=x", None)
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[test]
fn github_config_needs_its_client_credentials_at_startup() {
    use script_portal::infrastructure::config::load_from_str;
    let yaml = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/config.demo.yaml"),
    )
    .unwrap()
    .replace(
        "auth:\n  enabled: false",
        "auth:\n  enabled: true\n  github:\n    client_id: \"\"\n    client_secret: \"s\"",
    );
    let cfg = load_from_str(
        &yaml,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config")
            .as_path(),
    )
    .unwrap();
    let Err(err) = cfg.into_runtime() else {
        panic!("an empty client id must be rejected")
    };
    assert!(err.to_string().contains("client_id and client_secret"));
    let _ = Arc::new(());
}

#[test]
fn the_example_github_config_loads_with_its_environment() {
    use script_portal::infrastructure::config::load_from_path;
    std::env::set_var("GITHUB_CLIENT_ID", "example-id");
    std::env::set_var("GITHUB_CLIENT_SECRET", "example-secret");
    let cfg = load_from_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/config.github.example.yaml")
            .as_path(),
    )
    .unwrap();
    let rt = cfg.into_runtime().unwrap();
    assert!(matches!(
        rt.auth,
        script_portal::infrastructure::config::runtime::AuthRuntime::Enabled {
            github: Some(_),
            ..
        }
    ));
}
