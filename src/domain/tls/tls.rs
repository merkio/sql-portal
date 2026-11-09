//! TLS settings shared by every outbound connection (database, LDAP, OIDC).
//!
//! The same files work for a Kubernetes `kubernetes.io/tls` secret mounted into
//! the pod: `tls.crt` and `tls.key` as the client identity, and the CA bundle
//! (for example cert-manager's `ca.crt`) as `ca_file`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

/// Resolved TLS settings for one connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsOptions {
    /// Certificate(s) to trust: a CA, or the server's own self-signed certificate.
    pub ca_file: Option<PathBuf>,
    /// Skip certificate verification entirely. Development only.
    pub insecure: bool,
    /// Client certificate for mutual TLS (PEM).
    pub cert_file: Option<PathBuf>,
    /// Private key for `cert_file` (PEM).
    pub key_file: Option<PathBuf>,
}

impl TlsOptions {
    pub fn is_set(&self) -> bool {
        self.ca_file.is_some() || self.insecure || self.cert_file.is_some()
    }

    /// Checks the combinations that cannot work, and that referenced files exist.
    pub fn check(&self) -> anyhow::Result<()> {
        if self.insecure && self.ca_file.is_some() {
            anyhow::bail!("ca_file and insecure cannot be combined");
        }
        if self.cert_file.is_some() != self.key_file.is_some() {
            anyhow::bail!("cert_file and key_file must be set together");
        }
        if self.cert_file.is_some() && self.ca_file.is_none() && !self.insecure {
            anyhow::bail!("a client certificate needs ca_file (or insecure) to verify the server");
        }
        for (name, path) in [
            ("ca_file", &self.ca_file),
            ("cert_file", &self.cert_file),
            ("key_file", &self.key_file),
        ] {
            if let Some(path) = path {
                if !path.is_file() {
                    anyhow::bail!("{name} not found: {}", path.display());
                }
            }
        }
        Ok(())
    }
}

/// Applies the options to a reqwest client (ClickHouse, OIDC).
pub fn apply_reqwest(
    mut builder: reqwest::ClientBuilder,
    tls: &TlsOptions,
) -> anyhow::Result<reqwest::ClientBuilder> {
    if let Some(ca) = &tls.ca_file {
        let pem =
            std::fs::read(ca).with_context(|| format!("cannot read CA file {}", ca.display()))?;
        for cert in reqwest::Certificate::from_pem_bundle(&pem)
            .with_context(|| format!("invalid CA file {}", ca.display()))?
        {
            builder = builder.add_root_certificate(cert);
        }
    }
    if let (Some(cert), Some(key)) = (&tls.cert_file, &tls.key_file) {
        // reqwest wants the certificate and key in one PEM buffer.
        let mut pem = std::fs::read(cert)
            .with_context(|| format!("cannot read client certificate {}", cert.display()))?;
        pem.push(b'\n');
        pem.extend(
            std::fs::read(key)
                .with_context(|| format!("cannot read client key {}", key.display()))?,
        );
        builder = builder.identity(
            reqwest::Identity::from_pem(&pem)
                .map_err(|e| anyhow!("invalid client identity: {e}"))?,
        );
    }
    if tls.insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }
    Ok(builder)
}

/// rustls configuration for LDAP over TLS (`ldaps://` or StartTLS).
pub fn rustls_client_config(tls: &TlsOptions) -> anyhow::Result<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?;

    let verified = if tls.insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate::new(provider)))
    } else {
        let mut roots = RootCertStore::empty();
        if let Some(ca) = &tls.ca_file {
            for cert in CertificateDer::pem_file_iter(ca)
                .with_context(|| format!("cannot read CA file {}", ca.display()))?
            {
                roots.add(cert.with_context(|| format!("invalid CA file {}", ca.display()))?)?;
            }
        }
        builder.with_root_certificates(roots)
    };

    let config = match (&tls.cert_file, &tls.key_file) {
        (Some(cert), Some(key)) => {
            let chain = CertificateDer::pem_file_iter(cert)
                .with_context(|| format!("cannot read client certificate {}", cert.display()))?
                .collect::<Result<Vec<_>, _>>()
                .with_context(|| format!("invalid client certificate {}", cert.display()))?;
            let key = PrivateKeyDer::from_pem_file(key)
                .with_context(|| format!("invalid client key {}", key.display()))?;
            verified.with_client_auth_cert(chain, key)?
        }
        _ => verified.with_no_client_auth(),
    };
    Ok(Arc::new(config))
}

/// Accepts any server certificate. Used only when `insecure` is set.
#[derive(Debug)]
struct AcceptAnyCertificate {
    provider: Arc<CryptoProvider>,
}

impl AcceptAnyCertificate {
    fn new(provider: Arc<CryptoProvider>) -> Self {
        Self { provider }
    }
}

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rejects_conflicting_options() {
        let both = TlsOptions {
            ca_file: Some(PathBuf::from("ca.pem")),
            insecure: true,
            ..Default::default()
        };
        assert!(both
            .check()
            .unwrap_err()
            .to_string()
            .contains("cannot be combined"));

        let half_identity = TlsOptions {
            ca_file: Some(PathBuf::from("ca.pem")),
            cert_file: Some(PathBuf::from("tls.crt")),
            ..Default::default()
        };
        assert!(half_identity
            .check()
            .unwrap_err()
            .to_string()
            .contains("set together"));

        let no_verification_target = TlsOptions {
            cert_file: Some(PathBuf::from("tls.crt")),
            key_file: Some(PathBuf::from("tls.key")),
            ..Default::default()
        };
        assert!(no_verification_target
            .check()
            .unwrap_err()
            .to_string()
            .contains("needs ca_file"));
    }

    #[test]
    fn check_reports_missing_files() {
        let missing = TlsOptions {
            ca_file: Some(PathBuf::from("/no/such/ca.pem")),
            ..Default::default()
        };
        assert!(missing
            .check()
            .unwrap_err()
            .to_string()
            .contains("ca_file not found"));
    }

    #[test]
    fn insecure_config_builds_without_files() {
        let tls = TlsOptions {
            insecure: true,
            ..Default::default()
        };
        tls.check().unwrap();
        rustls_client_config(&tls).unwrap();
    }
}
