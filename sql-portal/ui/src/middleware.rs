use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use metrics::{counter, gauge, histogram};

/// Records request count, latency and in-flight gauge using the *matched
/// route* (not the raw path) as label, which keeps cardinality bounded.
pub async fn track_metrics(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let method = req.method().as_str().to_owned();

    gauge!("http_requests_in_flight").increment(1.0);
    let response = next.run(req).await;
    gauge!("http_requests_in_flight").decrement(1.0);

    let status = response.status().as_u16().to_string();
    let labels = [("method", method), ("path", path), ("status", status)];
    counter!("http_requests_total", &labels).increment(1);
    histogram!("http_request_duration_seconds", &labels).record(start.elapsed().as_secs_f64());
    response
}

/// Adds defensive security headers, including a CSP that only allows
/// same-origin scripts plus the configured chart library origin.
pub async fn security_headers(
    State(csp): State<HeaderValue>,
    req: Request,
    next: Next,
) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    response
}

/// Builds the Content-Security-Policy value for the given chart library URL.
pub fn content_security_policy(chartjs_url: &str) -> HeaderValue {
    let origin = url::Url::parse(chartjs_url)
        .ok()
        .map(|u| u.origin().ascii_serialization())
        .filter(|o| o != "null")
        .unwrap_or_default();
    let script_src = if origin.is_empty() {
        "'self'".to_string()
    } else {
        format!("'self' {origin}")
    };
    let policy = format!(
        "default-src 'self'; script-src {script_src}; style-src 'self'; img-src 'self' data:; \
         connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
    );
    HeaderValue::from_str(&policy).expect("valid header value")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_allows_only_the_chart_origin() {
        let csp = content_security_policy(
            "https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js",
        );
        let s = csp.to_str().unwrap();
        assert!(s.contains("script-src 'self' https://cdn.jsdelivr.net;"));
        assert!(!s.contains("unsafe-inline"));
        assert!(s.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn csp_for_self_hosted_library() {
        let csp = content_security_policy("/static/chart.js");
        assert!(csp.to_str().unwrap().contains("script-src 'self';"));
    }
}
