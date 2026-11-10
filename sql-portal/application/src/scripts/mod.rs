use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, instrument, warn};

use crate::observability::Metrics;
use sql_portal_domain::{
    ChartData, PageInfo, PageRequest, Principal, QueryExecutor, QueryRequest, QueryResult,
    ScriptAccessRule, ScriptCatalog, ScriptDefinition, ScriptError, Value,
};

/// Entry shown in the list of available pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSummary {
    pub name: String,
    pub title: String,
    pub description: String,
    pub group: Option<String>,
    /// The principal may run the script. Otherwise it is listed as locked.
    pub accessible: bool,
    /// Permissions a caller needs to run the script (all of them).
    pub required_permissions: Vec<String>,
}

/// A script the principal is allowed to use. Only [`ScriptService::authorize`]
/// creates one, so holding it proves the permission check happened.
#[derive(Debug, Clone)]
pub struct AuthorizedScript(Arc<ScriptDefinition>);

impl Deref for AuthorizedScript {
    type Target = ScriptDefinition;

    fn deref(&self) -> &ScriptDefinition {
        &self.0
    }
}

/// Outcome of running a script.
#[derive(Debug, Clone)]
pub struct ScriptRun {
    pub script: Arc<ScriptDefinition>,
    pub params: Vec<(String, Value)>,
    /// The rows of the table: the whole result, or one page of it.
    pub result: QueryResult,
    /// Set when the table is paged; `None` when every row is in `result`.
    pub page: Option<PageInfo>,
    /// One entry per configured chart; errors are user-facing messages.
    pub charts: Vec<Result<ChartData, String>>,
    /// The charts were built from fewer rows than the query returns (`max_rows`).
    pub capped: bool,
    pub duration: Duration,
}

/// Use cases around discovering and running scripts.
pub struct ScriptService {
    catalog: Arc<dyn ScriptCatalog>,
    executor: Arc<dyn QueryExecutor>,
    metrics: Arc<dyn Metrics>,
    access: Arc<dyn ScriptAccessRule>,
}

/// Default script access rule: the principal must hold every permission the
/// script declares.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequiredPermissions;

impl ScriptAccessRule for RequiredPermissions {
    fn can_access(&self, principal: &Principal, script: &ScriptDefinition) -> bool {
        principal.can_all(&script.required_permissions)
    }
}

impl ScriptService {
    pub fn new(
        catalog: Arc<dyn ScriptCatalog>,
        executor: Arc<dyn QueryExecutor>,
        metrics: Arc<dyn Metrics>,
        access: Arc<dyn ScriptAccessRule>,
    ) -> Self {
        Self {
            catalog,
            executor,
            metrics,
            access,
        }
    }

    /// Every script, sorted by title, each marked as accessible or locked for
    /// the principal.
    pub fn list_all(&self, principal: &Principal) -> Vec<ScriptSummary> {
        let mut list: Vec<ScriptSummary> = self
            .catalog
            .all()
            .into_iter()
            .map(|s| ScriptSummary {
                name: s.name.to_string(),
                title: s.title.clone(),
                description: s.description.clone(),
                group: s.group.clone(),
                accessible: self.access.can_access(principal, &s),
                required_permissions: s
                    .required_permissions
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
            })
            .collect();
        list.sort_by_key(|s| s.title.to_lowercase());
        list
    }

    /// Scripts the principal is allowed to use, sorted by title.
    pub fn list_available(&self, principal: &Principal) -> Vec<ScriptSummary> {
        self.list_all(principal)
            .into_iter()
            .filter(|s| s.accessible)
            .collect()
    }

    /// The only gate: finds the script and checks the principal may use it.
    /// Denials are recorded in metrics.
    #[instrument(skip(self, principal), fields(user = %principal.user.subject))]
    pub fn authorize(
        &self,
        principal: &Principal,
        name: &str,
    ) -> Result<AuthorizedScript, ScriptError> {
        let outcome = match self.catalog.get(name) {
            None => Err(ScriptError::NotFound),
            Some(script) if self.access.can_access(principal, &script) => {
                Ok(AuthorizedScript(script))
            }
            Some(_) => Err(ScriptError::Forbidden),
        };
        if let Err(e) = &outcome {
            // Unknown names are not recorded under their own label to keep
            // metric cardinality bounded.
            let label = if matches!(e, ScriptError::NotFound) {
                "_unknown"
            } else {
                name
            };
            self.metrics
                .script_run(label, e.outcome(), Duration::ZERO, 0);
        }
        outcome
    }

