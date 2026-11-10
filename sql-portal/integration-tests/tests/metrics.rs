//! Verifies the Prometheus pipeline end to end. This lives in its own test
//! binary because the `metrics` recorder is process-global.

use sql_portal_integration_tests::harness as common;

use std::time::Duration;

use common::{harness, Options};
use sql_portal_adapters::telemetry::{describe_and_init, install_prometheus, MetricsTelemetry};
use sql_portal_domain::Telemetry;

#[tokio::test]
async fn http_and_business_metrics_are_exposed() {
    let handle = install_prometheus().expect("install recorder once");
    describe_and_init("test", true);

    let h = harness(Options {
        auth_enabled: false,
        ..Options::default()
    })
    .await;
    h.get("/healthz", None).await;
    h.get("/scripts/monthly_trend", None).await;
    h.get("/definitely-not-a-route", None).await;
    MetricsTelemetry.script_run("monthly_trend", "success", Duration::from_millis(5), 6);
    MetricsTelemetry.auth_attempt("ldap", "invalid_credentials");

    let text = handle.render();
    assert!(
        text.contains(r#"http_requests_total{method="GET",path="/healthz",status="200"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"path="/scripts/{name}""#),
        "route templates, not raw paths"
    );
    assert!(text.contains("http_request_duration_seconds_bucket"));
    assert!(text.contains("http_requests_in_flight"));
    assert!(
        text.contains(r#"auth_attempts_total{provider="ldap",outcome="invalid_credentials"} 1"#)
    );
    assert!(text.contains(r#"script_runs_total{script="monthly_trend",outcome="success"} 1"#));
    assert!(text.contains(r#"build_info{version="test"} 1"#));
    assert!(text.contains("auth_enabled 1"));
    // Unknown paths never create per-path series (bounded cardinality).
    assert!(!text.contains("definitely-not-a-route"));
}
