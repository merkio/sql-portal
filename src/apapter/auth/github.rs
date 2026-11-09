//! GitHub OAuth 2.0 sign-in: authorization URL, code exchange, and the user
//! with the organisations and teams they belong to, as access groups.
//!
//! Groups are `org` for an organisation and `org/team-slug` for a team, so the
//! access configuration can grant roles per organisation or per team.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{redirect, Client, StatusCode};
use serde::Deserialize;

use crate::domain::ports::GithubLogin;
use crate::domain::{AuthError, AuthProvider, User};
use crate::infrastructure::config::model::GithubConfig;

const SCOPES: &str = "read:user user:email read:org";

pub struct GithubAdapter {
    http: Client,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    web_url: String,
    api_url: String,
    allowed_orgs: Vec<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ApiUser {
    login: String,
    name: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
struct ApiEmail {
    email: String,
    primary: bool,
    verified: bool,
}

#[derive(Deserialize)]
struct ApiOrg {
    login: String,
}

#[derive(Deserialize)]
struct ApiTeam {
    slug: String,
    organization: ApiOrg,
}

impl GithubAdapter {
    pub fn new(cfg: &GithubConfig, public_url: &str) -> anyhow::Result<Self> {
        let http = Client::builder()
            // GitHub's API rejects requests without a user agent.
            .user_agent(concat!("script-portal/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            // Redirects could carry the access token to another host.
            .redirect(redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            client_id: cfg.client_id.clone(),
            client_secret: cfg.client_secret.clone(),
            redirect_uri: format!("{}/login/github/callback", public_url.trim_end_matches('/')),
            web_url: cfg.web_url.trim_end_matches('/').to_string(),
            api_url: cfg.api_url.trim_end_matches('/').to_string(),
            allowed_orgs: cfg.allowed_orgs.clone(),
        })
    }

    async fn token(&self, code: &str) -> Result<String, AuthError> {
        let response = self
            .http
            .post(format!("{}/login/oauth/access_token", self.web_url))
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(unavailable)?;
        let body: TokenResponse = response.json().await.map_err(unavailable)?;
        match body.access_token {
            Some(token) => Ok(token),
            None => Err(AuthError::Rejected(format!(
                "GitHub did not issue a token: {}",
                body.error.unwrap_or_else(|| "no error given".into())
            ))),
        }
    }

    /// GET from the API with the user's token. A missing optional resource is
    /// not an error for the caller; the callers decide.
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        token: &str,
        path: &str,
    ) -> Result<Option<T>, AuthError> {
        let response = self
            .http
            .get(format!("{}{path}", self.api_url))
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(unavailable)?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => Ok(Some(response.json().await.map_err(unavailable)?)),
            status => Err(AuthError::ProviderUnavailable(format!(
                "GitHub API answered {status} for {path}"
            ))),
        }
    }
}

#[async_trait]
impl GithubLogin for GithubAdapter {
    fn authorize_url(&self, state: &str) -> String {
        let mut url = url::Url::parse(&format!("{}/login/oauth/authorize", self.web_url))
            .expect("web_url was validated at startup");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", SCOPES)
            .append_pair("state", state);
        url.into()
    }

    async fn complete(&self, code: &str) -> Result<User, AuthError> {
        let token = self.token(code).await?;
        let user: ApiUser = self
            .get(&token, "/user")
            .await?
            .ok_or_else(|| AuthError::ProviderUnavailable("GitHub returned no user".into()))?;

        let email = match user.email {
            Some(email) => Some(email),
            None => self
                .get::<Vec<ApiEmail>>(&token, "/user/emails")
                .await?
                .unwrap_or_default()
                .into_iter()
                .find(|e| e.primary && e.verified)
                .map(|e| e.email),
        };

        let orgs = self
            .get::<Vec<ApiOrg>>(&token, "/user/orgs?per_page=100")
            .await?
            .unwrap_or_default();
        let teams = self
            .get::<Vec<ApiTeam>>(&token, "/user/teams?per_page=100")
            .await?
            .unwrap_or_default();

        let mut groups: Vec<String> = orgs.into_iter().map(|o| o.login).collect();
        groups.extend(
            teams
                .into_iter()
                .map(|t| format!("{}/{}", t.organization.login, t.slug)),
        );

        if !self.allowed_orgs.is_empty() {
            let member = self.allowed_orgs.iter().any(|allowed| {
                groups.iter().any(|g| {
                    g.eq_ignore_ascii_case(allowed)
                        || g.split('/')
                            .next()
                            .is_some_and(|org| org.eq_ignore_ascii_case(allowed))
                })
            });
            if !member {
                return Err(AuthError::Rejected(
                    "the GitHub account is not a member of an allowed organisation".into(),
                ));
            }
        }

        Ok(User {
            display_name: user.name.unwrap_or_else(|| user.login.clone()),
            subject: user.login,
            email,
            groups,
            provider: AuthProvider::Github,
        })
    }
}

fn unavailable(e: impl std::fmt::Display) -> AuthError {
    AuthError::ProviderUnavailable(e.to_string())
}
