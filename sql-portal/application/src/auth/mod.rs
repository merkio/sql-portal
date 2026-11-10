use std::sync::Arc;

use tracing::{info, instrument, warn};

use crate::observability::Metrics;
use sql_portal_domain::GrantSource;
use sql_portal_domain::{AuthError, Grants, Principal, User};
use sql_portal_domain::{
    CredentialAuthenticator, GithubLogin, OidcAuthorization, OidcPending, OidcProvider,
};

/// Where a GitHub sign-in starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAuthorization {
    pub redirect_url: String,
    pub state: String,
}

/// Query parameters returned by the identity provider.
#[derive(Debug, Clone)]
pub struct OidcCallback {
    pub code: String,
    pub state: String,
}

/// The configured identity providers. Absent providers are simply `None`.
#[derive(Clone, Default)]
pub struct Providers {
    pub ldap: Option<Arc<dyn CredentialAuthenticator>>,
    pub oidc: Option<Arc<dyn OidcProvider>>,
    pub github: Option<Arc<dyn GithubLogin>>,
}

/// Whether authentication is on at all. `Disabled` is the master switch:
/// every request is an anonymous administrator and no provider is consulted.
#[derive(Clone)]
pub enum AuthMode {
    Disabled,
    Enabled(Providers),
}

/// Use cases around authenticating users and resolving their permissions.
pub struct AuthService {
    mode: AuthMode,
    permissions: Arc<dyn GrantSource>,
    metrics: Arc<dyn Metrics>,
}

impl AuthService {
    pub fn new(
        mode: AuthMode,
        permissions: Arc<dyn GrantSource>,
        metrics: Arc<dyn Metrics>,
    ) -> Self {
        Self {
            mode,
            permissions,
            metrics,
        }
    }

    /// The single place that turns the master switch into an error.
    fn providers(&self) -> Result<&Providers, AuthError> {
        match &self.mode {
            AuthMode::Disabled => Err(AuthError::Disabled),
            AuthMode::Enabled(providers) => Ok(providers),
        }
    }

    /// `false` means *all* authorizers are switched off.
    pub fn is_enabled(&self) -> bool {
        matches!(self.mode, AuthMode::Enabled(_))
    }

    pub fn ldap_enabled(&self) -> bool {
        self.providers().is_ok_and(|p| p.ldap.is_some())
    }

    pub fn oidc_enabled(&self) -> bool {
        self.providers().is_ok_and(|p| p.oidc.is_some())
    }

    pub fn github_enabled(&self) -> bool {
        self.providers().is_ok_and(|p| p.github.is_some())
    }

    /// The GitHub sign-in URL, and the state that must come back with the code.
    #[instrument(skip(self), fields(provider = "github"))]
    pub async fn github_begin(&self) -> Result<GithubAuthorization, AuthError> {
        let github = self
            .providers()?
            .github
            .as_ref()
            .ok_or(AuthError::ProviderNotEnabled)?;
        let state = uuid::Uuid::new_v4().simple().to_string();
        Ok(GithubAuthorization {
            redirect_url: github.authorize_url(&state),
            state,
        })
    }

