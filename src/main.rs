use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use script_portal::bootstrap::build_app;
use script_portal::infrastructure::config::load_from_path;
use script_portal::infrastructure::telemetry::{
    describe_and_init, init_tracing, install_prometheus,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::info;

#[derive(Parser)]
#[command(
    name = "script-portal",
    version,
    about = "Config-driven SQL script portal"
)]
struct Cli {
    /// Path to the YAML configuration file.
    #[arg(
        short,
        long,
        env = "SCRIPT_PORTAL_CONFIG",
        default_value = "config/config.yaml"
    )]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server (default).
    Serve,
    /// Validate the configuration and exit.
    CheckConfig,
    /// Probe a running instance (used by the Docker HEALTHCHECK).
    Healthcheck {
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: String,
        #[arg(long, default_value = "/readyz")]
        path: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Healthcheck { addr, path } => healthcheck(&addr, &path).await,
        Command::CheckConfig => {
            let cfg = load_from_path(&cli.config)?;
            cfg.validate()?;
            println!("configuration OK ({} scripts)", cfg.app.scripts.len());
            Ok(())
        }
        Command::Serve => serve(&cli.config).await,
    }
}

async fn serve(config_path: &Path) -> anyhow::Result<()> {
    let cfg = load_from_path(config_path)
        .with_context(|| format!("loading {}", config_path.display()))?;
    init_tracing(
        &cfg.app.observability.log_level,
        cfg.app.observability.log_format,
    );

    let handle = install_prometheus()?;
    describe_and_init(env!("CARGO_PKG_VERSION"), cfg.app.auth.enabled);
    let render: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(move || handle.render());

    let runtime = cfg.into_runtime()?;
    let router = build_app(&runtime, render).await?;

    let bind = std::env::var("SCRIPT_PORTAL_BIND").unwrap_or_else(|_| cfg.app.server.bind.clone());
    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("invalid bind address '{bind}'"))?;
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, version = env!("CARGO_PKG_VERSION"), "listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("shut down cleanly");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}

/// Minimal HTTP/1.0 probe so the distroless image needs no curl.
async fn healthcheck(addr: &str, path: &str) -> anyhow::Result<()> {
    let probe = async {
        let mut stream = TcpStream::connect(addr).await?;
        let request = format!("GET {path} HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        let head = String::from_utf8_lossy(&buf);
        let status_line = head.lines().next().unwrap_or_default().to_string();
        anyhow::ensure!(
            status_line.split_whitespace().nth(1) == Some("200"),
            "unhealthy: {status_line}"
        );
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(3), probe)
        .await
        .context("health check timed out")?
}
