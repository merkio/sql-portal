use sql_portal_integration_tests::harness as common;

use axum::http::StatusCode;
use common::{failing_noncritical_probe, failing_probe, harness, Options};

fn no_auth() -> Options {
    Options {
        auth_enabled: false,
        ..Options::default()
    }
}

// ------------------------------------------------------ authentication

#[tokio::test]
async fn anonymous_users_are_redirected_to_login() {
    let h = harness(Options::default()).await;
    let r = h.get("/", None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), Some("/login"));
    let r = h.get("/scripts/sales_by_region", None).await;
    assert_eq!(r.location(), Some("/login"));
}

#[tokio::test]
async fn login_page_offers_ldap_form_and_oidc_link() {
    let h = harness(Options::default()).await;
    let r = h.get("/login", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("name=\"username\""));
    assert!(r.body.contains("/login/oidc"));
    assert!(r.session_cookie().is_some());
}

#[tokio::test]
async fn ldap_login_lists_only_permitted_scripts() {
    let h = harness(Options::default()).await;
    let cookie = h.login_ldap("alice", "secret").await;

    let r = h.get("/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Sales by region"));
    assert!(r.body.contains("Monthly trend"));
    assert!(r.body.contains("Top products"));
    assert!(
        !r.body.contains("Order sizes"),
        "admin-only script must be hidden"
    );
    assert!(r.body.contains("action=\"/logout\""));
}

