//! LDAP authentication adapter (search-then-bind) built on `ldap3`.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use ldap3::{ldap_escape, Ldap, LdapConnAsync, LdapConnSettings, Scope, SearchEntry};

use crate::config::model::LdapConfig;
use crate::tls::{rustls_client_config, TlsOptions};
use sql_portal_domain::CredentialAuthenticator;
use sql_portal_domain::{AuthError, AuthProvider, User};

pub struct LdapAuthenticator {
    cfg: LdapConfig,
    tls: Option<std::sync::Arc<rustls::ClientConfig>>,
}

impl LdapAuthenticator {
    pub fn new(cfg: LdapConfig, tls: &TlsOptions) -> anyhow::Result<Self> {
        let tls = if tls.is_set() {
            Some(rustls_client_config(tls)?)
        } else {
            None
        };
        Ok(Self { cfg, tls })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(self.cfg.timeout_secs.max(1))
    }

    async fn connect(&self) -> Result<Ldap, AuthError> {
        let mut settings = LdapConnSettings::new()
            .set_conn_timeout(self.timeout())
            .set_starttls(self.cfg.start_tls);
        if let Some(tls) = &self.tls {
            settings = settings.set_config(tls.clone());
        }
        let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &self.cfg.url)
            .await
            .map_err(unavailable)?;
        ldap3::drive!(conn);
        ldap.with_timeout(self.timeout());
        Ok(ldap)
    }

    async fn find_user(&self, ldap: &mut Ldap, username: &str) -> Result<SearchEntry, AuthError> {
        let filter = build_user_filter(&self.cfg.user_filter, username);
        let mut attrs = vec![
            self.cfg.username_attribute.as_str(),
            self.cfg.display_name_attribute.as_str(),
            self.cfg.email_attribute.as_str(),
        ];
        if !self.cfg.group_attribute.is_empty() {
            attrs.push(self.cfg.group_attribute.as_str());
        }
        let (entries, _) = ldap
            .search(&self.cfg.base_dn, Scope::Subtree, &filter, attrs)
            .await
            .map_err(unavailable)?
            .success()
            .map_err(unavailable)?;

        let mut entries = entries.into_iter();
        match (entries.next(), entries.next()) {
            (Some(first), None) => Ok(SearchEntry::construct(first)),
            (None, _) => Err(AuthError::InvalidCredentials),
            (Some(_), Some(_)) => {
                tracing::warn!("ambiguous LDAP user filter matched several entries");
                Err(AuthError::InvalidCredentials)
            }
        }
    }

    async fn search_groups(
        &self,
        ldap: &mut Ldap,
        user_dn: &str,
        username: &str,
    ) -> Result<Vec<String>, AuthError> {
        let Some(search) = &self.cfg.group_search else {
            return Ok(vec![]);
        };
        let filter = search
            .filter
            .replace("{user_dn}", &ldap_escape(user_dn))
            .replace("{username}", &ldap_escape(username));
        let (entries, _) = ldap
            .search(
                &search.base_dn,
                Scope::Subtree,
                &filter,
                vec![search.name_attribute.as_str()],
            )
            .await
            .map_err(unavailable)?
            .success()
            .map_err(unavailable)?;

        let mut groups = Vec::new();
        for raw in entries {
            let entry = SearchEntry::construct(raw);
            groups.push(entry.dn.clone());
            groups.extend(attr_values(&entry.attrs, &search.name_attribute));
        }
        Ok(groups)
    }
}

#[async_trait]
impl CredentialAuthenticator for LdapAuthenticator {
    async fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError> {
        let username = username.trim();
        // An empty password would turn into an unauthenticated ("anonymous")
        // bind on many servers, which "succeeds". Never allow it.
        if username.is_empty() || password.is_empty() {
            return Err(AuthError::InvalidCredentials);
        }

        // 1. Look the user up with the service account.
        let mut service = self.connect().await?;
        let bind = service
            .simple_bind(&self.cfg.bind_dn, &self.cfg.bind_password)
            .await
            .map_err(unavailable)?;
        if bind.rc != 0 {
            return Err(AuthError::ProviderUnavailable(
                "LDAP service account bind failed".into(),
            ));
        }
        let entry = self.find_user(&mut service, username).await?;

        // 2. Verify the password with a bind as the user on a fresh connection
        //    (re-binding the service connection would change its identity).
        let mut as_user = self.connect().await?;
        let result = as_user
            .simple_bind(&entry.dn, password)
            .await
            .map_err(unavailable)?;
        let _ = as_user.unbind().await;
        match result.rc {
            0 => {}
            49 => return Err(AuthError::InvalidCredentials),
            rc => {
                return Err(AuthError::ProviderUnavailable(format!(
                    "unexpected LDAP result code {rc}"
                )))
            }
        }

        // 3. Collect groups (memberOf and/or group search) and build the user.
        let mut user = user_from_entry(&entry, &self.cfg, username);
        let found = self
            .search_groups(&mut service, &entry.dn, username)
            .await?;
        for g in found {
            if !user.groups.contains(&g) {
                user.groups.push(g);
            }
        }
        let _ = service.unbind().await;
        Ok(user)
    }
}

fn unavailable(e: impl std::fmt::Display) -> AuthError {
    AuthError::ProviderUnavailable(e.to_string())
}

/// Substitutes the escaped user name into the configured filter template.
pub fn build_user_filter(template: &str, username: &str) -> String {
    template.replace("{username}", &ldap_escape(username))
}