    /// Runs a script. With `page`, the table holds that page of the whole
    /// result (the count and the page are two queries); without it, the table
    /// holds the result capped at `max_rows`. Charts always use the capped result.
    #[instrument(skip(self, script, raw_params), fields(script = %script.name))]
    pub async fn run(
        &self,
        script: AuthorizedScript,
        raw_params: &HashMap<String, String>,
        page: Option<PageRequest>,
    ) -> Result<ScriptRun, ScriptError> {
        let started = Instant::now();
        let outcome = self.run_inner(&script, raw_params, page, started).await;
        let name = script.name.to_string();
        match &outcome {
            Ok(run) => {
                self.metrics
                    .script_run(&name, "success", run.duration, run.result.rows.len());
                info!(rows = run.result.rows.len(), "script executed");
            }
            Err(e) => {
                self.metrics
                    .script_run(&name, e.outcome(), started.elapsed(), 0);
                warn!(error = %e, "script run failed");
            }
        }
        outcome
    }

    async fn run_inner(
        &self,
        script: &AuthorizedScript,
        raw_params: &HashMap<String, String>,
        page: Option<PageRequest>,
        started: Instant,
    ) -> Result<ScriptRun, ScriptError> {
        let params = script
            .bind_params(raw_params)
            .map_err(ScriptError::InvalidParams)?;
        let timeout = Duration::from_secs(script.timeout_secs);
        // A table that is not shown has nothing to page.
        let page = page.filter(|_| script.show_table);

        let (result, page_info, chart_source) = match page {
            None => {
                let all = self
                    .query(&script.sql, &params, script.max_rows, timeout)
                    .await?;
                (all, None, None)
            }
            Some(request) => {
                let body = script.sql.trim().trim_end_matches(';').trim_end();
                let total = self.count_rows(body, &params, timeout).await?;
                let info = PageInfo::plan(request, total);
                let paged_sql = format!(
                    "SELECT * FROM (\n{body}\n) AS portal_page LIMIT {} OFFSET {}",
                    info.size,
                    info.offset()
                );
                let table = self.query(&paged_sql, &params, info.size, timeout).await?;
                let chart_source = if script.charts.is_empty() {
                    None
                } else {
                    Some(self.query(body, &params, script.max_rows, timeout).await?)
                };
                (table, Some(info), chart_source)
            }
        };

        let chart_input = chart_source.as_ref().unwrap_or(&result);
        let charts = script.charts.iter().map(|c| c.build(chart_input)).collect();
        let capped = chart_input.truncated;
        Ok(ScriptRun {
            script: script.0.clone(),
            params,
            result,
            page: page_info,
            charts,
            capped,
            duration: started.elapsed(),
        })
    }

    async fn query(
        &self,
        sql: &str,
        params: &[(String, Value)],
        max_rows: usize,
        timeout: Duration,
    ) -> Result<QueryResult, ScriptError> {
        Ok(self
            .executor
            .execute(QueryRequest {
                sql: sql.to_string(),
                params: params.to_vec(),
                max_rows,
                timeout,
            })
            .await?)
    }

