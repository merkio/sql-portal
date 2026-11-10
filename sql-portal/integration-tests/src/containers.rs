//! Docker fixtures for the container tests (`--features docker`).
//!
//! # Lifecycle
//!
//! Every container is stopped and removed when its owner is dropped, which
//! happens at the end of the test, also when the test panics:
//!
//! * [`Portal`] (the portal's own image) belongs to a single test.
//! * Database servers are expensive, so tests in one binary share them through
//!   [`SharedContainer`]: each test holds an `Arc<`[`Lease`]`>`, and the
//!   container is removed when the last lease is dropped.
//!
//! Shared containers are started and removed on a dedicated runtime. Each
//! `#[tokio::test]` has its own runtime, and the Docker client of a container
//! is bound to the runtime that created it, so a container started inside one
//! test could not be used or cleaned up after that test's runtime is gone.
//! Containers are never kept in a `static` directly: statics are not dropped,
//! so the container would outlive the test process.
//!
//! All containers carry the label [`LABEL`]. If a run is killed (e.g. SIGKILL)
//! before cleanup, `make test-docker-clean` removes leftovers.
//!
//! # Images
//!
//! Database images are pinned ([`POSTGRES_TAG`], [`CLICKHOUSE_TAG`]). The
//! portal image is taken from `SCRIPT_PORTAL_IMAGE` (default
//! [`DEFAULT_PORTAL_IMAGE`]) and is never pulled: build it first with
//! `make image-test`.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use anyhow::Context;
use testcontainers_modules::clickhouse::ClickHouse;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::core::{ContainerPort, ExecCommand, WaitFor};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{
    ContainerAsync, ContainerRequest, GenericImage, ImageExt,
};
use tokio::runtime::Runtime;

/// Label put on every container started by these tests.
pub const LABEL: (&str, &str) = ("org.sql-portal.integration-test", "true");
pub const POSTGRES_TAG: &str = "16-alpine";
pub const CLICKHOUSE_TAG: &str = "24.8-alpine";
pub const CLICKHOUSE_PASSWORD: &str = "secret";
pub const DEFAULT_PORTAL_IMAGE: &str = "script-portal:test";
const PORTAL_PORT: u16 = 8080;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------- shared

/// The runtime that owns shared containers (see the module docs).
fn fixture_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("container-fixtures")
            .enable_all()
            .build()
            .expect("fixture runtime starts")
    })
}

/// A running shared container and the URL to reach it. Dropping the last
/// `Arc<Lease>` stops and removes the container and waits until it is gone.
pub struct Lease {
    pub url: String,
    container: Option<Box<dyn Any + Send + Sync>>,
}

impl Lease {
    fn new<I: testcontainers_modules::testcontainers::Image + 'static>(
        url: String,
        container: ContainerAsync<I>,
    ) -> Self {
        Self {
            url,
            container: Some(Box::new(container)),
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(container) = self.container.take() else {
            return;
        };
        let (done, wait) = std::sync::mpsc::channel();
        fixture_runtime().spawn(async move {
            drop(container); // ContainerAsync::drop removes the container
            let _ = done.send(());
        });
        // Block until removal finished, so the test process cannot exit first.
        let _ = wait.recv_timeout(Duration::from_secs(60));
    }
}

type StartFn = fn() -> Pin<Box<dyn Future<Output = anyhow::Result<Lease>> + Send>>;

/// A container shared by the tests of one binary while at least one of them
/// holds a lease. Declare it as a `static`; only a weak reference is kept.
pub struct SharedContainer {
    slot: tokio::sync::Mutex<Weak<Lease>>,
    start: StartFn,
}

impl SharedContainer {
    pub const fn new(start: StartFn) -> Self {
        Self {
            slot: tokio::sync::Mutex::const_new(Weak::new()),
            start,
        }
    }

    /// The running container, started on first use (or again after every
    /// previous lease was dropped).
    pub async fn lease(&self) -> Arc<Lease> {
        let mut slot = self.slot.lock().await;
        if let Some(lease) = slot.upgrade() {
            return lease;
        }
        let lease = fixture_runtime()
            .spawn((self.start)())
            .await
            .expect("container start task panicked")
            .unwrap_or_else(|e| panic!("cannot start container: {e:#}"));
        let lease = Arc::new(lease);
        *slot = Arc::downgrade(&lease);
        lease
    }
}

// ---------------------------------------------------------------- databases

/// PostgreSQL ([`POSTGRES_TAG`]), user/password/database `postgres`.
pub fn postgres_image() -> ContainerRequest<Postgres> {
    Postgres::default()
        .with_tag(POSTGRES_TAG)
        .with_label(LABEL.0, LABEL.1)
        .with_startup_timeout(STARTUP_TIMEOUT)
}

