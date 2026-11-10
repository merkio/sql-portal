//! OpenID Connect relying-party adapter (authorization code flow + PKCE).

use crate::tls::{apply_reqwest, TlsOptions};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreResponseType};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse,
};
use serde_json::Value as Json;
use tokio::sync::OnceCell;

use crate::config::model::OidcConfig;
use sql_portal_domain::{AuthError, AuthProvider, User};
use sql_portal_domain::{OidcAuthorization, OidcPending, OidcProvider};

type DiscoveredClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub struct OidcAdapter {
    cfg: OidcConfig,
    redirect_url: String,
    http: openidconnect::reqwest::Client,
    client: OnceCell<DiscoveredClient>,
}

impl OidcAdapter {
    pub fn new(cfg: OidcConfig, public_url: &str, tls: &TlsOptions) -> anyhow::Result<Self> {
        let redirect_url = format!("{}/auth/callback", public_url.trim_end_matches('/'));
        // Following redirects would open SSRF holes (per the OAuth2 crate docs).
        let builder = openidconnect::reqwest::ClientBuilder::new()
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10));
        let http = apply_reqwest(builder, tls)?.build()?;
        Ok(Self {
            cfg,
            redirect_url,
            http,
            client: OnceCell::new(),
        })
    }

    pub fn redirect_url(&self) -> &str {
        &self.redirect_url
    }

    /// Discovers the provider lazily so that an IdP outage at startup does not
    /// prevent the service from starting.
    async fn client(&self) -> Result<&DiscoveredClient, AuthError> {
        self.client
            .get_or_try_init(|| async {
                let issuer = IssuerUrl::new(self.cfg.issuer_url.clone()).map_err(unavailable)?;
                let metadata = CoreProviderMetadata::discover_async(issuer, &self.http)
                    .await
                    .map_err(unavailable)?;
                let redirect = RedirectUrl::new(self.redirect_url.clone()).map_err(unavailable)?;
                Ok(CoreClient::from_provider_metadata(
                    metadata,
                    ClientId::new(self.cfg.client_id.clone()),
                    self.cfg.client_secret.clone().map(ClientSecret::new),
                )
                .set_redirect_uri(redirect))
            })
            .await
    }
}

#[async_trait]
impl OidcProvider for OidcAdapter {
    async fn begin(&self) -> Result<OidcAuthorization, AuthError> {
        let client = self.client().await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = client
            .authorize_url(
                AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge);
        for scope in &self.cfg.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }
        let (url, state, nonce) = request.url();
        Ok(OidcAuthorization {
            redirect_url: url.to_string(),
            pending: OidcPending {
                state: state.secret().clone(),
                nonce: nonce.secret().clone(),
                pkce_verifier: verifier.secret().clone(),
            },
        })
    }

    async fn complete(&self, code: &str, pending: &OidcPending) -> Result<User, AuthError> {
        let client = self.client().await?;
        let token = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| AuthError::Rejected(e.to_string()))?
            .set_pkce_verifier(PkceCodeVerifier::new(pending.pkce_verifier.clone()))
            .request_async(&self.http)
            .await
            .map_err(|e| AuthError::Rejected(format!("token exchange failed: {e}")))?;

        let id_token = token
            .id_token()
            .ok_or_else(|| AuthError::Rejected("provider returned no ID token".into()))?;

        // Signature, issuer, audience, expiry and nonce verification.
        id_token
            .claims(
                &client.id_token_verifier(),
                &Nonce::new(pending.nonce.clone()),
            )
            .map_err(|e| AuthError::Rejected(format!("ID token verification failed: {e}")))?;

        // The token is now trusted; read the configurable claims from its payload.
        let jwt = serde_json::to_value(id_token)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| AuthError::Rejected("cannot serialise ID token".into()))?;
        let payload =
            jwt_payload(&jwt).ok_or_else(|| AuthError::Rejected("malformed ID token".into()))?;
        user_from_claims(&payload, &self.cfg)
            .ok_or_else(|| AuthError::Rejected("ID token has no subject".into()))
    }

    async fn probe(&self) -> Result<(), String> {
        self.client().await.map(|_| ()).map_err(|e| e.to_string())
    }
}

