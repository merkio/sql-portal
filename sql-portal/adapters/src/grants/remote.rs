//! HTTP client for the external grants service.

use std::fmt::Display;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{redirect, Client, StatusCode};
use serde::{Deserialize, Serialize};

use crate::tls::{apply_reqwest, TlsOptions};
use sql_portal_domain::GrantSource;
use sql_portal_domain::{AuthError, Grants, PermissionSet, User};

/// Loads roles and permissions from `POST {url}`. Uses the same TLS settings as
/// the other connections: a CA, a client identity for mutual TLS, or `insecure`.
pub struct RemoteGrants {
    client: Client,
    url: String,
    bearer_token: Option<String>,
}

#[derive(Serialize)]
struct LookupRequest<'a> {
    subject: &'a str,
    display_name: &'a str,
    email: Option<&'a str>,
    groups: &'a [String],
}

#[derive(Deserialize)]
struct LookupResponse {
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
}

impl RemoteGrants {
    pub fn new(
        url: &str,
        bearer_token: Option<String>,
        timeout: Duration,
        tls: &TlsOptions,
    ) -> anyhow::Result<Self> {
        // Redirects could send credentials to another host.
        let builder = Client::builder()
            .timeout(timeout)
            .redirect(redirect::Policy::none());
        Ok(Self {
            client: apply_reqwest(builder, tls)?.build()?,
            url: url.to_string(),
            bearer_token,
        })
    }
}

#[async_trait]
impl GrantSource for RemoteGrants {
    async fn grants_for(&self, user: &User) -> Result<Grants, AuthError> {
        let mut request = self.client.post(&self.url).json(&LookupRequest {
            subject: &user.subject,
            display_name: &user.display_name,
            email: user.email.as_deref(),
            groups: &user.groups,
        });
        if let Some(token) = &self.bearer_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(unavailable)?;

        match response.status() {
            // An identity the service does not know holds no grants.
            StatusCode::NOT_FOUND => return Ok(Grants::default()),
            status if status.is_success() => {}
            status => {
                return Err(AuthError::ProviderUnavailable(format!(
                    "grants service answered {status}"
                )))
            }
        }
        let body: LookupResponse = response.json().await.map_err(unavailable)?;
        Ok(Grants {
            roles: body.roles.into_iter().collect(),
            permissions: PermissionSet::new(body.permissions),
        })
    }
}

fn unavailable(e: impl Display) -> AuthError {
    AuthError::ProviderUnavailable(e.to_string())
}
