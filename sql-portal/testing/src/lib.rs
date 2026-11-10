//! In-memory fakes for the ports, used by unit and integration tests.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use sql_portal_domain::{
    AuthError, AuthProvider, QueryError, QueryRequest, QueryResult, ScriptDefinition, User,
};
use sql_portal_domain::{
    CredentialAuthenticator, OidcAuthorization, OidcPending, OidcProvider, QueryExecutor,
    ScriptCatalog,
};

/// LDAP fake accepting exactly one username/password pair.
pub struct FakeLdap {
    username: String,
    password: String,
    groups: Vec<String>,
}

impl FakeLdap {
    pub fn accepting(username: &str, password: &str) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            groups: vec![],
        }
    }

    pub fn with_groups(mut self, groups: &[&str]) -> Self {
        self.groups = groups.iter().map(|g| g.to_string()).collect();
        self
    }
}

#[async_trait]
impl CredentialAuthenticator for FakeLdap {
    async fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError> {
        if username == self.username && password == self.password {
            Ok(User {
                subject: username.into(),
                display_name: format!("{username} (ldap)"),
                email: Some(format!("{username}@example.org")),
                groups: self.groups.clone(),
                provider: AuthProvider::Ldap,
            })
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }
}

/// OIDC fake that always succeeds when the callback is consistent.
#[derive(Default)]
pub struct FakeOidc {
    pub groups: Vec<String>,
}

#[async_trait]
impl OidcProvider for FakeOidc {
    async fn begin(&self) -> Result<OidcAuthorization, AuthError> {
        let pending = OidcPending {
            state: "state-123".into(),
            nonce: "nonce-123".into(),
            pkce_verifier: "verifier-123".into(),
        };
        Ok(OidcAuthorization {
            redirect_url: "https://idp.example.org/authorize?state=state-123".into(),
            pending,
        })
    }

    async fn complete(&self, code: &str, pending: &OidcPending) -> Result<User, AuthError> {
        if code == "good-code" || code == "c" {
            assert_eq!(pending.pkce_verifier, "verifier-123");
            Ok(User {
                subject: "oidc-user".into(),
                display_name: "OIDC User".into(),
                email: None,
                groups: self.groups.clone(),
                provider: AuthProvider::Oidc,
            })
        } else {
            Err(AuthError::Rejected("bad code".into()))
        }
    }

    async fn probe(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Executor returning a canned result (or error) and remembering the last request.
#[derive(Default)]
pub struct FakeExecutor {
    result: Mutex<Option<Result<QueryResult, QueryError>>>,
    last: Mutex<Option<QueryRequest>>,
}

impl FakeExecutor {
    pub fn returning(result: QueryResult) -> Self {
        Self {
            result: Mutex::new(Some(Ok(result))),
            last: Mutex::default(),
        }
    }

    pub fn failing(error: QueryError) -> Self {
        Self {
            result: Mutex::new(Some(Err(error))),
            last: Mutex::default(),
        }
    }

    pub fn last_request(&self) -> Option<QueryRequest> {
        self.last.lock().unwrap().clone()
    }
}

#[async_trait]
impl QueryExecutor for FakeExecutor {
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, QueryError> {
        *self.last.lock().unwrap() = Some(request);
        self.result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(QueryResult::default()))
    }

    async fn ping(&self) -> Result<(), QueryError> {
        Ok(())
    }

    async fn run_script(&self, _sql: &str) -> Result<(), QueryError> {
        Ok(())
    }
}

pub struct InMemoryCatalog {
    scripts: Vec<Arc<ScriptDefinition>>,
}

impl InMemoryCatalog {
    pub fn new(scripts: Vec<ScriptDefinition>) -> Self {
        Self {
            scripts: scripts.into_iter().map(Arc::new).collect(),
        }
    }
}

impl ScriptCatalog for InMemoryCatalog {
    fn all(&self) -> Vec<Arc<ScriptDefinition>> {
        self.scripts.clone()
    }

    fn get(&self, name: &str) -> Option<Arc<ScriptDefinition>> {
        self.scripts
            .iter()
            .find(|s| s.name.as_str() == name)
            .cloned()
    }
}

/// GitHub fake: accepts the code `good-code` for the given user, rejects others.
pub struct FakeGithub {
    pub user: User,
}

#[async_trait]
impl sql_portal_domain::GithubLogin for FakeGithub {
    fn authorize_url(&self, state: &str) -> String {
        format!("https://github.example.org/login/oauth/authorize?state={state}")
    }

    async fn complete(&self, code: &str) -> Result<User, AuthError> {
        if code == "good-code" {
            Ok(self.user.clone())
        } else {
            Err(AuthError::Rejected("bad verification code".into()))
        }
    }
}
