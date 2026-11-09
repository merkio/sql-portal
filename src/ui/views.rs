//! HTML views. Pages are rendered from the templates in `templates/`; every
//! value goes through autoescaping. JSON embedded in `<script
//! type="application/json">` blocks goes through [`json_for_script`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;
use serde_json::json;

use super::render::render;
use super::state::UiSettings;
use crate::application::{ScriptRun, ScriptSummary};
use crate::domain::{
    ChartData, DatasetData, FieldError, ParamDefinition, ParamKind, ScriptDefinition, Value,
};

/// HTML-escapes text for element content and attribute values.
pub fn esc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Serialises JSON so it can never terminate the surrounding `<script>` block.
pub fn json_for_script(value: &serde_json::Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Navigation state for the page header.
pub struct Nav {
    pub display_name: String,
    /// `Some` renders a logout button using this CSRF token.
    pub logout_csrf: Option<String>,
}

#[derive(Serialize)]
struct NavContext<'a> {
    display_name: &'a str,
    logout_csrf: Option<&'a str>,
}

#[derive(Serialize)]
struct LayoutContext<'a> {
    title: &'a str,
    asset_version: &'static str,
    ui_title: &'a str,
    nav: Option<NavContext<'a>>,
    body: &'a str,
    with_charts: bool,
    chartjs_url: &'a str,
}

/// A complete page around a rendered body.
pub fn page(
    ui: &UiSettings,
    title: &str,
    nav: Option<&Nav>,
    body: &str,
    with_charts: bool,
) -> String {
    render(
        "layout.html",
        LayoutContext {
            title,
            asset_version: super::render::asset_version(),
            ui_title: &ui.title,
            nav: nav.map(|n| NavContext {
                display_name: &n.display_name,
                logout_csrf: n.logout_csrf.as_deref(),
            }),
            body,
            with_charts,
            // Config-controlled: escaped by `esc` (which keeps `/` readable) and
            // inserted as-is.
            chartjs_url: &esc(&ui.chartjs_url),
        },
    )
}

#[derive(Serialize)]
struct ScriptLink<'a> {
    name: &'a str,
    title: &'a str,
    description: &'a str,
    accessible: bool,
    required_permissions: &'a [String],
}

#[derive(Serialize)]
struct GroupContext<'a> {
    /// `None` when the list has no groups at all: then no heading is shown.
    name: Option<&'a str>,
    scripts: Vec<ScriptLink<'a>>,
}

#[derive(Serialize)]
struct IndexContext<'a> {
    total: usize,
    groups: Vec<GroupContext<'a>>,
    show_all: bool,
    /// Locked scripts for this user, whether or not they are listed.
    locked: usize,
}

/// Groups in alphabetical order, ungrouped scripts last. Within a group the
/// order of `scripts` (by title) is kept.
fn group_scripts(scripts: &[ScriptSummary]) -> Vec<GroupContext<'_>> {
    fn link(s: &ScriptSummary) -> ScriptLink<'_> {
        ScriptLink {
            name: &s.name,
            title: &s.title,
            description: &s.description,
            accessible: s.accessible,
            required_permissions: &s.required_permissions,
        }
    }
    // Keyed by the lower-cased name, so "sales" and "Sales" are one group.
    let mut named: BTreeMap<String, GroupContext<'_>> = BTreeMap::new();
    let mut ungrouped = Vec::new();
    for s in scripts {
        match s.group.as_deref() {
            Some(group) => named
                .entry(group.to_lowercase())
                .or_insert_with(|| GroupContext {
                    name: Some(group),
                    scripts: Vec::new(),
                })
                .scripts
                .push(link(s)),
            None => ungrouped.push(link(s)),
        }
    }
    let has_named = !named.is_empty();
    let mut groups: Vec<GroupContext<'_>> = named.into_values().collect();
    if !ungrouped.is_empty() {
        groups.push(GroupContext {
            name: has_named.then_some("Other scripts"),
            scripts: ungrouped,
        });
    }
    groups
}

/// The start page. `scripts` is the list to show: every script when `show_all`,
/// otherwise only the accessible ones. `locked` counts the locked scripts.
pub fn index_body(scripts: &[ScriptSummary], show_all: bool, locked: usize) -> String {
    render(
        "index.html",
        IndexContext {
            total: scripts.len(),
            groups: group_scripts(scripts),
            show_all,
            locked,
        },
    )
}

