//! Running the router on a listener.

use std::future::Future;

use axum::Router;
use tokio::net::TcpListener;

/// Serves `router` on `listener` until `shutdown` completes, then drains
/// in-flight requests.
pub async fn serve<F>(listener: TcpListener, router: Router, shutdown: F) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
}
