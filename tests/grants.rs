//! The external grants service over real HTTP, with the local cache in front.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use script_portal::domain::access::identity::{Grants, User};
use script_portal::domain::ports::GrantSource;
use script_portal::domain::{AuthProvider, Permission};
use script_portal::infrastructure::grants::{CachedGrants, RemoteGrants};
use script_portal::infrastructure::tls::TlsOptions;
use serde_json::{json, Value};

const TOKEN: &str = "s3cret";

struct Service {
    url: String,
    lookups: Arc<AtomicUsize>,
}

/// A stand-in grants service: `alice` has grants, `bad` fails, others are unknown.
async fn spawn_service() -> Service {
    let lookups = Arc::new(AtomicUsize::new(0));
    let counter = lookups.clone();
    let app = Router::new().route(
        "/permissions",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let authorised = headers.get("authorization").and_then(|v| v.to_str().ok())
                    == Some(&format!("Bearer {TOKEN}"));
                if !authorised {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": "token"})));
                }
                match body["subject"].as_str() {
                    Some("alice") => (
                        StatusCode::OK,
                        Json(json!({"roles": ["analyst"], "permissions": ["scripts.sales.read"]})),
                    ),
                    Some("bad") => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({}))),
                    _ => (StatusCode::NOT_FOUND, Json(json!({}))),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Service {
        url: format!("http://{addr}/permissions"),
        lookups,
    }
}

fn user(subject: &str) -> User {
    User {
        subject: subject.into(),
        display_name: subject.into(),
        email: Some(format!("{subject}@example.org")),
        groups: vec!["analysts".into()],
        provider: AuthProvider::Oidc,
    }
}

fn remote(url: &str, token: Option<&str>) -> RemoteGrants {
    RemoteGrants::new(
        url,
        token.map(str::to_string),
        Duration::from_secs(2),
        &TlsOptions::default(),
    )
    .unwrap()
}

fn can(grants: &Grants, permission: &str) -> bool {
    grants
        .permissions
        .grants(&Permission::new(permission).unwrap())
}

#[tokio::test]
async fn grants_are_loaded_once_and_then_served_from_the_cache() {
    let service = spawn_service().await;
    let cache = CachedGrants::new(
        Arc::new(remote(&service.url, Some(TOKEN))),
        Duration::from_secs(60),
    );

    let first = cache.grants_for(&user("alice")).await.unwrap();
    assert!(first.roles.contains("analyst"));
    assert!(can(&first, "scripts.sales.read"));

    let second = cache.grants_for(&user("alice")).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(
        service.lookups.load(Ordering::SeqCst),
        1,
        "second check hit the cache"
    );
}

#[tokio::test]
async fn unknown_identities_hold_no_grants_and_are_cached() {
    let service = spawn_service().await;
    let cache = CachedGrants::new(
        Arc::new(remote(&service.url, Some(TOKEN))),
        Duration::from_secs(60),
    );
    let grants = cache.grants_for(&user("nobody")).await.unwrap();
    assert!(grants.roles.is_empty() && !can(&grants, "scripts.sales.read"));
    cache.grants_for(&user("nobody")).await.unwrap();
    assert_eq!(service.lookups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn wrong_or_missing_token_is_an_error() {
    let service = spawn_service().await;
    assert!(remote(&service.url, Some("wrong"))
        .grants_for(&user("alice"))
        .await
        .is_err());
    assert!(remote(&service.url, None)
        .grants_for(&user("alice"))
        .await
        .is_err());
}

#[tokio::test]
async fn server_errors_are_not_cached() {
    let service = spawn_service().await;
    let cache = CachedGrants::new(
        Arc::new(remote(&service.url, Some(TOKEN))),
        Duration::from_secs(60),
    );
    assert!(cache.grants_for(&user("bad")).await.is_err());
    assert!(cache.grants_for(&user("bad")).await.is_err());
    assert_eq!(
        service.lookups.load(Ordering::SeqCst),
        2,
        "a failure must be retried, not cached"
    );
}

#[tokio::test]
async fn an_unreachable_service_is_an_error() {
    // Port 9 (discard) on localhost is not listening.
    let cache = CachedGrants::new(
        Arc::new(remote("http://127.0.0.1:9/permissions", Some(TOKEN))),
        Duration::from_secs(60),
    );
    assert!(cache.grants_for(&user("alice")).await.is_err());
}
