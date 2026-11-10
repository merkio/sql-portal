//! Architecture rules, checked against the Cargo dependency graph.
//!
//! Layer boundaries are crate boundaries, so the compiler already rejects an
//! import that crosses a missing edge. These tests pin the edges themselves:
//! adding a forbidden dependency to a `Cargo.toml` fails here with the rule
//! that it breaks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use sql_portal_integration_tests::REPO_ROOT;

/// Normal (non-dev, non-build) dependencies of every workspace package.
fn dependency_graph() -> BTreeMap<String, BTreeSet<String>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .arg("--manifest-path")
        .arg(Path::new(REPO_ROOT).join("Cargo.toml"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");
    metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|package| {
            let deps = package["dependencies"]
                .as_array()
                .expect("dependencies")
                .iter()
                .filter(|d| d["kind"].is_null())
                .map(|d| d["name"].as_str().expect("name").to_string())
                .collect();
            (package["name"].as_str().expect("name").to_string(), deps)
        })
        .collect()
}

fn deps_of<'a>(
    graph: &'a BTreeMap<String, BTreeSet<String>>,
    crate_name: &str,
) -> &'a BTreeSet<String> {
    graph
        .get(crate_name)
        .unwrap_or_else(|| panic!("{crate_name} is not a workspace member"))
}

/// The workspace crates `crate_name` depends on.
fn internal_deps(graph: &BTreeMap<String, BTreeSet<String>>, crate_name: &str) -> BTreeSet<String> {
    deps_of(graph, crate_name)
        .iter()
        .filter(|d| graph.contains_key(*d))
        .cloned()
        .collect()
}

fn assert_internal_deps_within(crate_name: &str, allowed: &[&str]) {
    let graph = dependency_graph();
    let allowed: BTreeSet<String> = allowed.iter().map(|s| s.to_string()).collect();
    let extra: Vec<_> = internal_deps(&graph, crate_name)
        .difference(&allowed)
        .cloned()
        .collect();
    assert!(
        extra.is_empty(),
        "{crate_name} must not depend on {extra:?} (allowed: {allowed:?})"
    );
}

#[test]
fn domain_depends_only_on_the_sql_executor_contract() {
    assert_internal_deps_within("sql-portal-domain", &["sql-executor"]);
}

#[test]
fn domain_has_no_web_database_or_identity_provider_libraries() {
    let graph = dependency_graph();
    let forbidden = [
        "axum",
        "tower",
        "tower-http",
        "tower-sessions",
        "sqlx",
        "reqwest",
        "ldap3",
        "openidconnect",
        "rustls",
        "tokio",
        "metrics",
        "tracing",
    ];
    let found: Vec<_> = deps_of(&graph, "sql-portal-domain")
        .iter()
        .filter(|d| forbidden.contains(&d.as_str()))
        .collect();
    assert!(
        found.is_empty(),
        "domain depends on infrastructure crates {found:?}"
    );
}

#[test]
fn application_depends_only_on_domain() {
    assert_internal_deps_within("sql-portal-application", &["sql-portal-domain"]);
}

#[test]
fn adapters_implement_domain_ports_over_the_datasources() {
    assert_internal_deps_within(
        "sql-portal-adapters",
        &[
            "sql-portal-domain",
            "sql-executor",
            "sqlite-dialect",
            "postgres-dialect",
            "clickhouse-dialect",
            "tls-config",
        ],
    );
}

#[test]
fn ui_depends_on_use_cases_not_on_adapters() {
    assert_internal_deps_within(
        "sql-portal-api",
        &["sql-portal-domain", "sql-portal-application"],
    );
}

#[test]
fn deployment_is_the_only_crate_that_wires_every_layer() {
    let graph = dependency_graph();
    let deployment = internal_deps(&graph, "sql-portal");
    for layer in [
        "sql-portal-domain",
        "sql-portal-application",
        "sql-portal-adapters",
        "sql-portal-api",
    ] {
        assert!(deployment.contains(layer), "deployment must wire {layer}");
    }
    let dependents: Vec<_> = graph
        .iter()
        .filter(|(name, deps)| {
            deps.contains("sql-portal") && *name != "sql-portal-integration-tests"
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        dependents.is_empty(),
        "only tests may depend on deployment, found {dependents:?}"
    );
}

#[test]
fn test_fakes_are_never_a_production_dependency() {
    let graph = dependency_graph();
    let dependents: Vec<_> = graph
        .iter()
        .filter(|(name, deps)| {
            deps.contains("sql-portal-testing") && *name != "sql-portal-integration-tests"
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        dependents.is_empty(),
        "{dependents:?} depend on test fakes outside dev-dependencies"
    );
}

#[test]
fn datasources_build_on_the_sql_executor_only() {
    assert_internal_deps_within("sql-executor", &[]);
    for dialect in ["sqlite-dialect", "postgres-dialect", "clickhouse-dialect"] {
        assert_internal_deps_within(dialect, &["sql-executor"]);
    }
}

#[test]
fn common_crates_do_not_know_the_portal() {
    for name in [
        "config-loader",
        "template-render",
        "tls-config",
        "prometheus-metrics",
    ] {
        assert_internal_deps_within(name, &[]);
    }
}
