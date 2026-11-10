//! End-to-end checks of the shipped Docker image (`--features docker`).
//!
//! Each test starts its own portal container from `SCRIPT_PORTAL_IMAGE`
//! (default `script-portal:test`, build it with `make image-test`), checks it
//! over HTTP or with `exec`, and the container is stopped and removed when the
//! test ends.

#![cfg(feature = "docker")]

use reqwest::StatusCode;
use serde_json::Value;
use sql_portal_integration_tests::containers::{
    portal_image_ref, postgres_image, unique_name, Portal, PortalOptions, LABEL,
};
use testcontainers_modules::testcontainers::core::WaitFor;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{GenericImage, ImageExt};

async fn json(portal: &Portal, path: &str) -> (StatusCode, Value) {
    let response = portal.get(path).await.expect("request succeeds");
    let status = response.status();
    (status, response.json().await.expect("JSON body"))
}

// ------------------------------------------------------------ demo image

#[tokio::test]
async fn demo_image_is_live_and_ready() {
    let portal = Portal::start(PortalOptions::default()).await;

    let live = portal.get("/healthz").await.unwrap();
    assert_eq!(live.status(), StatusCode::OK);

    let (status, ready) = json(&portal, "/readyz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["status"], "ready", "{ready}");
    assert_eq!(ready["checks"][0]["name"], "database", "{ready}");
    assert_eq!(ready["checks"][0]["status"], "up", "{ready}");
}

#[tokio::test]
async fn demo_image_lists_and_runs_scripts_through_the_api() {
    let portal = Portal::start(PortalOptions::default()).await;

    let (status, list) = json(&portal, "/api/scripts").await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = list["scripts"]
        .as_array()
        .expect("scripts array")
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(names.contains(&"top_products"), "{names:?}");

    let (status, run) = json(&portal, "/api/scripts/top_products?limit=2").await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(
        run["columns"],
        serde_json::json!(["product", "units", "revenue"])
    );
    assert_eq!(run["rows"].as_array().unwrap().len(), 2);

    let (status, invalid) = json(&portal, "/api/scripts/top_products?limit=0").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid["fields"][0]["param"], "limit");
}

#[tokio::test]
async fn demo_image_serves_html_with_security_headers() {
    let portal = Portal::start(PortalOptions::default()).await;

    let page = portal.get("/").await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let headers = page.headers();
    for name in [
        "content-security-policy",
        "x-content-type-options",
        "x-frame-options",
        "referrer-policy",
        "permissions-policy",
    ] {
        assert!(headers.contains_key(name), "missing {name}: {headers:?}");
    }
    assert!(page.text().await.unwrap().contains("<html"));
}

#[tokio::test]
async fn demo_image_exposes_prometheus_metrics() {
    let portal = Portal::start(PortalOptions::default()).await;
    portal.get("/healthz").await.unwrap();

    let metrics = portal.get("/metrics").await.unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let text = metrics.text().await.unwrap();
    assert!(
        text.contains(r#"http_requests_total{method="GET",path="/healthz",status="200"}"#),
        "{text}"
    );
    assert!(text.contains("build_info{version="), "{text}");
    assert!(text.contains("auth_enabled 0"), "{text}");
}

/// The Dockerfile's HEALTHCHECK runs this command. Podman ignores HEALTHCHECK
/// in OCI images, so the command is exercised directly.
#[tokio::test]
async fn built_in_healthcheck_command_reports_health() {
    let portal = Portal::start(PortalOptions::default()).await;

    let (code, output) = portal
        .exec(&[
            "/usr/local/bin/script-portal",
            "healthcheck",
            "--addr",
            "127.0.0.1:8080",
            "--path",
            "/readyz",
        ])
        .await;
    assert_eq!(code, Some(0), "{output}");

    let (code, output) = portal
        .exec(&[
            "/usr/local/bin/script-portal",
            "healthcheck",
            "--addr",
            "127.0.0.1:8080",
            "--path",
            "/no-such-path",
        ])
        .await;
    assert_ne!(code, Some(0), "unhealthy path must fail: {output}");
}

// ------------------------------------------------------------ CLI in the image

fn portal_image() -> GenericImage {
    let (name, tag) = portal_image_ref();
    GenericImage::new(name, tag)
}

#[tokio::test]
async fn check_config_validates_the_bundled_configuration() {
    let _container = portal_image()
        .with_wait_for(WaitFor::message_on_either_std("configuration OK"))
        .with_cmd(["check-config"])
        .with_label(LABEL.0, LABEL.1)
        .start()
        .await
        .expect("check-config succeeds on the demo configuration");
}

#[tokio::test]
async fn invalid_configuration_fails_at_startup() {
    // Authentication enabled without any provider is rejected.
    let config = "server:\n  bind: 0.0.0.0:8080\ndatabase:\n  url: \"sqlite::memory:\"\nauth:\n  enabled: true\nscripts: []\n";
    let _container = portal_image()
        .with_wait_for(WaitFor::message_on_stderr("Error"))
        .with_copy_to("/app/config/test.yaml", config.as_bytes().to_vec())
        .with_env_var("SCRIPT_PORTAL_CONFIG", "/app/config/test.yaml")
        .with_label(LABEL.0, LABEL.1)
        .start()
        .await
        .expect("the portal reports the configuration error");
}

// ------------------------------------------------------------ image + PostgreSQL

#[tokio::test]
async fn image_runs_scripts_against_postgres_on_a_shared_network() {
    let network = unique_name("sql-portal-it");
    let pg_host = unique_name("pg");
    let _postgres = postgres_image()
        .with_network(&network)
        .with_container_name(&pg_host)
        .start()
        .await
        .expect("PostgreSQL starts");

    let config = format!(
        r#"server:
  bind: "0.0.0.0:8080"
database:
  url: "postgres://postgres:postgres@{pg_host}:5432/postgres"
auth:
  enabled: false
scripts:
  - name: server_version
    title: Server version
    sql: "SELECT version() AS version"
  - name: doubled
    title: Doubled
    sql: "SELECT CAST(:n AS integer) * 2 AS doubled"
    params:
      - {{ name: n, type: integer, required: true }}
"#
    );
    let portal = Portal::start(PortalOptions {
        config: Some(config),
        network: Some(network),
        ..Default::default()
    })
    .await;

    let (status, ready) = json(&portal, "/readyz").await;
    assert_eq!(status, StatusCode::OK, "{ready}");
    assert_eq!(
        ready["checks"][0]["status"], "up",
        "database probe: {ready}"
    );

    let (status, version) = json(&portal, "/api/scripts/server_version").await;
    assert_eq!(status, StatusCode::OK, "{version}");
    let version = version["rows"][0][0].as_str().unwrap_or_default();
    assert!(version.starts_with("PostgreSQL 16"), "{version}");

    let (status, doubled) = json(&portal, "/api/scripts/doubled?n=21").await;
    assert_eq!(status, StatusCode::OK, "{doubled}");
    assert_eq!(doubled["rows"][0][0], 42);
}