fn unavailable(e: impl std::fmt::Display) -> AuthError {
    AuthError::ProviderUnavailable(e.to_string())
}

/// Decodes the (already verified) payload segment of a compact JWT.
pub fn jwt_payload(jwt: &str) -> Option<Json> {
    let segment = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(segment).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Maps verified ID-token claims to a domain user.
pub fn user_from_claims(claims: &Json, cfg: &OidcConfig) -> Option<User> {
    let subject = claims.get("sub")?.as_str()?.to_string();
    let string_claim = |name: &str| claims.get(name).and_then(Json::as_str).map(str::to_owned);

    let username = string_claim(&cfg.username_claim).unwrap_or_else(|| subject.clone());
    let display_name = string_claim("name").unwrap_or_else(|| username.clone());
    let groups = match claims.get(&cfg.groups_claim) {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(Json::String(s)) => s
            .split(',')
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty())
            .collect(),
        _ => vec![],
    };
    Some(User {
        // Use the readable username for role mapping; `sub` stays reachable
        // through user_roles by configuring the claim accordingly.
        subject: username,
        display_name,
        email: string_claim("email"),
        groups,
        provider: AuthProvider::Oidc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg() -> OidcConfig {
        serde_yaml_ng::from_str("issuer_url: https://idp.example.org\nclient_id: portal\n").unwrap()
    }

    #[test]
    fn maps_claims_with_group_array() {
        let claims = json!({
            "sub": "123",
            "preferred_username": "alice",
            "name": "Alice L.",
            "email": "alice@example.org",
            "groups": ["analysts", "ops"]
        });
        let u = user_from_claims(&claims, &cfg()).unwrap();
        assert_eq!(u.subject, "alice");
        assert_eq!(u.display_name, "Alice L.");
        assert_eq!(u.email.as_deref(), Some("alice@example.org"));
        assert_eq!(u.groups, vec!["analysts", "ops"]);
        assert_eq!(u.provider, AuthProvider::Oidc);
    }

    #[test]
    fn falls_back_to_sub_and_handles_string_groups() {
        let claims = json!({"sub": "123", "groups": "a, b,,c"});
        let u = user_from_claims(&claims, &cfg()).unwrap();
        assert_eq!(u.subject, "123");
        assert_eq!(u.display_name, "123");
        assert_eq!(u.groups, vec!["a", "b", "c"]);
    }

    #[test]
    fn honours_custom_claim_names() {
        let mut c = cfg();
        c.username_claim = "email".into();
        c.groups_claim = "roles".into();
        let claims = json!({"sub": "1", "email": "a@b.c", "roles": ["x"]});
        let u = user_from_claims(&claims, &c).unwrap();
        assert_eq!(u.subject, "a@b.c");
        assert_eq!(u.groups, vec!["x"]);
    }

    #[test]
    fn requires_subject() {
        assert!(user_from_claims(&json!({"name": "x"}), &cfg()).is_none());
    }

    #[test]
    fn decodes_jwt_payload() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"abc"}"#);
        let jwt = format!("header.{payload}.sig");
        assert_eq!(jwt_payload(&jwt).unwrap()["sub"], "abc");
        assert!(jwt_payload("no-dots").is_none());
        assert!(jwt_payload("a.!!!.c").is_none());
    }

    #[test]
    fn redirect_url_is_built_from_public_url() {
        let a =
            OidcAdapter::new(cfg(), "https://portal.example.org/", &TlsOptions::default()).unwrap();
        assert_eq!(a.redirect_url(), "https://portal.example.org/auth/callback");
    }

    #[tokio::test]
    async fn unreachable_issuer_is_unavailable_not_a_panic() {
        let mut c = cfg();
        c.issuer_url = "http://127.0.0.1:1".into();
        let a = OidcAdapter::new(c, "http://localhost", &TlsOptions::default()).unwrap();
        assert!(matches!(
            a.begin().await,
            Err(AuthError::ProviderUnavailable(_))
        ));
        assert!(a.probe().await.is_err());
    }
}
