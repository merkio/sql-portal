//! Live TLS tests against servers that use a self-signed certificate. They are
//! ignored by default. Environment:
//!
//! - `TLS_PG_URL`: `postgres://user:pass@host:port/db` with `ssl=on`
//! - `TLS_CH_URL`: `clickhouses://user:pass@host:port/db` (HTTPS)
//! - `TLS_CA_FILE`: PEM file with the server's self-signed certificate
//!
//! ```sh
//! cargo test --test tls -- --ignored
//! ```

use std::path::PathBuf;
use std::time::Duration;

use sql_portal_adapters::sql::connect_database;
use sql_portal_adapters::tls::TlsOptions;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} to run the TLS tests"))
}

fn trusted() -> TlsOptions {
    TlsOptions {
        ca_file: Some(PathBuf::from(env("TLS_CA_FILE"))),
        ..Default::default()
    }
}

fn insecure() -> TlsOptions {
    TlsOptions {
        insecure: true,
        ..Default::default()
    }
}

async fn ping(url: &str, tls: &TlsOptions) -> anyhow::Result<()> {
    let db = connect_database(url, 1, Duration::from_secs(5), tls).await?;
    db.ping().await.map_err(|e| anyhow::anyhow!("{e}"))
}

#[tokio::test]
#[ignore = "needs TLS_PG_URL and TLS_CA_FILE"]
async fn postgres_trusts_the_self_signed_certificate() {
    ping(&env("TLS_PG_URL"), &trusted()).await.unwrap();
}

#[tokio::test]
#[ignore = "needs TLS_PG_URL"]
async fn postgres_insecure_mode_accepts_any_certificate() {
    ping(&env("TLS_PG_URL"), &insecure()).await.unwrap();
}

#[tokio::test]
#[ignore = "needs TLS_PG_URL"]
async fn postgres_rejects_unknown_certificate_when_verification_is_required() {
    // Without a configured CA, full verification must fail.
    let url = format!("{}?sslmode=verify-full", env("TLS_PG_URL"));
    assert!(ping(&url, &TlsOptions::default()).await.is_err());
}

#[tokio::test]
#[ignore = "needs TLS_CH_URL and TLS_CA_FILE"]
async fn clickhouse_trusts_the_self_signed_certificate() {
    ping(&env("TLS_CH_URL"), &trusted()).await.unwrap();
}

#[tokio::test]
#[ignore = "needs TLS_CH_URL"]
async fn clickhouse_insecure_mode_accepts_any_certificate() {
    ping(&env("TLS_CH_URL"), &insecure()).await.unwrap();
}

#[tokio::test]
#[ignore = "needs TLS_CH_URL"]
async fn clickhouse_rejects_unknown_certificate_by_default() {
    assert!(ping(&env("TLS_CH_URL"), &TlsOptions::default())
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "needs TLS_CH_URL and TLS_CA_FILE"]
async fn tls_options_need_a_secure_clickhouse_url() {
    let plain = env("TLS_CH_URL").replacen("clickhouses://", "clickhouse://", 1);
    assert!(
        connect_database(&plain, 1, Duration::from_secs(5), &trusted())
            .await
            .is_err()
    );
}

// --- LDAP over LDAPS -----------------------------------------------------------

fn ldap_config(url: &str) -> sql_portal_adapters::config::model::LdapConfig {
    serde_yaml_ng::from_str(&format!(
        "url: {url}\nbind_dn: cn=admin,dc=example,dc=org\nbind_password: adminpass\nbase_dn: dc=example,dc=org\n"
    ))
    .expect("valid LDAP config")
}

#[tokio::test]
#[ignore = "needs TLS_LDAP_URL and TLS_CA_FILE"]
async fn ldap_authenticates_over_ldaps_with_the_shared_ca() {
    use sql_portal_adapters::auth::LdapAuthenticator;
    use sql_portal_domain::CredentialAuthenticator;
    let auth = LdapAuthenticator::new(ldap_config(&env("TLS_LDAP_URL")), &trusted()).unwrap();
    let user = auth.authenticate("alice", "alicepw").await.unwrap();
    assert_eq!(user.subject, "alice");
}

#[tokio::test]
#[ignore = "needs TLS_LDAP_URL"]
async fn ldap_rejects_an_unknown_certificate_by_default() {
    use sql_portal_adapters::auth::LdapAuthenticator;
    use sql_portal_domain::CredentialAuthenticator;
    let auth =
        LdapAuthenticator::new(ldap_config(&env("TLS_LDAP_URL")), &TlsOptions::default()).unwrap();
    assert!(auth.authenticate("alice", "alicepw").await.is_err());
}

// --- mutual TLS -------------------------------------------------------------------

fn client_identity() -> TlsOptions {
    TlsOptions {
        ca_file: Some(PathBuf::from(env("TLS_CA_FILE"))),
        cert_file: Some(PathBuf::from(env("TLS_CLIENT_CERT"))),
        key_file: Some(PathBuf::from(env("TLS_CLIENT_KEY"))),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "needs TLS_PG_MTLS_URL, TLS_CA_FILE, TLS_CLIENT_CERT, TLS_CLIENT_KEY"]
async fn postgres_authenticates_with_the_client_certificate() {
    ping(&env("TLS_PG_MTLS_URL"), &client_identity())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs TLS_PG_MTLS_URL and TLS_CA_FILE"]
async fn postgres_refuses_a_connection_without_the_client_certificate() {
    let trust_only = TlsOptions {
        ca_file: Some(PathBuf::from(env("TLS_CA_FILE"))),
        ..Default::default()
    };
    assert!(ping(&env("TLS_PG_MTLS_URL"), &trust_only).await.is_err());
}