#[derive(Serialize)]
struct LoginContext<'a> {
    ldap: bool,
    oidc: bool,
    github: bool,
    csrf: &'a str,
    error: Option<&'a str>,
}

pub fn login_body(ldap: bool, oidc: bool, github: bool, csrf: &str, error: Option<&str>) -> String {
    render(
        "login.html",
        LoginContext {
            ldap,
            oidc,
            github,
            csrf,
            error,
        },
    )
}

#[derive(Serialize)]
struct MessageContext<'a> {
    title: &'a str,
    message: &'a str,
}

pub fn message_body(title: &str, message: &str) -> String {
    render("message.html", MessageContext { title, message })
}

#[derive(Serialize)]
struct OptionContext<'a> {
    value: &'a str,
    label: &'a str,
    selected: bool,
}

#[derive(Serialize)]
struct FieldContext<'a> {
    id: String,
    name: &'a str,
    label: &'a str,
    kind: &'static str,
    value: String,
    checked: bool,
    required: bool,
    min: Option<String>,
    max: Option<String>,
    max_length: String,
    help: Option<&'a str>,
    error: Option<&'a str>,
    invalid: bool,
    options: Vec<OptionContext<'a>>,
}

fn field_context<'a>(
    p: &'a ParamDefinition,
    value: Option<&str>,
    error: Option<&'a FieldError>,
) -> FieldContext<'a> {
    let current = value
        .map(str::to_owned)
        .or_else(|| p.default.clone())
        .unwrap_or_default();
    let kind = match p.kind {
        ParamKind::Text => "text",
        ParamKind::Integer => "integer",
        ParamKind::Number => "number",
        ParamKind::Boolean => "boolean",
        ParamKind::Date => "date",
        ParamKind::Select => "select",
    };
    FieldContext {
        id: format!("p-{}", p.name),
        name: &p.name,
        label: &p.label,
        kind,
        checked: p.kind == ParamKind::Boolean
            && matches!(
                current.to_ascii_lowercase().as_str(),
                "true" | "1" | "on" | "yes"
            ),
        value: current.clone(),
        required: p.required,
        min: p.min.map(|m| m.to_string()),
        max: p.max.map(|m| m.to_string()),
        max_length: p.max_length.to_string(),
        help: p.help.as_deref(),
        error: error.map(|e| e.message.as_str()),
        invalid: error.is_some(),
        options: p
            .options
            .iter()
            .map(|o| OptionContext {
                value: &o.value,
                label: &o.label,
                selected: o.value == current,
            })
            .collect(),
    }
}

/// Chart.js configuration for a prepared chart.
pub fn chart_config(chart: &ChartData) -> serde_json::Value {
    let datasets: Vec<_> = chart
        .datasets
        .iter()
        .map(|d| match &d.data {
            DatasetData::Values(v) => json!({"label": d.label, "data": v}),
            DatasetData::Points(p) => json!({
                "label": d.label,
                "data": p.iter().map(|(x, y)| json!({"x": x, "y": y})).collect::<Vec<_>>()
            }),
        })
        .collect();
    json!({
        "type": chart.kind.as_str(),
        "data": {"labels": chart.labels, "datasets": datasets},
        "options": {"plugins": {"title": {"display": true, "text": chart.title}}}
    })
}

#[derive(Serialize)]
struct CellContext {
    text: String,
    numeric: bool,
}

#[derive(Serialize)]
struct ChartContext {
    index: usize,
    title: String,
    /// Already safe for a `<script>` block (see [`json_for_script`]).
    json: String,
    error: Option<String>,
}

#[derive(Serialize)]
struct RunContext<'a> {
    meta: String,
    /// What the charts or the table leave out, in words; `None` when nothing is left out.
    notice: Option<String>,
    pager: Option<PagerContext<'a>>,
    charts: Vec<ChartContext>,
    show_table: bool,
    empty: bool,
    columns: Vec<String>,
    rows: Vec<Vec<CellContext>>,
}