    /// Number of rows the query returns, across all pages.
    async fn count_rows(
        &self,
        body: &str,
        params: &[(String, Value)],
        timeout: Duration,
    ) -> Result<u64, ScriptError> {
        let counted = format!("SELECT COUNT(*) AS n FROM (\n{body}\n) AS portal_count");
        let result = self.query(&counted, params, 1, timeout).await?;
        Ok(match result.rows.first().and_then(|row| row.first()) {
            Some(Value::Int(n)) => u64::try_from(*n).unwrap_or(0),
            Some(Value::Float(f)) => f.max(0.0) as u64,
            Some(Value::Text(t)) => t.parse().unwrap_or(0),
            _ => 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::RecordingMetrics;
    use sql_portal_domain::scripts::{ParamDefinition, ParamKind};
    use sql_portal_domain::{ChartDefinition, ChartKind, Permission, QueryError};
    use sql_portal_domain::{PermissionSet, ScriptName};
    use sql_portal_testing::{FakeExecutor, InMemoryCatalog};

    fn script(name: &str, title: &str, perms: &[&str]) -> ScriptDefinition {
        let mut s =
            ScriptDefinition::new(ScriptName::new(name).unwrap(), title, "SELECT 1").unwrap();
        s.required_permissions = perms.iter().map(|p| Permission::new(*p).unwrap()).collect();
        s
    }

    fn principal(grants: &[&str]) -> Principal {
        let mut p = Principal::anonymous_admin();
        p.permissions = PermissionSet::new(grants.iter().copied());
        p
    }

    fn service(
        scripts: Vec<ScriptDefinition>,
        exec: FakeExecutor,
    ) -> (ScriptService, Arc<RecordingMetrics>, Arc<FakeExecutor>) {
        let t = Arc::new(RecordingMetrics::default());
        let exec = Arc::new(exec);
        (
            ScriptService::new(
                Arc::new(InMemoryCatalog::new(scripts)),
                exec.clone(),
                t.clone(),
                Arc::new(RequiredPermissions),
            ),
            t,
            exec,
        )
    }

    #[test]
    fn lists_only_permitted_scripts_sorted_by_title() {
        let (svc, _, _) = service(
            vec![
                script("b", "Zeta", &["a.read"]),
                script("a", "alpha", &["a.read"]),
                script("c", "Hidden", &["secret"]),
                script("d", "Open", &[]),
            ],
            FakeExecutor::default(),
        );
        let titles: Vec<_> = svc
            .list_available(&principal(&["a.read"]))
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(titles, vec!["alpha", "Open", "Zeta"]);
    }

    /// A replacement policy, as a dependent crate would supply.
    struct DenyEverything;

    impl ScriptAccessRule for DenyEverything {
        fn can_access(&self, _: &Principal, _: &ScriptDefinition) -> bool {
            false
        }
    }

    #[test]
    fn replacement_access_policy_is_used_for_listing_and_authorizing() {
        let svc = ScriptService::new(
            Arc::new(InMemoryCatalog::new(vec![script("a", "A", &[])])),
            Arc::new(FakeExecutor::default()),
            Arc::new(RecordingMetrics::default()),
            Arc::new(DenyEverything),
        );
        // The default policy would allow this principal; the replacement does not.
        assert!(svc.list_available(&principal(&["x"])).is_empty());
        assert!(matches!(
            svc.authorize(&principal(&["x"]), "a"),
            Err(ScriptError::Forbidden)
        ));
    }

    #[test]
    fn authorize_distinguishes_not_found_and_forbidden() {
        let (svc, _, _) = service(vec![script("a", "A", &["x"])], FakeExecutor::default());
        assert!(matches!(
            svc.authorize(&principal(&[]), "missing"),
            Err(ScriptError::NotFound)
        ));
        assert!(matches!(
            svc.authorize(&principal(&[]), "a"),
            Err(ScriptError::Forbidden)
        ));
        assert!(svc.authorize(&principal(&["x"]), "a").is_ok());
    }

    #[test]
    fn listing_and_authorizing_agree() {
        let (svc, _, _) = service(
            vec![
                script("a", "A", &["x"]),
                script("b", "B", &[]),
                script("c", "C", &["y"]),
            ],
            FakeExecutor::default(),
        );
        for grants in [&[][..], &["x"][..], &["x", "y"][..]] {
            let p = principal(grants);
            for name in ["a", "b", "c"] {
                let listed = svc.list_available(&p).iter().any(|s| s.name == name);
                let allowed = svc.authorize(&p, name).is_ok();
                assert_eq!(listed, allowed, "{name} with {grants:?}");
            }
        }
    }

    #[tokio::test]
    async fn run_executes_with_bound_params_and_builds_charts() {
        let mut s = script("a", "A", &["x"]);
        s.sql = "SELECT :n".into();
        let mut n = ParamDefinition::new("n", ParamKind::Integer);
        n.required = true;
        s.params = vec![n];
        s.charts = vec![ChartDefinition {
            title: "c".into(),
            kind: ChartKind::Bar,
            x: "k".into(),
            y: vec!["v".into()],
        }];
        s.validate().unwrap();

        let (svc, metrics, exec) = service(
            vec![s],
            FakeExecutor::returning(QueryResult {
                columns: vec!["k".into(), "v".into()],
                rows: vec![vec![Value::Text("a".into()), Value::Int(1)]],
                truncated: false,
            }),
        );
        let authorized = svc.authorize(&principal(&["x"]), "a").unwrap();
        let run = svc
            .run(
                authorized,
                &HashMap::from([("n".to_string(), "7".to_string())]),
                None,
            )
            .await
            .unwrap();

        assert_eq!(run.result.rows.len(), 1);
        assert!(run.charts[0].is_ok());
        let req = exec.last_request().unwrap();
        assert_eq!(req.params, vec![("n".to_string(), Value::Int(7))]);
        assert_eq!(
            metrics.script_events(),
            vec![("a".to_string(), "success".to_string(), 1)]
        );
    }

    #[tokio::test]
    async fn run_rejects_invalid_params_without_touching_database() {
        let mut s = script("a", "A", &[]);
        s.sql = "SELECT :n".into();
        let mut n = ParamDefinition::new("n", ParamKind::Integer);
        n.required = true;
        s.params = vec![n];
        let (svc, metrics, exec) = service(vec![s], FakeExecutor::default());

        let authorized = svc.authorize(&principal(&[]), "a").unwrap();
        let err = svc
            .run(authorized, &HashMap::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, ScriptError::InvalidParams(ref e) if e.len() == 1));
        assert!(exec.last_request().is_none());
        assert_eq!(metrics.script_events()[0].1, "invalid_params");
    }

    #[test]
    fn denied_access_is_recorded_and_never_authorized() {
        let (svc, metrics, exec) = service(vec![script("a", "A", &["x"])], FakeExecutor::default());
        let err = svc.authorize(&principal(&[]), "a").unwrap_err();
        assert!(matches!(err, ScriptError::Forbidden));
        assert!(exec.last_request().is_none());
        assert_eq!(metrics.script_events()[0].1, "forbidden");
    }

    #[test]
    fn unknown_scripts_use_bounded_metric_label() {
        let (svc, metrics, _) = service(vec![], FakeExecutor::default());
        let _ = svc.authorize(&principal(&[]), "whatever");
        assert_eq!(metrics.script_events()[0].0, "_unknown");
        assert_eq!(metrics.script_events()[0].1, "not_found");
    }

    #[tokio::test]
    async fn query_errors_are_propagated_and_classified() {
        let (svc, metrics, _) = service(
            vec![script("a", "A", &[])],
            FakeExecutor::failing(QueryError::Timeout),
        );
        let authorized = svc.authorize(&principal(&[]), "a").unwrap();
        let err = svc
            .run(authorized, &HashMap::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, ScriptError::Query(QueryError::Timeout)));
        assert_eq!(metrics.script_events()[0].1, "timeout");
    }

    #[tokio::test]
    async fn chart_errors_do_not_fail_the_run() {
        let mut s = script("a", "A", &[]);
        s.charts = vec![ChartDefinition {
            title: "c".into(),
            kind: ChartKind::Line,
            x: "missing".into(),
            y: vec!["v".into()],
        }];
        let (svc, _, _) = service(
            vec![s],
            FakeExecutor::returning(QueryResult {
                columns: vec!["k".into()],
                rows: vec![vec![Value::Int(1)]],
                truncated: false,
            }),
        );
        let authorized = svc.authorize(&principal(&[]), "a").unwrap();
        let run = svc.run(authorized, &HashMap::new(), None).await.unwrap();
        assert!(run.charts[0].is_err());
    }
}
