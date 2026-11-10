//! Prometheus exposition for the `metrics` facade.
//!
//! Builds a recorder with caller-supplied histogram buckets, installs it as the
//! process-global recorder and renders the text exposition format. Knows
//! nothing about the metrics an application records.

use anyhow::Context;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

pub use metrics_exporter_prometheus::PrometheusRecorder;

/// Latency buckets in seconds, from 5 ms to 30 s.
pub const DEFAULT_LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// Histogram buckets for every metric whose name ends with `suffix`.
#[derive(Debug, Clone, Copy)]
pub struct Buckets<'a> {
    pub suffix: &'a str,
    pub values: &'a [f64],
}

/// Renders the current state of a recorder as Prometheus text.
#[derive(Debug, Clone)]
pub struct Handle(PrometheusHandle);

impl Handle {
    pub fn render(&self) -> String {
        self.0.render()
    }
}

/// Builds a recorder without installing it, for example for
/// `metrics::with_local_recorder` in tests.
pub fn build_recorder(buckets: &[Buckets<'_>]) -> anyhow::Result<(PrometheusRecorder, Handle)> {
    let mut builder = PrometheusBuilder::new();
    for b in buckets {
        builder = builder
            .set_buckets_for_metric(Matcher::Suffix(b.suffix.to_string()), b.values)
            .with_context(|| format!("invalid buckets for '*{}'", b.suffix))?;
    }
    let recorder = builder.build_recorder();
    let handle = Handle(recorder.handle());
    Ok((recorder, handle))
}

/// Installs a recorder as the process-global one. Fails when a recorder is
/// already installed.
pub fn install(buckets: &[Buckets<'_>]) -> anyhow::Result<Handle> {
    let (recorder, handle) = build_recorder(buckets)?;
    metrics::set_global_recorder(recorder)
        .map_err(|e| anyhow::anyhow!("metrics recorder already installed: {e}"))?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histograms_matching_the_suffix_get_buckets() {
        let (recorder, handle) = build_recorder(&[Buckets {
            suffix: "_seconds",
            values: DEFAULT_LATENCY_BUCKETS,
        }])
        .unwrap();
        metrics::with_local_recorder(&recorder, || {
            metrics::histogram!("call_seconds").record(0.2);
            metrics::histogram!("payload_bytes").record(10.0);
        });
        let text = handle.render();
        assert!(
            text.contains(r#"call_seconds_bucket{le="0.25"} 1"#),
            "{text}"
        );
        assert!(
            !text.contains("payload_bytes_bucket"),
            "summary, not histogram"
        );
    }

    #[test]
    fn empty_buckets_are_rejected() {
        let err = build_recorder(&[Buckets {
            suffix: "_seconds",
            values: &[],
        }])
        .unwrap_err();
        assert!(err.to_string().contains("_seconds"));
    }
}
