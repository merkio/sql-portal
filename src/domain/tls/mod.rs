//! TLS configuration shared across all outbound connections.

mod tls;

pub use tls::{apply_reqwest, rustls_client_config, TlsOptions};