#[derive(Serialize)]
struct PagerContext<'a> {
    first_row: u64,
    last_row: u64,
    total: u64,
    current: usize,
    pages: usize,
    prev: Option<String>,
    next: Option<String>,
    links: Vec<PageLinkContext>,
    /// Rows per page in effect, and the choices offered for it.
    size: usize,
    size_options: Vec<SizeOptionContext>,
    /// Form values to keep when the page size changes (the page resets to 1).
    hidden_fields: Vec<HiddenFieldContext<'a>>,
}

#[derive(Serialize)]
struct PageLinkContext {
    /// `None` marks a gap ("…") between shown page numbers.
    number: Option<usize>,
    href: String,
    current: bool,
    /// Not next to the active page; hidden on narrow screens.
    far: bool,
}

/// Query string that keeps the form values and selects `page`.
fn page_href(values: &BTreeMap<&str, &str>, page: usize) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in values.iter().filter(|(name, _)| !name.starts_with('_')) {
        query.append_pair(name, value);
    }
    query.append_pair("_run", "1");
    query.append_pair("_page", &page.to_string());
    if let Some(size) = values.get("_size") {
        query.append_pair("_size", size);
    }
    format!("?{}", query.finish())
}

/// Rows per page offered in the selector.
const PAGE_SIZE_CHOICES: [usize; 5] = [5, 10, 25, 50, 100];

#[derive(Serialize)]
struct SizeOptionContext {
    value: usize,
    selected: bool,
}

#[derive(Serialize)]
struct HiddenFieldContext<'a> {
    name: &'a str,
    value: &'a str,
}

/// The rows-per-page choice shown above the table. The script's own page size
/// is always among the choices, so the default can be selected.
fn size_options(current: usize, default: usize) -> Vec<SizeOptionContext> {
    let mut values: BTreeSet<usize> = PAGE_SIZE_CHOICES.into_iter().collect();
    values.insert(default);
    values
        .into_iter()
        .map(|value| SizeOptionContext {
            value,
            selected: value == current,
        })
        .collect()
}

/// Page numbers to link to: the first, the last, and a window around the
/// current page. Gaps between them are `None`.
fn page_numbers(current: usize, pages: usize) -> Vec<Option<usize>> {
    let mut shown: BTreeSet<usize> = BTreeSet::from([1, pages]);
    shown.extend(current.saturating_sub(2).max(1)..=(current + 2).min(pages));
    let mut out = Vec::new();
    let mut previous: Option<usize> = None;
    for n in shown {
        if previous.is_some_and(|p| n > p + 1) {
            out.push(None);
        }
        out.push(Some(n));
        previous = Some(n);
    }
    out
}

fn pager_context<'a>(
    run: &ScriptRun,
    values: &BTreeMap<&'a str, &'a str>,
) -> Option<PagerContext<'a>> {
    let info = run.page?;
    let href = |page: usize| page_href(values, page);
    let hidden_fields = values
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .map(|(name, value)| HiddenFieldContext { name, value })
        .collect();
    Some(PagerContext {
        size: info.size,
        size_options: size_options(info.size, run.script.page_size),
        hidden_fields,
        first_row: info.first_row(),
        last_row: info.last_row(),
        total: info.total,
        current: info.number,
        pages: info.pages,
        prev: (info.number > 1).then(|| href(info.number - 1)),
        next: (info.number < info.pages).then(|| href(info.number + 1)),
        links: page_numbers(info.number, info.pages)
            .into_iter()
            .map(|number| PageLinkContext {
                number,
                href: number.map(href).unwrap_or_default(),
                current: number == Some(info.number),
                far: number.is_none_or(|n| n.abs_diff(info.number) > 1),
            })
            .collect(),
    })
}