#[tokio::test]
async fn forbidden_and_missing_scripts() {
    let h = harness(Options::default()).await;
    let cookie = h.login_ldap("alice", "secret").await;
    assert_eq!(
        h.get("/scripts/order_sizes", Some(&cookie)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.get("/scripts/nope", Some(&cookie)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.get("/no/such/page", None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn wrong_password_is_rejected_generically() {
    let h = harness(Options::default()).await;
    let page = h.get("/login", None).await;
    let cookie = page.session_cookie().unwrap();
    let r = h
        .post_form(
            "/login",
            &format!("username=alice&password=wrong&csrf={}", page.csrf()),
            Some(&cookie),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.body.contains("Invalid username or password."));
    assert_eq!(
        h.get("/", Some(&cookie)).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn login_requires_a_valid_csrf_token() {
    let h = harness(Options::default()).await;
    let page = h.get("/login", None).await;
    let cookie = page.session_cookie().unwrap();
    let r = h
        .post_form(
            "/login",
            "username=alice&password=secret&csrf=forged",
            Some(&cookie),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn login_rotates_the_session_id() {
    let h = harness(Options::default()).await;
    let page = h.get("/login", None).await;
    let before = page.session_cookie().unwrap();
    let after = h.login_ldap("alice", "secret").await;
    assert_ne!(
        before, after,
        "session id must change on login (fixation protection)"
    );
}

#[tokio::test]
async fn logout_ends_the_session() {
    let h = harness(Options::default()).await;
    let cookie = h.login_ldap("alice", "secret").await;
    let home = h.get("/", Some(&cookie)).await;
    let csrf = home.csrf();

    assert_eq!(
        h.post_form("/logout", "csrf=bad", Some(&cookie))
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    let r = h
        .post_form("/logout", &format!("csrf={csrf}"), Some(&cookie))
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), Some("/login"));
    assert_eq!(
        h.get("/", Some(&cookie)).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn oidc_flow_grants_roles_from_groups() {
    let h = harness(Options::default()).await;
    let start = h.get("/login/oidc", None).await;
    assert_eq!(start.status, StatusCode::SEE_OTHER);
    assert!(start
        .location()
        .unwrap()
        .starts_with("https://idp.example.org/authorize"));
    let cookie = start.session_cookie().unwrap();

    let cb = h
        .get(
            "/auth/callback?code=good-code&state=state-123",
            Some(&cookie),
        )
        .await;
    assert_eq!(cb.status, StatusCode::SEE_OTHER);
    assert_eq!(cb.location(), Some("/"));
    let cookie = cb.session_cookie().unwrap();

    // Fake OIDC user is in group "admins" -> admin role -> admin-only script visible.
    let home = h.get("/", Some(&cookie)).await;
    assert!(home.body.contains("Order sizes"));
    assert_eq!(
        h.get("/scripts/order_sizes", Some(&cookie)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn oidc_callback_rejects_state_mismatch_and_missing_session() {
    let h = harness(Options::default()).await;
    let start = h.get("/login/oidc", None).await;
    let cookie = start.session_cookie().unwrap();

    let bad = h
        .get("/auth/callback?code=good-code&state=forged", Some(&cookie))
        .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);

    // The pending state is single-use.
    let replay = h
        .get(
            "/auth/callback?code=good-code&state=state-123",
            Some(&cookie),
        )
        .await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);

    let none = h.get("/auth/callback?code=x&state=y", None).await;
    assert_eq!(none.status, StatusCode::BAD_REQUEST);

    let denied = h.get("/auth/callback?error=access_denied", None).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
}

// ------------------------------------------------------ auth disabled

#[tokio::test]
async fn disabled_auth_serves_everything_without_login() {
    let h = harness(no_auth()).await;
    let home = h.get("/", None).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(
        home.body.contains("Order sizes"),
        "anonymous admin sees all scripts"
    );
    assert!(!home.body.contains("/logout"));
    assert_eq!(
        h.get("/scripts/order_sizes?_run=1&region=EU", None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(h.get("/login", None).await.location(), Some("/"));
}

#[tokio::test]
async fn disabled_auth_rejects_login_attempts() {
    let h = harness(no_auth()).await;
    let r = h.get("/login/oidc", None).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

// ------------------------------------------------------------ scripts

#[tokio::test]
async fn text_search_filters_rows_on_the_server() {
    let h = harness(no_auth()).await;
    let widgets = h
        .get(
            "/scripts/find_sales?_run=1&q=widget&from_date=2024-01-01&to_date=2024-06-30",
            None,
        )
        .await;
    assert_eq!(widgets.status, StatusCode::OK);
    assert!(widgets.body.contains("Widget"));
    assert!(
        !widgets.body.contains("Gizmo"),
        "search must exclude other products"
    );

    // Matching is case-insensitive and also covers the region column.
    let apac = h
        .get(
            "/scripts/find_sales?_run=1&q=apac&from_date=2024-01-01&to_date=2024-06-30",
            None,
        )
        .await;
    assert!(apac.body.contains("APAC") && !apac.body.contains(">EU<"));

    let nothing = h
        .get(
            "/scripts/find_sales?_run=1&q=zzz-no-match&from_date=2024-01-01&to_date=2024-06-30",
            None,
        )
        .await;
    assert!(nothing.body.contains("returned no rows"));
}

#[tokio::test]
async fn script_page_renders_form_table_and_charts() {
    let h = harness(no_auth()).await;
    let r = h
        .get(
            "/scripts/sales_by_region?_run=1&from_date=2024-01-01&to_date=2024-03-31",
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("name=\"from_date\"") && r.body.contains("value=\"2024-03-31\""));
    assert!(r
        .body
        .contains("<th scope=\"col\" tabindex=\"0\" aria-sort=\"none\">total</th>"));
    assert!(
        r.body.contains("data-chart=\"chart-data-0\"")
            && r.body.contains("data-chart=\"chart-data-1\"")
    );
    assert!(r.body.contains("\"type\":\"bar\"") && r.body.contains("\"type\":\"doughnut\""));
    assert!(r.body.contains("https://cdn.example.org/chart.js"));
    assert!(r.body.contains("3 rows in"));
}

#[tokio::test]
async fn scripts_with_defaults_run_on_page_load() {
    let h = harness(no_auth()).await;
    let r = h.get("/scripts/monthly_trend", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("2024-01") && r.body.contains("\"type\":\"line\""));
}

#[tokio::test]
async fn invalid_parameters_return_422_with_field_errors() {
    let h = harness(no_auth()).await;
    let r = h
        .get("/scripts/top_products?_run=1&limit=99&region=MARS", None)
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.body.contains("must be at most 20"));
    assert!(r.body.contains("not an allowed option"));
    assert!(r.body.contains("field invalid"));
}

#[tokio::test]
async fn unchecked_checkbox_means_false_after_submit() {
    let h = harness(no_auth()).await;
    let off = h
        .get("/scripts/top_products?_run=1&region=APAC&limit=5", None)
        .await;
    assert!(off.body.contains("returned no rows"), "{}", off.body);
    let on = h
        .get(
            "/scripts/top_products?_run=1&region=APAC&limit=5&include_apac=true",
            None,
        )
        .await;
    assert!(on.body.contains("<table"));
}

#[tokio::test]
async fn scatter_script_without_table_renders_chart_only() {
    let h = harness(no_auth()).await;
    let r = h.get("/scripts/order_sizes?_run=1&region=EU", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("\"type\":\"scatter\""));
    assert!(!r.body.contains("<table"));
}

#[tokio::test]
async fn html_is_escaped_in_parameter_echo() {
    let h = harness(no_auth()).await;
    let r = h
        .get(
            "/scripts/sales_by_region?_run=1&from_date=%22%3E%3Cscript%3Ealert(1)%3C/script%3E",
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!r.body.contains("<script>alert(1)"));
}

// ---------------------------------------------------------------- API

#[tokio::test]
async fn api_requires_authentication_and_returns_json() {
    let h = harness(Options::default()).await;
    let r = h.get("/api/scripts", None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json()["error"], "authentication required");

    let cookie = h.login_ldap("alice", "secret").await;
    let list = h.get("/api/scripts", Some(&cookie)).await;
    assert_eq!(list.status, StatusCode::OK);
    let names: Vec<_> = list.json()["scripts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "find_sales",
            "gallery_all_types",
            "gallery_quarterly_bars",
            "gallery_brand_share",
            "gallery_sensor_readings",
            "gallery_survey_bands",
            "gallery_web_traffic",
            "monthly_trend",
            "sales_by_region",
            "showcase_table",
            "showcase_chart_error",
            "showcase_params",
            "showcase_column_types",
            "showcase_wide_table",
            "showcase_filter_bar_pie",
            "showcase_filter_monthly_line",
            "showcase_top_n_switch",
            "showcase_scatter_filters",
            "showcase_search_paged",
            "showcase_big_table",
            "showcase_bar",
            "showcase_line",
            "showcase_pie",
            "showcase_doughnut",
            "showcase_scatter",
            "showcase_mixed",
            "showcase_truncated",
            "showcase_empty",
            "top_products",
        ]
    );

    let run = h
        .get("/api/scripts/top_products?limit=2", Some(&cookie))
        .await;
    assert_eq!(run.status, StatusCode::OK);
    let body = run.json();
    assert_eq!(
        body["columns"],
        serde_json::json!(["product", "units", "revenue"])
    );
    assert_eq!(body["rows"].as_array().unwrap().len(), 2);

    assert_eq!(
        h.get("/api/scripts/order_sizes", Some(&cookie))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.get("/api/scripts/zzz", Some(&cookie)).await.status,
        StatusCode::NOT_FOUND
    );
    let bad = h
        .get("/api/scripts/top_products?limit=0", Some(&cookie))
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad.json()["fields"][0]["param"], "limit");
}

// -------------------------------------------------------- operations

#[tokio::test]
async fn health_and_readiness_endpoints() {
    let h = harness(Options::default()).await;
    let live = h.get("/healthz", None).await;
    assert_eq!(live.status, StatusCode::OK);
    assert_eq!(live.json()["status"], "ok");

    let ready = h.get("/readyz", None).await;
    assert_eq!(ready.status, StatusCode::OK);
    assert_eq!(ready.json()["status"], "ready");
    assert_eq!(ready.json()["checks"][0]["name"], "database");
    assert_eq!(ready.headers["cache-control"], "no-store");
}

#[tokio::test]
async fn readiness_fails_on_critical_probe_but_not_on_optional_one() {
    let h = harness(Options {
        extra_probes: vec![failing_probe()],
        ..Options::default()
    })
    .await;
    let r = h.get("/readyz", None).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.json()["status"], "not_ready");
    assert_eq!(h.get("/healthz", None).await.status, StatusCode::OK);

    let h = harness(Options {
        extra_probes: vec![failing_noncritical_probe()],
        ..Options::default()
    })
    .await;
    let r = h.get("/readyz", None).await;
    assert_eq!(r.status, StatusCode::OK);
    // Extra probes are reported after the database and OIDC probes.
    let checks = r.json()["checks"].as_array().unwrap().clone();
    assert_eq!(checks.last().unwrap()["status"], "down");
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text() {
    let h = harness(Options::default()).await;
    let r = h.get("/metrics", None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/plain; version=0.0.4"));
    assert_eq!(r.body, "# test metrics\n");
}

#[tokio::test]
async fn security_headers_and_static_assets() {
    let h = harness(Options::default()).await;
    let r = h.get("/login", None).await;
    let csp = r.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'self' https://cdn.example.org;"));
    assert!(!csp.contains("unsafe-inline"));
    assert_eq!(r.headers["x-frame-options"], "DENY");
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    assert!(r.headers.contains_key("x-request-id"));

    let css = h.get("/static/app.css", None).await;
    assert!(css.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    let js = h.get("/static/app.js", None).await;
    assert!(js.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));
}

#[tokio::test]
async fn session_cookie_is_http_only_and_lax() {
    let h = harness(Options::default()).await;
    let r = h.get("/login", None).await;
    let set_cookie = r
        .headers
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_lowercase();
    assert!(set_cookie.contains("httponly"));
    assert!(set_cookie.contains("samesite=lax"));
}
