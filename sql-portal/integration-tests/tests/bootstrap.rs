//! Exercises the real composition root (`build_app`) with the shipped configs.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sql_portal::build_app;
use sql_portal_adapters::config::runtime::AuthRuntime;
use sql_portal_adapters::config::{load_from_path, load_from_str};
use tower::ServiceExt;

fn manifest(path: &str) -> std::path::PathBuf {
    Path::new(sql_portal_integration_tests::REPO_ROOT).join(path)
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn demo_config_builds_a_working_app() {
    let cfg = load_from_path(&manifest("config/config.demo.yaml")).unwrap();
    let rt = cfg.into_runtime().unwrap();
    let app = build_app(&rt, Arc::new(|| "demo_metric 1\n".into()))
        .await
        .unwrap();

    assert_eq!(get(&app, "/readyz").await.0, StatusCode::OK);
    let (status, body) = get(&app, "/scripts/monthly_trend").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("2024-06"));
    assert_eq!(get(&app, "/metrics").await.1, "demo_metric 1\n");
}

#[tokio::test]
async fn metrics_can_be_disabled_by_configuration() {
    let yaml = std::fs::read_to_string(manifest("config/config.demo.yaml"))
        .unwrap()
        .replace(
            "auth:\n  enabled: false",
            "observability:\n  metrics_enabled: false\nauth:\n  enabled: false",
        );
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    let app = build_app(&cfg.into_runtime().unwrap(), Arc::new(String::new))
        .await
        .unwrap();
    assert_eq!(get(&app, "/metrics").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn custom_metrics_path_is_honoured() {
    let yaml = std::fs::read_to_string(manifest("config/config.demo.yaml"))
        .unwrap()
        .replace(
            "auth:\n  enabled: false",
            "observability:\n  metrics_path: /prom\nauth:\n  enabled: false",
        );
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    let app = build_app(&cfg.into_runtime().unwrap(), Arc::new(|| "x 1\n".into()))
        .await
        .unwrap();
    assert_eq!(get(&app, "/prom").await.0, StatusCode::OK);
    assert_eq!(get(&app, "/metrics").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enabling_auth_without_providers_fails_fast() {
    let yaml = std::fs::read_to_string(manifest("config/config.demo.yaml"))
        .unwrap()
        .replace("enabled: false", "enabled: true");
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    // The rule is checked while the runtime value is built, before any adapter starts.
    let Err(err) = cfg.into_runtime() else {
        panic!("auth without providers must be rejected")
    };
    assert!(err
        .to_string()
        .contains("neither auth.ldap, auth.oidc nor auth.github"));
}

#[tokio::test]
async fn unreachable_database_fails_startup() {
    let yaml = std::fs::read_to_string(manifest("config/config.demo.yaml"))
        .unwrap()
        .replace("sqlite::memory:", "sqlite:///nonexistent-dir/x.db?mode=ro");
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    let rt = cfg.into_runtime().unwrap();
    assert!(build_app(&rt, Arc::new(String::new)).await.is_err());
}

#[tokio::test]
async fn example_config_is_valid_when_secrets_are_provided() {
    // The example uses ${ENV} placeholders; supply defaults through the loader.
    let yaml = std::fs::read_to_string(manifest("config/config.example.yaml")).unwrap();
    std::env::set_var("DATABASE_URL", "sqlite::memory:");
    std::env::set_var("LDAP_BIND_PASSWORD", "x");
    std::env::set_var("OIDC_CLIENT_SECRET", "y");
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    let rt = cfg.into_runtime().unwrap();
    assert!(matches!(
        rt.auth,
        AuthRuntime::Enabled {
            ldap: Some(_),
            oidc: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn production_wiring_builds_every_configured_adapter() {
    // LDAP, OIDC and the grants service all point at a closed port: wiring must
    // succeed without connecting, and only the optional OIDC probe reports down.
    let yaml = std::fs::read_to_string(manifest("config/config.demo.yaml"))
        .unwrap()
        .replace(
            "auth:\n  enabled: false",
            "auth:\n  enabled: true\n  ldap:\n    url: \"ldap://127.0.0.1:1\"\n    bind_dn: \"cn=svc\"\n    bind_password: \"p\"\n    base_dn: \"dc=example,dc=org\"\n  oidc:\n    issuer_url: \"https://127.0.0.1:1\"\n    client_id: \"portal\"",
        )
        .replace(
            "access:\n",
            "access:\n  external:\n    url: \"https://127.0.0.1:1/grants\"\n    cache_ttl_secs: 60\n",
        );
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    let rt = cfg.into_runtime().unwrap();
    let app = build_app(&rt, Arc::new(String::new)).await.unwrap();

    let (status, body) = get(&app, "/readyz").await;
    assert_eq!(status, StatusCode::OK, "OIDC is optional: {body}");
    assert!(
        body.contains("\"name\":\"oidc\"") && body.contains("\"status\":\"down\""),
        "{body}"
    );
}

#[tokio::test]
async fn migrations_run_only_when_enabled_explicitly() {
    let enabled = std::fs::read_to_string(manifest("config/config.demo.yaml")).unwrap();
    assert!(enabled.contains("migrations:\n    enabled: true"));

    // Demo data is there when migrations are enabled.
    let cfg = load_from_str(&enabled, &manifest("config")).unwrap();
    let app = build_app(&cfg.into_runtime().unwrap(), Arc::new(String::new))
        .await
        .unwrap();
    assert_eq!(get(&app, "/scripts/monthly_trend").await.0, StatusCode::OK);

    // Without the opt-in the schema does not exist, so the script cannot run.
    let disabled = enabled.replace(
        "migrations:\n    enabled: true",
        "migrations:\n    enabled: false",
    );
    let cfg = load_from_str(&disabled, &manifest("config")).unwrap();
    let app = build_app(&cfg.into_runtime().unwrap(), Arc::new(String::new))
        .await
        .unwrap();
    let (status, body) = get(&app, "/scripts/monthly_trend").await;
    assert_ne!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn enabled_migrations_need_an_existing_directory() {
    let yaml = std::fs::read_to_string(manifest("config/config.demo.yaml"))
        .unwrap()
        .replace("dir: migrations/sqlite", "dir: migrations/does-not-exist");
    let cfg = load_from_str(&yaml, &manifest("config")).unwrap();
    let Err(err) = cfg.into_runtime() else {
        panic!("a missing migrations directory must be rejected")
    };
    assert!(err.to_string().contains("not a directory"));
}
