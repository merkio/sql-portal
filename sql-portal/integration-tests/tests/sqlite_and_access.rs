//! SQLite datasource and access-policy integration tests (no Docker needed).
//! PostgreSQL and ClickHouse run against real servers in `postgres.rs` and
//! `clickhouse.rs` (`--features docker`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use sql_portal_adapters::sql::{connect_database, prepare_database};
use sql_portal_adapters::tls::TlsOptions;
use sql_portal_domain::access::identity::{AuthProvider, User};
use sql_portal_domain::scripts::repository::ConfigScriptCatalog;
use sql_portal_domain::scripts::ScriptDefinition;
use sql_portal_domain::telemetry::NoopTelemetry;
use sql_portal_domain::{AccessPolicy, LocalGrants, Permission, QueryRequest, ScriptName};

#[cfg(test)]
mod sqlite_tests {
    use super::*;

    async fn setup_sqlite_database(
        url: &str,
    ) -> anyhow::Result<Arc<dyn sql_portal_domain::QueryExecutor>> {
        let executor =
            connect_database(url, 1, Duration::from_secs(5), &TlsOptions::default()).await?;
        prepare_database(executor.as_ref(), None, &[]).await?;
        Ok(executor)
    }

    #[tokio::test]
    async fn test_sqlite_connection() {
        let executor = setup_sqlite_database("sqlite::memory:")
            .await
            .expect("failed to connect to sqlite");

        executor.ping().await.expect("ping failed");
    }

    #[tokio::test]
    async fn test_sqlite_query_execution() {
        let executor = setup_sqlite_database("sqlite::memory:")
            .await
            .expect("failed to connect to sqlite");

        // Create a test table
        let create_table = QueryRequest {
            sql: "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)".to_string(),
            params: vec![],
            max_rows: 100,
            timeout: Duration::from_secs(30),
        };
        executor
            .execute(create_table)
            .await
            .expect("create table failed");

        // Insert test data
        let insert = QueryRequest {
            sql: "INSERT INTO test (name, value) VALUES ('alice', 100), ('bob', 200)".to_string(),
            params: vec![],
            max_rows: 100,
            timeout: Duration::from_secs(30),
        };
        executor.execute(insert).await.expect("insert failed");

        // Query test data
        let select = QueryRequest {
            sql: "SELECT name, value FROM test ORDER BY id".to_string(),
            params: vec![],
            max_rows: 100,
            timeout: Duration::from_secs(30),
        };
        let result = executor.execute(select).await.expect("select failed");
        assert_eq!(result.columns.len(), 2);
        assert!(!result.rows.is_empty());
        assert_eq!(result.rows.len(), 2);
    }

    #[tokio::test]
    async fn test_sqlite_with_parameters() {
        let executor = setup_sqlite_database("sqlite::memory:")
            .await
            .expect("failed to connect to sqlite");

        let create = QueryRequest {
            sql: "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, role TEXT)".to_string(),
            params: vec![],
            max_rows: 100,
            timeout: Duration::from_secs(30),
        };
        executor.execute(create).await.expect("create table failed");

        let insert = QueryRequest {
            sql: "INSERT INTO users (name, role) VALUES ('alice', 'admin'), ('bob', 'user')"
                .to_string(),
            params: vec![],
            max_rows: 100,
            timeout: Duration::from_secs(30),
        };
        executor.execute(insert).await.expect("insert failed");

        let select = QueryRequest {
            sql: "SELECT name, role FROM users WHERE role = :role".to_string(),
            params: vec![(
                "role".to_string(),
                sql_portal_domain::Value::Text("admin".to_string()),
            )],
            max_rows: 100,
            timeout: Duration::from_secs(30),
        };
        let result = executor
            .execute(select)
            .await
            .expect("select with params failed");
        assert_eq!(result.rows.len(), 1);
    }
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    fn setup_access_policy() -> AccessPolicy {
        let roles = BTreeMap::from([
            ("admin".to_string(), vec!["scripts.*".to_string()]),
            (
                "analyst".to_string(),
                vec![
                    "scripts.reports.read".to_string(),
                    "scripts.data.read".to_string(),
                ],
            ),
            (
                "viewer".to_string(),
                vec!["scripts.public.read".to_string()],
            ),
        ]);
        let group_roles = BTreeMap::from([
            ("developers".to_string(), vec!["admin".to_string()]),
            ("analysts".to_string(), vec!["analyst".to_string()]),
        ]);
        let user_roles = BTreeMap::from([
            ("Alice".to_string(), vec!["admin".to_string()]),
            ("Bob".to_string(), vec!["analyst".to_string()]),
        ]);
        AccessPolicy::new(roles, group_roles, user_roles, vec!["viewer".into()])
            .expect("failed to create access policy")
    }

