//! In-process harness: the production router over the demo configuration,
//! with fake identity providers and an in-memory SQLite database.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, Response, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use sql_portal::{http::router, wire, Overrides};
use sql_portal_adapters::config::load_from_path;
use sql_portal_adapters::sql::SqlExecutor;
use sql_portal_application::{AuthMode, LocalGrants, Providers};
use sql_portal_domain::telemetry::NoopTelemetry;
use sql_portal_domain::{AuthProvider, User};
use sql_portal_domain::{GithubLogin, HealthProbe, QueryExecutor};
use sql_portal_testing::{FakeGithub, FakeLdap, FakeOidc, FakeProbe};
use tower::ServiceExt;

pub struct Harness {
    pub app: Router,
}

pub struct Options {
    pub auth_enabled: bool,
    pub extra_probes: Vec<Arc<dyn HealthProbe>>,
    /// Offer "Sign in with GitHub" (with a fake GitHub that accepts `good-code`).
    pub github: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auth_enabled: true,
            extra_probes: vec![],
            github: false,
        }
    }
}

/// Builds the production router over the demo configuration, through the same
/// wiring the server uses. Only the database connection, the telemetry sink and
/// the identity providers are replaced, and the UI settings are pinned so the
/// tests can check them.
pub async fn harness(options: Options) -> Harness {
    let cfg = load_from_path(&Path::new(crate::REPO_ROOT).join("config/config.demo.yaml"))
        .expect("demo config loads");
    let mut rt = cfg.into_runtime().expect("demo config is valid");
    rt.ui.title = "Test Portal".into();
    rt.ui.chartjs_url = "https://cdn.example.org/chart.js".into();

    let executor: Arc<dyn QueryExecutor> = Arc::new(
        SqlExecutor::connect("sqlite::memory:", 1, Duration::from_secs(5))
            .await
            .unwrap(),
    );
    let auth = if options.auth_enabled {
        AuthMode::Enabled(Providers {
            ldap: Some(Arc::new(
                FakeLdap::accepting("alice", "secret").with_groups(&["analysts"]),
            )),
            oidc: Some(Arc::new(FakeOidc {
                groups: vec!["admins".into()],
            })),
            github: options.github.then(|| {
                Arc::new(FakeGithub {
                    user: User {
                        subject: "octocat".into(),
                        display_name: "The Octocat".into(),
                        email: None,
                        groups: vec!["acme".into(), "acme/analysts".into()],
                        provider: AuthProvider::Github,
                    },
                }) as Arc<dyn GithubLogin>
            }),
        })
    } else {
        AuthMode::Disabled
    };
    let services = wire(
        &rt,
        Overrides {
            executor: Some(executor),
            auth: Some(auth),
            grants: Some(Arc::new(LocalGrants::new(rt.policy.clone()))),
            telemetry: Some(Arc::new(NoopTelemetry)),
            extra_probes: options.extra_probes,
            ..Default::default()
        },
    )
    .await
    .expect("harness wiring");
    Harness {
        app: router(services, Arc::new(|| "# test metrics\n".to_string())),
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: header::HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn location(&self) -> Option<&str> {
        self.headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
    }

    /// `name=value` of the session cookie set by this response, if any.
    pub fn session_cookie(&self) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|c| c.starts_with("sp_session="))
            .map(|c| c.split(';').next().unwrap().to_string())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("json body")
    }

    /// Extracts the hidden CSRF token from a rendered form.
    pub fn csrf(&self) -> String {
        let marker = "name=\"csrf\" value=\"";
        let start = self.body.find(marker).expect("csrf field") + marker.len();
        let end = self.body[start..].find('"').unwrap() + start;
        self.body[start..end].to_string()
    }
}

impl Harness {
    pub async fn send(&self, req: Request<Body>) -> Reply {
        let res: Response<Body> = self.app.clone().oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    pub async fn get(&self, uri: &str, cookie: Option<&str>) -> Reply {
        let mut b = Request::builder().uri(uri);
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        self.send(b.body(Body::empty()).unwrap()).await
    }

    pub async fn post_form(&self, uri: &str, form: &str, cookie: Option<&str>) -> Reply {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        self.send(b.body(Body::from(form.to_string())).unwrap())
            .await
    }

    /// Logs in through the LDAP form and returns the authenticated session cookie.
    pub async fn login_ldap(&self, user: &str, password: &str) -> String {
        let page = self.get("/login", None).await;
        let cookie = page.session_cookie().expect("session cookie on login page");
        let csrf = page.csrf();
        let reply = self
            .post_form(
                "/login",
                &format!("username={user}&password={password}&csrf={csrf}"),
                Some(&cookie),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::SEE_OTHER,
            "login failed: {}",
            reply.body
        );
        reply.session_cookie().expect("cycled session cookie")
    }
}

pub fn failing_probe() -> Arc<dyn HealthProbe> {
    Arc::new(FakeProbe::new("broken", false, true))
}

pub fn failing_noncritical_probe() -> Arc<dyn HealthProbe> {
    Arc::new(FakeProbe::new("flaky", false, false))
}