/// ClickHouse ([`CLICKHOUSE_TAG`]), user `default` with [`CLICKHOUSE_PASSWORD`].
pub fn clickhouse_image() -> ContainerRequest<ClickHouse> {
    ClickHouse::default()
        .with_tag(CLICKHOUSE_TAG)
        .with_env_var("CLICKHOUSE_PASSWORD", CLICKHOUSE_PASSWORD)
        .with_label(LABEL.0, LABEL.1)
        .with_startup_timeout(STARTUP_TIMEOUT)
}

/// Shared PostgreSQL; [`Lease::url`] is a `postgres://` URL.
pub static POSTGRES: SharedContainer = SharedContainer::new(|| {
    Box::pin(async {
        let container = postgres_image()
            .start()
            .await
            .context("starting PostgreSQL")?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(5432).await?;
        Ok(Lease::new(
            format!("postgres://postgres:postgres@{host}:{port}/postgres"),
            container,
        ))
    })
});

/// Shared ClickHouse; [`Lease::url`] is a `clickhouse://` URL (HTTP interface).
pub static CLICKHOUSE: SharedContainer = SharedContainer::new(|| {
    Box::pin(async {
        let container = clickhouse_image()
            .start()
            .await
            .context("starting ClickHouse")?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(8123).await?;
        Ok(Lease::new(
            format!("clickhouse://default:{CLICKHOUSE_PASSWORD}@{host}:{port}/default"),
            container,
        ))
    })
});

// ---------------------------------------------------------------- portal image

/// The portal image under test: `SCRIPT_PORTAL_IMAGE` or [`DEFAULT_PORTAL_IMAGE`].
pub fn portal_image_ref() -> (String, String) {
    let reference =
        std::env::var("SCRIPT_PORTAL_IMAGE").unwrap_or_else(|_| DEFAULT_PORTAL_IMAGE.into());
    match reference.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => (name.into(), tag.into()),
        _ => (reference, "latest".into()),
    }
}

/// How to start the portal container.
#[derive(Default)]
pub struct PortalOptions {
    /// YAML written to `/app/config/test.yaml` and used instead of the
    /// image's demo configuration.
    pub config: Option<String>,
    pub env: Vec<(String, String)>,
    /// Docker network to join (to reach other containers by name).
    pub network: Option<String>,
}

/// A running portal container, removed when dropped.
pub struct Portal {
    container: ContainerAsync<GenericImage>,
    pub base_url: String,
    http: reqwest::Client,
}

impl Portal {
    /// Starts the image and waits until `/readyz` answers 200.
    pub async fn start(options: PortalOptions) -> Portal {
        let (name, tag) = portal_image_ref();
        let mut request = GenericImage::new(&name, &tag)
            .with_exposed_port(ContainerPort::Tcp(PORTAL_PORT))
            .with_wait_for(WaitFor::message_on_either_std("listening"))
            .with_label(LABEL.0, LABEL.1)
            .with_env_var("RUST_LOG", "info")
            .with_startup_timeout(STARTUP_TIMEOUT);
        if let Some(yaml) = options.config {
            request = request
                .with_copy_to("/app/config/test.yaml", yaml.into_bytes())
                .with_env_var("SCRIPT_PORTAL_CONFIG", "/app/config/test.yaml");
        }
        for (key, value) in options.env {
            request = request.with_env_var(key, value);
        }
        if let Some(network) = options.network {
            request = request.with_network(network);
        }
        let container = request.start().await.unwrap_or_else(|e| {
            panic!(
                "cannot start the portal image {name}:{tag}: {e:#}\n\
                 Build it first with `make image-test` or set SCRIPT_PORTAL_IMAGE."
            )
        });
        let host = container.get_host().await.expect("container host");
        let port = container
            .get_host_port_ipv4(PORTAL_PORT)
            .await
            .expect("mapped portal port");
        let portal = Portal {
            container,
            base_url: format!("http://{host}:{port}"),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client"),
        };
        portal.wait_until_ready().await;
        portal
    }

    async fn wait_until_ready(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(response) = self.get("/readyz").await {
                if response.status().is_success() {
                    return;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "portal did not become ready; logs:\n{}",
                self.logs().await
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    pub async fn get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        self.http
            .get(format!("{}{path}", self.base_url))
            .send()
            .await
    }

    /// Runs a command inside the container; returns its exit code and output.
    pub async fn exec(&self, cmd: &[&str]) -> (Option<i64>, String) {
        let mut result = self
            .container
            .exec(ExecCommand::new(cmd.iter().copied()))
            .await
            .expect("exec in portal container");
        let mut output =
            String::from_utf8_lossy(&result.stdout_to_vec().await.unwrap_or_default()).into_owned();
        output.push_str(&String::from_utf8_lossy(
            &result.stderr_to_vec().await.unwrap_or_default(),
        ));
        (result.exit_code().await.ok().flatten(), output)
    }

    /// Container stdout and stderr, for failure messages.
    pub async fn logs(&self) -> String {
        let out = self.container.stdout_to_vec().await.unwrap_or_default();
        let err = self.container.stderr_to_vec().await.unwrap_or_default();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        )
    }
}

/// A unique name, for networks and containers of one test.
pub fn unique_name(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}