/// Case-insensitive attribute lookup (servers differ in attribute casing).
fn attr_values(attrs: &HashMap<String, Vec<String>>, name: &str) -> Vec<String> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

fn first_attr(attrs: &HashMap<String, Vec<String>>, name: &str) -> Option<String> {
    attr_values(attrs, name).into_iter().find(|v| !v.is_empty())
}

/// Extracts the value of the first RDN when it is `cn=`: `cn=ops,ou=g` -> `ops`.
pub fn cn_from_dn(dn: &str) -> Option<String> {
    let first = dn.split(',').next()?.trim();
    let (key, value) = first.split_once('=')?;
    if key.trim().eq_ignore_ascii_case("cn") && !value.trim().is_empty() {
        Some(value.trim().to_string())
    } else {
        None
    }
}

/// Maps an LDAP entry to a domain user. Groups contain both the full DN and
/// the CN, so either can be used in `access.group_roles`.
pub fn user_from_entry(entry: &SearchEntry, cfg: &LdapConfig, login: &str) -> User {
    let subject =
        first_attr(&entry.attrs, &cfg.username_attribute).unwrap_or_else(|| login.to_string());
    let display_name =
        first_attr(&entry.attrs, &cfg.display_name_attribute).unwrap_or_else(|| subject.clone());
    let email = first_attr(&entry.attrs, &cfg.email_attribute);

    let mut groups = Vec::new();
    if !cfg.group_attribute.is_empty() {
        for dn in attr_values(&entry.attrs, &cfg.group_attribute) {
            if let Some(cn) = cn_from_dn(&dn) {
                if !groups.contains(&cn) {
                    groups.push(cn);
                }
            }
            if !groups.contains(&dn) {
                groups.push(dn);
            }
        }
    }
    User {
        subject,
        display_name,
        email,
        groups,
        provider: AuthProvider::Ldap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LdapConfig {
        serde_yaml_ng::from_str(
            "url: ldap://x\nbind_dn: cn=svc\nbind_password: p\nbase_dn: dc=example,dc=org\n",
        )
        .unwrap()
    }

    fn entry(attrs: &[(&str, &[&str])]) -> SearchEntry {
        SearchEntry {
            dn: "uid=alice,ou=people,dc=example,dc=org".into(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
            bin_attrs: HashMap::new(),
        }
    }

    #[test]
    fn filter_escapes_special_characters() {
        let f = build_user_filter("(&(objectClass=person)(uid={username}))", "a*)(uid=*");
        assert!(!f.contains("a*)(uid=*"));
        assert!(f.contains(r"a\2a\29\28uid=\2a"));
    }

    #[test]
    fn filter_keeps_plain_names() {
        assert_eq!(
            build_user_filter("(uid={username})", "alice"),
            "(uid=alice)"
        );
    }

    #[test]
    fn extracts_cn_from_dn() {
        assert_eq!(
            cn_from_dn("cn=Admins,ou=groups,dc=x").as_deref(),
            Some("Admins")
        );
        assert_eq!(cn_from_dn("CN = ops ,ou=g").as_deref(), Some("ops"));
        assert_eq!(cn_from_dn("ou=groups,dc=x"), None);
        assert_eq!(cn_from_dn("garbage"), None);
    }

    #[test]
    fn maps_entry_to_user_with_cn_and_dn_groups() {
        let e = entry(&[
            ("uid", &["alice"]),
            ("cn", &["Alice Liddell"]),
            ("mail", &["alice@example.org"]),
            (
                "memberOf",
                &["cn=analysts,ou=groups,dc=example,dc=org", "ou=odd,dc=x"],
            ),
        ]);
        let u = user_from_entry(&e, &cfg(), "login");
        assert_eq!(u.subject, "alice");
        assert_eq!(u.display_name, "Alice Liddell");
        assert_eq!(u.email.as_deref(), Some("alice@example.org"));
        assert_eq!(
            u.groups,
            vec![
                "analysts",
                "cn=analysts,ou=groups,dc=example,dc=org",
                "ou=odd,dc=x"
            ]
        );
        assert_eq!(u.provider, AuthProvider::Ldap);
    }

    #[test]
    fn attribute_lookup_is_case_insensitive_and_has_fallbacks() {
        let e = entry(&[("MEMBEROF", &["cn=g,dc=x"])]);
        let u = user_from_entry(&e, &cfg(), "bob");
        assert_eq!(u.subject, "bob");
        assert_eq!(u.display_name, "bob");
        assert!(u.email.is_none());
        assert!(u.groups.contains(&"g".to_string()));
    }

    #[test]
    fn group_attribute_can_be_disabled() {
        let mut c = cfg();
        c.group_attribute = String::new();
        let e = entry(&[("memberOf", &["cn=g,dc=x"])]);
        assert!(user_from_entry(&e, &c, "bob").groups.is_empty());
    }

    #[tokio::test]
    async fn empty_credentials_are_rejected_before_any_network_io() {
        let auth = LdapAuthenticator::new(cfg(), &TlsOptions::default()).unwrap();
        assert!(matches!(
            auth.authenticate("alice", "").await,
            Err(AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            auth.authenticate("  ", "pw").await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn unreachable_server_is_reported_as_unavailable() {
        let mut c = cfg();
        c.url = "ldap://127.0.0.1:1".into();
        c.timeout_secs = 1;
        let auth = LdapAuthenticator::new(c, &TlsOptions::default()).unwrap();
        assert!(matches!(
            auth.authenticate("alice", "pw").await,
            Err(AuthError::ProviderUnavailable(_))
        ));
    }
}