fn run_context<'a>(run: &ScriptRun, values: &BTreeMap<&'a str, &'a str>) -> RunContext<'a> {
    let rows = run.result.rows.len();
    let charts = run
        .charts
        .iter()
        .enumerate()
        .map(|(index, chart)| match chart {
            Ok(data) => ChartContext {
                index,
                title: data.title.clone(),
                json: json_for_script(&chart_config(data)),
                error: None,
            },
            Err(message) => ChartContext {
                index,
                title: String::new(),
                json: String::new(),
                error: Some(message.clone()),
            },
        })
        .collect();
    let meta = match run.page {
        Some(info) if info.total > 0 => format!(
            "{rows} row{} in {} ms · rows {}–{} of {}",
            if rows == 1 { "" } else { "s" },
            run.duration.as_millis(),
            info.first_row(),
            info.last_row(),
            info.total
        ),
        _ => format!(
            "{rows} row{} in {} ms",
            if rows == 1 { "" } else { "s" },
            run.duration.as_millis()
        ),
    };
    // Charts are capped at max_rows; an unpaged table is capped the same way.
    let notice = match (run.capped, run.page) {
        (false, _) => None,
        (true, Some(_)) => Some(format!("charts use the first {} rows", run.script.max_rows)),
        (true, None) => Some(format!(
            "truncated to the first {} rows",
            run.script.max_rows
        )),
    };
    RunContext {
        meta,
        notice,
        pager: pager_context(run, values),
        charts,
        show_table: run.script.show_table,
        empty: run.result.rows.is_empty(),
        columns: run.result.columns.clone(),
        rows: run
            .result
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|v| CellContext {
                        text: v.display(),
                        numeric: matches!(v, Value::Int(_) | Value::Float(_)),
                    })
                    .collect()
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct ScriptContext<'a> {
    title: &'a str,
    description: Option<&'a str>,
    error: Option<&'a str>,
    has_params: bool,
    name: &'a str,
    fields: Vec<FieldContext<'a>>,
    /// Relative link to this page with its parameters, for sharing.
    share_href: String,
    run: Option<RunContext<'a>>,
}

/// Link to the page with the given values; once run, the page and size too,
/// so the recipient sees the same rows.
fn share_href(values: &BTreeMap<&str, &str>, run: Option<&ScriptRun>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in values.iter().filter(|(name, _)| !name.starts_with('_')) {
        query.append_pair(name, value);
    }
    if let Some(run) = run {
        query.append_pair("_run", "1");
        if let Some(info) = run.page {
            query.append_pair("_page", &info.number.to_string());
            query.append_pair("_size", &info.size.to_string());
        }
    }
    format!("?{}", query.finish())
}