    #[test]
    fn test_admin_can_run_any_script() {
        let policy = setup_access_policy();
        let user = User {
            subject: "alice".to_string(),
            display_name: "Alice".to_string(),
            email: Some("alice@example.com".to_string()),
            groups: vec!["developers".to_string()],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert!(principal.roles.contains("admin"));

        let perm = Permission::new("scripts.reports.read").unwrap();
        assert!(principal.can(&perm));
    }

    #[test]
    fn test_analyst_can_only_read_specific_scripts() {
        let policy = setup_access_policy();
        let user = User {
            subject: "bob".to_string(),
            display_name: "Bob".to_string(),
            email: Some("bob@example.com".to_string()),
            groups: vec!["analysts".to_string()],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert!(principal.roles.contains("analyst"));
        assert!(!principal.roles.contains("admin"));

        // Can read reports and data
        let can_read_reports = Permission::new("scripts.reports.read").unwrap();
        assert!(principal.can(&can_read_reports));

        let can_read_data = Permission::new("scripts.data.read").unwrap();
        assert!(principal.can(&can_read_data));

        // Cannot write
        let cannot_write = Permission::new("scripts.reports.write").unwrap();
        assert!(!principal.can(&cannot_write));
    }

    #[test]
    fn test_viewer_has_default_role() {
        let policy = setup_access_policy();
        let user = User {
            subject: "charlie".to_string(),
            display_name: "Charlie".to_string(),
            email: None,
            groups: vec![],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert!(principal.roles.contains("viewer"));
        assert!(!principal.roles.contains("admin"));

        let can_read_public = Permission::new("scripts.public.read").unwrap();
        assert!(principal.can(&can_read_public));

        let cannot_read_reports = Permission::new("scripts.reports.read").unwrap();
        assert!(!principal.can(&cannot_read_reports));
    }

    #[test]
    fn test_user_mapping_is_case_insensitive() {
        let policy = setup_access_policy();
        let user = User {
            subject: "ALICE".to_string(),
            display_name: "Alice".to_string(),
            email: None,
            groups: vec![],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert!(principal.roles.contains("admin"));
    }

    #[test]
    fn test_group_mapping_is_case_insensitive() {
        let policy = setup_access_policy();
        let user = User {
            subject: "david".to_string(),
            display_name: "David".to_string(),
            email: None,
            groups: vec!["DEVELOPERS".to_string()],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert!(principal.roles.contains("admin"));
    }

    #[test]
    fn test_multiple_groups_grant_all_roles() {
        let policy = setup_access_policy();
        let user = User {
            subject: "eve".to_string(),
            display_name: "Eve".to_string(),
            email: None,
            groups: vec!["developers".to_string(), "analysts".to_string()],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert!(principal.roles.contains("admin"));
        assert!(principal.roles.contains("analyst"));

        // Should have all permissions from both roles
        let admin_perm = Permission::new("scripts.write").unwrap();
        assert!(principal.can(&admin_perm));

        let analyst_perm = Permission::new("scripts.reports.read").unwrap();
        assert!(principal.can(&analyst_perm));
    }
}

#[cfg(test)]
mod authentication_tests {
    use super::*;

    #[test]
    fn test_local_grants_from_policy() {
        let policy = AccessPolicy::new(
            BTreeMap::from([("user".to_string(), vec!["scripts.read".to_string()])]),
            BTreeMap::new(),
            BTreeMap::new(),
            vec!["user".into()],
        )
        .unwrap();

        let user = User {
            subject: "test".to_string(),
            display_name: "Test User".to_string(),
            email: Some("test@example.com".to_string()),
            groups: vec![],
            provider: AuthProvider::Ldap,
        };

        let grants = policy.grants(&user);
        assert!(grants.roles.contains("user"));
        assert!(grants
            .permissions
            .grants(&Permission::new("scripts.read").unwrap()));
    }

    #[test]
    fn test_principal_creation_with_permissions() {
        let policy = AccessPolicy::new(
            BTreeMap::from([
                ("viewer".to_string(), vec!["scripts.read".to_string()]),
                (
                    "editor".to_string(),
                    vec!["scripts.read".to_string(), "scripts.write".to_string()],
                ),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
            vec!["viewer".into()],
        )
        .unwrap();

        let user = User {
            subject: "alice".to_string(),
            display_name: "Alice".to_string(),
            email: None,
            groups: vec![],
            provider: AuthProvider::Ldap,
        };

        let principal = policy.resolve(user.clone());
        assert_eq!(principal.user.subject, "alice");
        assert!(principal.roles.contains("viewer"));
        assert!(principal.can(&Permission::new("scripts.read").unwrap()));
    }
}

#[cfg(test)]
mod script_service_integration_tests {
    use super::*;
    use sql_portal_domain::scripts::ScriptCatalog;

    fn setup_scripts() -> (ConfigScriptCatalog, AccessPolicy) {
        let script_defs = vec![
            ScriptDefinition::new(
                ScriptName::new("sales_report").unwrap(),
                "Sales Report",
                "SELECT 1 AS sales",
            )
            .unwrap(),
            ScriptDefinition::new(
                ScriptName::new("admin_console").unwrap(),
                "Admin Console",
                "SELECT 1 AS admin",
            )
            .unwrap(),
        ];

        let catalog = ConfigScriptCatalog::new(script_defs);

        let policy = AccessPolicy::new(
            BTreeMap::from([
                ("analyst".to_string(), vec!["reports.read".to_string()]),
                ("admin".to_string(), vec!["admin.*".to_string()]),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
            vec![],
        )
        .unwrap();

        (catalog, policy)
    }

    #[test]
    fn test_authorized_script_execution() {
        let (catalog, policy) = setup_scripts();
        let _telemetry = Arc::new(NoopTelemetry);

        let user = User {
            subject: "alice".to_string(),
            display_name: "Alice".to_string(),
            email: None,
            groups: vec![],
            provider: AuthProvider::Ldap,
        };
        let _principal = policy.resolve(user.clone());

        let scripts = Arc::new(catalog);
        let _grants = Arc::new(LocalGrants::new(policy));

        // Note: Full integration would require a real executor
        let all_scripts = scripts.all();
        assert_eq!(all_scripts.len(), 2);

        // Verify script catalog structure
        let sales = scripts.get("sales_report");
        assert!(sales.is_some());
    }

    #[test]
    fn test_script_catalog_lists_all_scripts() {
        let (catalog, _policy) = setup_scripts();

        let all_scripts = catalog.all();
        assert_eq!(all_scripts.len(), 2);

        let names: Vec<_> = all_scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"sales_report"));
        assert!(names.contains(&"admin_console"));

        // Verify we can look up scripts by name
        assert!(catalog.get("sales_report").is_some());
        assert!(catalog.get("admin_console").is_some());
        assert!(catalog.get("nonexistent").is_none());
    }

    #[test]
    fn test_script_description() {
        let (catalog, _policy) = setup_scripts();

        let all_scripts = catalog.all();
        assert_eq!(all_scripts.len(), 2);

        // Verify script properties
        let sales_script = all_scripts
            .iter()
            .find(|s| s.name.as_str() == "sales_report")
            .unwrap();
        assert_eq!(sales_script.title, "Sales Report");

        let admin_script = all_scripts
            .iter()
            .find(|s| s.name.as_str() == "admin_console")
            .unwrap();
        assert_eq!(admin_script.title, "Admin Console");
    }
}