    /// Completes the GitHub sign-in. `expected_state` is the value stored when
    /// the sign-in started; a different `returned_state` is a forgery attempt.
    #[instrument(
        skip(self, code, expected_state, returned_state),
        fields(provider = "github")
    )]
    pub async fn github_complete(
        &self,
        code: &str,
        expected_state: &str,
        returned_state: &str,
    ) -> Result<User, AuthError> {
        let github = self
            .providers()?
            .github
            .as_ref()
            .ok_or(AuthError::ProviderNotEnabled)?;
        let result = if constant_time_eq(returned_state.as_bytes(), expected_state.as_bytes()) {
            github.complete(code).await
        } else {
            Err(AuthError::Rejected("state mismatch".into()))
        };
        self.record("github", &result);
        match &result {
            Ok(user) => info!(subject = %user.subject, "github login succeeded"),
            Err(e) => warn!(error = %e, "github login failed"),
        }
        result
    }

    /// Resolves the principal for a request, asking the permission source for
    /// the user's grants. With authentication disabled every request is served
    /// as an anonymous administrator. If the source cannot answer, the user is
    /// denied everything (fail closed) and the failure is logged.
    pub async fn principal(&self, session_user: Option<User>) -> Option<Principal> {
        if !self.is_enabled() {
            return Some(Principal::anonymous_admin());
        }
        let user = session_user?;
        let grants = match self.permissions.grants_for(&user).await {
            Ok(grants) => grants,
            Err(e) => {
                warn!(error = %e, subject = %user.subject, "grants unavailable; denying access");
                Grants::default()
            }
        };
        Some(Principal {
            user,
            roles: grants.roles,
            permissions: grants.permissions,
        })
    }

    #[instrument(skip(self, password), fields(provider = "ldap"))]
    pub async fn login_ldap(&self, username: &str, password: &str) -> Result<User, AuthError> {
        let ldap = self
            .providers()?
            .ldap
            .as_ref()
            .ok_or(AuthError::ProviderNotEnabled)?;
        let result = ldap.authenticate(username, password).await;
        self.record("ldap", &result);
        match &result {
            Ok(user) => info!(subject = %user.subject, "ldap login succeeded"),
            Err(e) => warn!(error = %e, "ldap login failed"),
        }
        result
    }

    #[instrument(skip(self), fields(provider = "oidc"))]
    pub async fn oidc_begin(&self) -> Result<OidcAuthorization, AuthError> {
        self.providers()?
            .oidc
            .as_ref()
            .ok_or(AuthError::ProviderNotEnabled)?
            .begin()
            .await
    }

    #[instrument(skip(self, callback, pending), fields(provider = "oidc"))]
    pub async fn oidc_complete(
        &self,
        callback: &OidcCallback,
        pending: &OidcPending,
    ) -> Result<User, AuthError> {
        let oidc = self
            .providers()?
            .oidc
            .as_ref()
            .ok_or(AuthError::ProviderNotEnabled)?;

        // CSRF protection: the state must match the one we issued.
        let result = if constant_time_eq(callback.state.as_bytes(), pending.state.as_bytes()) {
            oidc.complete(&callback.code, pending).await
        } else {
            Err(AuthError::Rejected("state mismatch".into()))
        };
        self.record("oidc", &result);
        match &result {
            Ok(user) => info!(subject = %user.subject, "oidc login succeeded"),
            Err(e) => warn!(error = %e, "oidc login failed"),
        }
        result
    }

    fn record(&self, provider: &str, result: &Result<User, AuthError>) {
        let outcome = match result {
            Ok(_) => "success",
            Err(e) => e.outcome(),
        };
        self.metrics.auth_attempt(provider, outcome);
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::NoopMetrics;
    use crate::observability::RecordingMetrics;
    use sql_portal_domain::GrantSource;
    use sql_portal_domain::LocalGrants;
    use sql_portal_domain::{AccessPolicy, AuthProvider, Grants, PermissionSet};
    use sql_portal_testing::{FakeLdap, FakeOidc};
    use std::collections::BTreeMap;

    fn policy() -> AccessPolicy {
        AccessPolicy::new(
            BTreeMap::from([("r".to_string(), vec!["x.read".to_string()])]),
            BTreeMap::new(),
            BTreeMap::new(),
            vec!["r".into()],
        )
        .unwrap()
    }

    fn service(enabled: bool, metrics: Arc<dyn Metrics>) -> AuthService {
        let mode = if enabled {
            AuthMode::Enabled(Providers {
                ldap: Some(Arc::new(FakeLdap::accepting("alice", "pw"))),
                oidc: Some(Arc::new(FakeOidc::default())),
                github: None,
            })
        } else {
            AuthMode::Disabled
        };
        AuthService::new(mode, Arc::new(LocalGrants::new(policy())), metrics)
    }

    #[tokio::test]
    async fn disabled_auth_serves_anonymous_admin() {
        let s = service(false, Arc::new(NoopMetrics));
        let p = s.principal(None).await.unwrap();
        assert!(p
            .permissions
            .grants(&sql_portal_domain::Permission::new("any").unwrap()));
        assert!(!s.ldap_enabled());
        assert!(!s.oidc_enabled());
        assert!(matches!(
            s.login_ldap("alice", "pw").await,
            Err(AuthError::Disabled)
        ));
        assert!(matches!(s.oidc_begin().await, Err(AuthError::Disabled)));
    }

    #[tokio::test]
    async fn enabled_auth_requires_session_user() {
        let s = service(true, Arc::new(NoopMetrics));
        assert!(s.principal(None).await.is_none());
    }

    /// A permission source that answers with fixed grants or fails.
    struct FixedSource(Option<Grants>);

    #[async_trait::async_trait]
    impl GrantSource for FixedSource {
        async fn grants_for(&self, _: &User) -> Result<Grants, AuthError> {
            self.0
                .clone()
                .ok_or_else(|| AuthError::ProviderUnavailable("down".into()))
        }
    }

    fn user(subject: &str) -> User {
        User {
            subject: subject.into(),
            display_name: subject.into(),
            email: None,
            groups: vec![],
            provider: AuthProvider::Ldap,
        }
    }

    #[tokio::test]
    async fn principal_takes_grants_from_the_permission_source() {
        let grants = Grants {
            roles: ["analyst".to_string()].into_iter().collect(),
            permissions: PermissionSet::new(["scripts.sales.read"]),
        };
        let s = AuthService::new(
            AuthMode::Enabled(Providers::default()),
            Arc::new(FixedSource(Some(grants))),
            Arc::new(NoopMetrics),
        );
        let p = s.principal(Some(user("zoe"))).await.unwrap();
        assert!(p.roles.contains("analyst"));
        assert!(p.can(&sql_portal_domain::Permission::new("scripts.sales.read").unwrap()));
    }

    #[tokio::test]
    async fn unavailable_permission_source_denies_everything() {
        let s = AuthService::new(
            AuthMode::Enabled(Providers::default()),
            Arc::new(FixedSource(None)),
            Arc::new(NoopMetrics),
        );
        let p = s
            .principal(Some(user("zoe")))
            .await
            .expect("still signed in");
        assert!(p.roles.is_empty());
        assert!(!p.can(&sql_portal_domain::Permission::new("scripts.sales.read").unwrap()));
    }

    #[tokio::test]
    async fn ldap_login_success_and_failure_are_recorded() {
        let t = Arc::new(RecordingMetrics::default());
        let s = service(true, t.clone());
        let user = s.login_ldap("alice", "pw").await.unwrap();
        assert_eq!(user.subject, "alice");
        assert!(matches!(
            s.login_ldap("alice", "bad").await,
            Err(AuthError::InvalidCredentials)
        ));
        assert_eq!(
            t.auth_events(),
            vec![
                ("ldap".to_string(), "success".to_string()),
                ("ldap".to_string(), "invalid_credentials".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn principal_resolves_default_roles() {
        let s = service(true, Arc::new(NoopMetrics));
        let user = s.login_ldap("alice", "pw").await.unwrap();
        let p = s.principal(Some(user)).await.unwrap();
        assert!(p.roles.contains("r"));
    }

    #[tokio::test]
    async fn missing_provider_is_reported() {
        let s = AuthService::new(
            AuthMode::Enabled(Providers::default()),
            Arc::new(LocalGrants::new(policy())),
            Arc::new(NoopMetrics),
        );
        assert!(matches!(
            s.login_ldap("a", "b").await,
            Err(AuthError::ProviderNotEnabled)
        ));
        assert!(matches!(
            s.oidc_begin().await,
            Err(AuthError::ProviderNotEnabled)
        ));
    }

    #[tokio::test]
    async fn oidc_flow_checks_state() {
        let t = Arc::new(RecordingMetrics::default());
        let s = service(true, t.clone());
        let auth = s.oidc_begin().await.unwrap();

        let bad = OidcCallback {
            code: "c".into(),
            state: "wrong".into(),
        };
        assert!(matches!(
            s.oidc_complete(&bad, &auth.pending).await,
            Err(AuthError::Rejected(_))
        ));

        let good = OidcCallback {
            code: "c".into(),
            state: auth.pending.state.clone(),
        };
        let user = s.oidc_complete(&good, &auth.pending).await.unwrap();
        assert_eq!(user.provider, AuthProvider::Oidc);
        assert_eq!(
            t.auth_events(),
            vec![
                ("oidc".to_string(), "rejected".to_string()),
                ("oidc".to_string(), "success".to_string())
            ]
        );
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