/// Script page: the parameter form and, when available, the results.
pub fn script_body(
    script: &ScriptDefinition,
    values: &HashMap<String, String>,
    field_errors: &[FieldError],
    run: Option<&ScriptRun>,
    error: Option<&str>,
) -> String {
    let sorted_values: BTreeMap<&str, &str> = values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    render(
        "script.html",
        ScriptContext {
            title: &script.title,
            description: Some(script.description.as_str()).filter(|d| !d.is_empty()),
            error,
            has_params: !script.params.is_empty(),
            name: script.name.as_str(),
            fields: script
                .params
                .iter()
                .map(|p| {
                    field_context(
                        p,
                        values.get(&p.name).map(String::as_str),
                        field_errors.iter().find(|e| e.param == p.name),
                    )
                })
                .collect(),
            share_href: share_href(&sorted_values, run),
            run: run.map(|r| run_context(r, &sorted_values)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ChartDefinition, ChartKind, QueryResult, ScriptName, SelectOption};
    use std::sync::Arc;
    use std::time::Duration;

    fn ui() -> UiSettings {
        UiSettings {
            title: "Portal <&>".into(),
            chartjs_url: "https://cdn.example.org/chart.js".into(),
        }
    }

    #[test]
    fn escapes_html() {
        assert_eq!(
            esc(r#"<a href="x">'&"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;"
        );
    }

    #[test]
    fn json_cannot_break_out_of_script_blocks() {
        let s = json_for_script(&json!({"x": "</script><script>alert(1)</script>&\u{2028}"}));
        assert!(!s.contains("</script>"));
        assert!(!s.contains('<'));
        assert!(s.contains("\\u003c/script\\u003e"));
        assert!(s.contains("\\u2028"));
    }

    #[test]
    fn page_escapes_title_and_includes_chart_script_on_demand() {
        let html = page(&ui(), "T<1>", None, "<p>x</p>", true);
        assert!(html.contains("T&lt;1&gt; · Portal &lt;&amp;&gt;"));
        assert!(html.contains("https://cdn.example.org/chart.js"));
        let html = page(&ui(), "T", None, "", false);
        assert!(!html.contains("cdn.example.org"));
    }

    #[test]
    fn nav_shows_logout_only_with_csrf() {
        let with = page(
            &ui(),
            "T",
            Some(&Nav {
                display_name: "Ann".into(),
                logout_csrf: Some("tok".into()),
            }),
            "",
            false,
        );
        assert!(with.contains("action=\"/logout\"") && with.contains("value=\"tok\""));
        let without = page(
            &ui(),
            "T",
            Some(&Nav {
                display_name: "Ann".into(),
                logout_csrf: None,
            }),
            "",
            false,
        );
        assert!(!without.contains("/logout"));
    }

    #[test]
    fn index_lists_scripts_or_explains_emptiness() {
        let html = index_body(
            &[ScriptSummary {
                name: "a".into(),
                title: "A <b>".into(),
                description: "d".into(),
                group: None,
                accessible: true,
                required_permissions: vec![],
            }],
            false,
            0,
        );
        assert!(html.contains("href=\"/scripts/a\"") && html.contains("A &lt;b&gt;"));
        assert!(!html.contains("group-title"), "no groups, no headings");
        assert!(index_body(&[], false, 0).contains("no scripts you can run"));
    }

    #[test]
    fn scripts_are_grouped_alphabetically_with_ungrouped_last() {
        let summary = |name: &str, group: Option<&str>| ScriptSummary {
            name: name.into(),
            title: name.into(),
            description: String::new(),
            group: group.map(str::to_string),
            accessible: true,
            required_permissions: vec![],
        };
        let scripts = [
            summary("a", Some("Sales")),
            summary("b", None),
            summary("c", Some("charts")),
            summary("d", Some("Charts")),
        ];
        let groups = group_scripts(&scripts);
        let names: Vec<_> = groups.iter().map(|g| g.name).collect();
        assert_eq!(
            names,
            vec![Some("charts"), Some("Sales"), Some("Other scripts")]
        );
        assert_eq!(groups[0].scripts.len(), 2, "case-insensitive grouping");

        let html = index_body(&[summary("a", Some("Sales")), summary("b", None)], false, 0);
        assert!(html.contains("<h2 class=\"group-title\">Sales"));
        assert!(html.contains("Other scripts"));
    }

    #[test]
    fn login_page_variants() {
        let both = login_body(true, true, false, "tok", None);
        assert!(
            both.contains("name=\"username\"")
                && both.contains("/login/oidc")
                && both.contains("tok")
        );
        let ldap_only = login_body(true, false, false, "t", Some("bad <x>"));
        assert!(!ldap_only.contains("/login/oidc") && ldap_only.contains("bad &lt;x&gt;"));
        assert!(login_body(false, false, false, "t", None).contains("No sign-in method"));
        let github_only = login_body(false, false, true, "t", None);
        assert!(
            github_only.contains("href=\"/login/github\"")
                && !github_only.contains("No sign-in method")
        );
    }

    fn script_with_params() -> ScriptDefinition {
        let mut s = ScriptDefinition::new_unchecked(
            ScriptName::new("s").unwrap(),
            "Title",
            "SELECT :t, :i, :n, :b, :d, :sel",
        );
        let mut t = ParamDefinition::new("t", ParamKind::Text);
        t.label = "Text".into();
        t.help = Some("Helpful".into());
        let mut i = ParamDefinition::new("i", ParamKind::Integer);
        i.min = Some(1.0);
        i.max = Some(9.0);
        let n = ParamDefinition::new("n", ParamKind::Number);
        let mut b = ParamDefinition::new("b", ParamKind::Boolean);
        b.default = Some("true".into());
        let d = ParamDefinition::new("d", ParamKind::Date);
        let mut sel = ParamDefinition::new("sel", ParamKind::Select);
        sel.options = vec![
            SelectOption {
                value: "a".into(),
                label: "A".into(),
            },
            SelectOption {
                value: "b".into(),
                label: "B".into(),
            },
        ];
        s.params = vec![t, i, n, b, d, sel];
        s.validate().unwrap();
        s
    }

    #[test]
    fn renders_every_input_kind() {
        let html = script_body(&script_with_params(), &HashMap::new(), &[], None, None);
        assert!(html.contains("type=\"text\""));
        assert!(
            html.contains("type=\"number\" step=\"1\"")
                && html.contains("min=\"1\"")
                && html.contains("max=\"9\"")
        );
        assert!(html.contains("step=\"any\""));
        assert!(html.contains("type=\"checkbox\"") && html.contains(" checked"));
        assert!(html.contains("type=\"date\""));
        assert!(html.contains("<select") && html.contains("<option value=\"a\""));
        assert!(html.contains("Helpful"));
        assert!(html.contains("name=\"_run\""));
    }

    #[test]
    fn preserves_values_and_shows_field_errors() {
        let values = HashMap::from([
            ("t".to_string(), "<script>".to_string()),
            ("sel".to_string(), "b".to_string()),
        ]);
        let errors = vec![FieldError {
            param: "i".into(),
            message: "must be at most 9".into(),
        }];
        let html = script_body(
            &script_with_params(),
            &values,
            &errors,
            None,
            Some("Fix errors"),
        );
        assert!(html.contains("value=\"&lt;script&gt;\""));
        assert!(html.contains("<option value=\"b\" selected>"));
        assert!(html.contains("must be at most 9") && html.contains("field invalid"));
        assert!(html.contains("Fix errors"));
    }

    fn run_with(
        charts: Vec<Result<ChartData, String>>,
        rows: Vec<Vec<Value>>,
        show_table: bool,
    ) -> ScriptRun {
        let mut s = ScriptDefinition::new(ScriptName::new("s").unwrap(), "T", "SELECT 1").unwrap();
        s.show_table = show_table;
        s.charts = vec![ChartDefinition {
            title: "c".into(),
            kind: ChartKind::Bar,
            x: "k".into(),
            y: vec!["v".into()],
        }];
        ScriptRun {
            script: Arc::new(s),
            params: vec![],
            result: QueryResult {
                columns: vec!["k".into(), "v".into()],
                rows,
                truncated: true,
            },
            page: None,
            capped: true,
            charts,
            duration: Duration::from_millis(12),
        }
    }

    #[test]
    fn renders_table_charts_and_meta() {
        let chart = ChartDefinition {
            title: "Bars </script>".into(),
            kind: ChartKind::Bar,
            x: "k".into(),
            y: vec!["v".into()],
        }
        .build(&QueryResult {
            columns: vec!["k".into(), "v".into()],
            rows: vec![vec![Value::Text("<x>".into()), Value::Int(3)]],
            truncated: false,
        })
        .unwrap();
        let run = run_with(
            vec![Ok(chart), Err("column 'z' not found".into())],
            vec![vec![Value::Text("<x>".into()), Value::Int(3)]],
            true,
        );
        let html = script_body(&run.script, &HashMap::new(), &[], Some(&run), None);
        assert!(html.contains("1 row in 12 ms"));
        assert!(html.contains("truncated to the first 1000 rows"));
        assert!(html.contains("<td>&lt;x&gt;</td>") && html.contains("<td class=\"num\">3</td>"));
        assert!(html.contains("data-chart=\"chart-data-0\""));
        assert!(
            !html.contains("</script>Bars"),
            "chart title must not break out"
        );
        assert!(html.contains("Chart unavailable: column &#x27;z&#x27; not found"));
    }

    #[test]
    fn empty_result_and_hidden_table() {
        let run = run_with(vec![], vec![], true);
        assert!(
            script_body(&run.script, &HashMap::new(), &[], Some(&run), None)
                .contains("returned no rows")
        );
        let run = run_with(vec![], vec![vec![Value::Null, Value::Null]], false);
        assert!(
            !script_body(&run.script, &HashMap::new(), &[], Some(&run), None).contains("<table>")
        );
    }

    #[test]
    fn chart_config_shapes() {
        use crate::domain::Dataset;
        let bar = chart_config(&ChartData {
            title: "t".into(),
            kind: ChartKind::Bar,
            labels: vec!["a".into()],
            datasets: vec![Dataset {
                label: "v".into(),
                data: DatasetData::Values(vec![Some(1.0), None]),
            }],
        });
        assert_eq!(bar["type"], "bar");
        assert_eq!(bar["data"]["datasets"][0]["data"], json!([1.0, null]));
        let sc = chart_config(&ChartData {
            title: "t".into(),
            kind: ChartKind::Scatter,
            labels: vec![],
            datasets: vec![Dataset {
                label: "v".into(),
                data: DatasetData::Points(vec![(1.0, 2.0)]),
            }],
        });
        assert_eq!(
            sc["data"]["datasets"][0]["data"][0],
            json!({"x": 1.0, "y": 2.0})
        );
    }
}
