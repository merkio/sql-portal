//! ClickHouse dialect.
//!
//! * **Parameters** are typed server-side: `:name` becomes `{name:Type}` and the
//!   value travels in a `param_name` query argument, so values never appear in
//!   the SQL text. Text values are escaped the way the server reads them.
//! * **Results** come back as `JSONCompact`. Cells are decoded by the column's
//!   declared type: exact 64-bit integers, padded `FixedString`s, and every
//!   other type as text (dates, uuids, arrays, maps, tuples, enums...).
//! * **Setup scripts** are split into statements, since the HTTP interface
//!   accepts one statement per request.
//! * **Endpoints** `clickhouse://` (HTTP, port 8123) and `clickhouses://`
//!   (HTTPS, port 8443) are parsed here.

use sql_dialect::{lookup, scan_placeholders, Cell, Dialect, DialectError, Statement, Table};

#[derive(Debug, Clone, Copy, Default)]
pub struct ClickHouse;

impl Dialect for ClickHouse {
    fn name(&self) -> &'static str {
        "clickhouse"
    }

    fn prepare(
        &self,
        sql: &str,
        params: &[(String, Cell)],
        _param_types: Option<&[String]>,
    ) -> Result<Statement, DialectError> {
        let mut out = String::with_capacity(sql.len());
        let mut query_params: Vec<(String, String)> = Vec::new();
        let mut last = 0;
        for p in scan_placeholders(sql) {
            let value = lookup(params, &p.name)?;
            out.push_str(&sql[last..p.start]);
            out.push_str(&format!("{{{}:{}}}", p.name, server_type(value)));
            if !query_params
                .iter()
                .any(|(n, _)| n == &format!("param_{}", p.name))
            {
                query_params.push((format!("param_{}", p.name), param_text(value)));
            }
            last = p.end;
        }
        out.push_str(&sql[last..]);
        Ok(Statement {
            sql: out,
            bind: Vec::new(),
            query_params,
        })
    }
}

/// The server-side type of a parameter, from the value's kind.
pub fn server_type(value: &Cell) -> &'static str {
    match value {
        Cell::Null => "Nullable(String)",
        Cell::Bool(_) => "Bool",
        Cell::Int(_) => "Int64",
        Cell::Float(_) => "Float64",
        Cell::Text(_) => "String",
    }
}

/// Text form of a parameter, as the server parses it for its declared type.
pub fn param_text(value: &Cell) -> String {
    match value {
        // TSV-escaped NULL marker.
        Cell::Null => "\\N".into(),
        Cell::Bool(b) => b.to_string(),
        Cell::Int(i) => i.to_string(),
        Cell::Float(f) if f.is_nan() => "nan".into(),
        Cell::Float(f) if f.is_infinite() => if *f > 0.0 { "inf" } else { "-inf" }.into(),
        Cell::Float(f) => f.to_string(),
        Cell::Text(s) => escape_text(s),
    }
}

/// Escapes a string the way ClickHouse's escaped text format reads it.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}

/// Splits a setup script into statements on `;`, ignoring quoted text and
/// comments. Comment-only fragments are dropped.
pub fn split_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut has_code = false;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' => {
                has_code = true;
                current.push(c);
                while let Some(d) = chars.next() {
                    current.push(d);
                    if d == '\\' && c != '`' {
                        if let Some(e) = chars.next() {
                            current.push(e);
                        }
                    } else if d == c {
                        if chars.peek() == Some(&c) {
                            current.extend(chars.next());
                        } else {
                            break;
                        }
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                current.push(c);
                for d in chars.by_ref() {
                    current.push(d);
                    if d == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                current.push(c);
                current.extend(chars.next());
                let mut prev = '\0';
                for d in chars.by_ref() {
                    current.push(d);
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
            }
            ';' => {
                if has_code {
                    statements.push(current.trim().to_string());
                }
                current.clear();
                has_code = false;
            }
            c => {
                if !c.is_whitespace() {
                    has_code = true;
                }
                current.push(c);
            }
        }
    }
    if has_code {
        statements.push(current.trim().to_string());
    }
    statements
}

/// Parses a `JSONCompact` body. An empty body (no result set) is an empty table.
pub fn parse_result(body: &str) -> Result<Table, DialectError> {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Envelope {
        meta: Vec<ColumnMeta>,
        data: Vec<Vec<serde_json::Value>>,
    }
    #[derive(Deserialize)]
    struct ColumnMeta {
        name: String,
        #[serde(rename = "type")]
        ty: String,
    }

    if body.trim().is_empty() {
        return Ok(Table::default());
    }
    let envelope: Envelope = serde_json::from_str(body)
        .map_err(|e| DialectError(format!("unexpected ClickHouse response: {e}")))?;
    let types: Vec<&str> = envelope.meta.iter().map(|c| c.ty.as_str()).collect();
    let rows = envelope
        .data
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(i, cell)| decode_cell(types.get(i).copied().unwrap_or(""), cell))
                .collect()
        })
        .collect();
    Ok(Table {
        columns: envelope.meta.into_iter().map(|c| c.name).collect(),
        rows,
    })
}

/// Decodes one JSON cell according to its declared ClickHouse type.
pub fn decode_cell(ty: &str, cell: &serde_json::Value) -> Cell {
    use serde_json::Value as Json;

    let base = ty
        .strip_prefix("Nullable(")
        .and_then(|t| t.strip_suffix(')'))
        .unwrap_or(ty);
    let integer = base.starts_with("Int") || base.starts_with("UInt");
    let float = base.starts_with("Float");

    match cell {
        Json::Null => Cell::Null,
        Json::Bool(b) => Cell::Bool(*b),
        // FixedString pads with NUL bytes.
        Json::String(s) if base.starts_with("FixedString") => {
            Cell::Text(s.trim_end_matches('\0').to_string())
        }
        // 64-bit integers are quoted by default; values beyond i64 stay text.
        Json::String(s) if integer => s
            .parse::<i64>()
            .map(Cell::Int)
            .unwrap_or_else(|_| Cell::Text(s.clone())),
        Json::String(s) if float => s
            .parse::<f64>()
            .map(Cell::Float)
            .unwrap_or_else(|_| Cell::Text(s.clone())),
        Json::String(s) => Cell::Text(s.clone()),
        Json::Number(n) if float => n
            .as_f64()
            .map(Cell::Float)
            .unwrap_or_else(|| Cell::Text(n.to_string())),
        // Integers beyond i64 stay exact as text; a float would round them.
        Json::Number(n) if integer => n
            .as_i64()
            .map(Cell::Int)
            .unwrap_or_else(|| Cell::Text(n.to_string())),
        Json::Number(n) => n
            .as_i64()
            .map(Cell::Int)
            .or_else(|| n.as_f64().map(Cell::Float))
            .unwrap_or_else(|| Cell::Text(n.to_string())),
        other => Cell::Text(other.to_string()),
    }
}

/// A parsed `clickhouse://` or `clickhouses://` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `http://host:port/` or `https://host:port/`.
    pub base: String,
    pub database: String,
    pub user: String,
    pub password: Option<String>,
}

impl Endpoint {
    /// Parses `clickhouse://[user[:password]@]host[:port][/database]`, or the
    /// `clickhouses://` form for HTTPS.
    pub fn parse(url: &str) -> Result<Endpoint, DialectError> {
        use percent_encoding::percent_decode_str;

        let parsed = url::Url::parse(url)
            .map_err(|e| DialectError(format!("invalid ClickHouse URL: {e}")))?;
        let (scheme, default_port) = match parsed.scheme() {
            "clickhouse" => ("http", 8123),
            "clickhouses" => ("https", 8443),
            other => {
                return Err(DialectError(format!(
                    "unsupported ClickHouse URL scheme '{other}'"
                )))
            }
        };
        let host = parsed
            .host_str()
            .ok_or_else(|| DialectError("ClickHouse URL needs a host".into()))?;
        let port = parsed.port().unwrap_or(default_port);
        let decode = |s: &str| percent_decode_str(s).decode_utf8_lossy().into_owned();

        Ok(Endpoint {
            base: format!("{scheme}://{host}:{port}/"),
            database: match parsed.path().trim_matches('/') {
                "" => "default".to_string(),
                db => decode(db),
            },
            user: match parsed.username() {
                "" => "default".to_string(),
                u => decode(u),
            },
            password: parsed.password().map(decode),
        })
    }

    pub fn is_https(&self) -> bool {
        self.base.starts_with("https://")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(pairs: &[(&str, Cell)]) -> Vec<(String, Cell)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn placeholders_become_typed_server_parameters() {
        let params = named(&[
            ("n", Cell::Int(7)),
            ("name", Cell::Text("a'b".into())),
            ("flag", Cell::Bool(true)),
            ("gone", Cell::Null),
        ]);
        let st = ClickHouse
            .prepare(
                "SELECT :n, :name, :n, :flag, :gone -- :ignored",
                &params,
                None,
            )
            .unwrap();
        assert_eq!(
            st.sql,
            "SELECT {n:Int64}, {name:String}, {n:Int64}, {flag:Bool}, {gone:Nullable(String)} -- :ignored"
        );
        assert_eq!(st.query_params.len(), 4);
        assert!(!st.sql.contains("a'b"), "values never enter the SQL text");
        assert!(st.bind.is_empty());
    }

    #[test]
    fn missing_values_are_errors() {
        assert!(ClickHouse.prepare("SELECT :x", &[], None).is_err());
    }

    #[test]
    fn text_parameters_are_escaped_and_special_floats_named() {
        assert_eq!(param_text(&Cell::Text("C:\\t\ty".into())), "C:\\\\t\\ty");
        assert_eq!(param_text(&Cell::Text("\\N".into())), "\\\\N");
        assert_eq!(param_text(&Cell::Null), "\\N");
        assert_eq!(param_text(&Cell::Float(f64::NAN)), "nan");
        assert_eq!(param_text(&Cell::Float(f64::NEG_INFINITY)), "-inf");
    }

    #[test]
    fn statements_split_outside_quotes_and_comments() {
        let sql = "CREATE TABLE t (s String) ENGINE = Memory;\n\
                   -- a ; in a comment\n\
                   INSERT INTO t VALUES ('a;b'), ('it\\'s; fine');\n\
                   /* block ; */ INSERT INTO t VALUES ('x');\n\
                   -- trailing comment only\n";
        let parts = split_statements(sql);
        assert_eq!(parts.len(), 3, "{parts:#?}");
        assert!(parts[0].starts_with("CREATE TABLE"));
        assert!(parts[1].contains("'a;b'") && parts[1].contains("it\\'s; fine"));
    }

    #[test]
    fn json_compact_cells_decode_by_type() {
        let body = r#"{
            "meta": [
                {"name": "id", "type": "UInt64"},
                {"name": "price", "type": "Float64"},
                {"name": "note", "type": "Nullable(String)"},
                {"name": "ok", "type": "Bool"},
                {"name": "day", "type": "Date"}
            ],
            "data": [
                ["18446744073709551615", 1, null, true, "2024-01-02"],
                ["2", 2.5, "x", false, "2024-01-03"]
            ]
        }"#;
        let t = parse_result(body).unwrap();
        assert_eq!(t.columns, vec!["id", "price", "note", "ok", "day"]);
        assert_eq!(t.rows[0][0], Cell::Text("18446744073709551615".into()));
        assert_eq!(t.rows[0][1], Cell::Float(1.0));
        assert_eq!(t.rows[0][2], Cell::Null);
        assert_eq!(t.rows[1][0], Cell::Int(2));
        assert_eq!(t.rows[1][4], Cell::Text("2024-01-03".into()));
    }

    #[test]
    fn fixed_strings_drop_padding_and_empty_body_is_empty() {
        let body =
            r#"{"meta":[{"name":"fs","type":"FixedString(4)"}],"data":[["ab\u0000\u0000"]]}"#;
        assert_eq!(
            parse_result(body).unwrap().rows[0][0],
            Cell::Text("ab".into())
        );
        assert_eq!(parse_result("").unwrap(), Table::default());
    }

    #[test]
    fn endpoints_apply_defaults_and_decode_credentials() {
        let e = Endpoint::parse("clickhouse://bob:p%40ss@ch.local/analytics").unwrap();
        assert_eq!(e.base, "http://ch.local:8123/");
        assert_eq!(e.database, "analytics");
        assert_eq!(e.user, "bob");
        assert_eq!(e.password.as_deref(), Some("p@ss"));
        assert!(!e.is_https());

        let tls = Endpoint::parse("clickhouses://ch.local").unwrap();
        assert_eq!(tls.base, "https://ch.local:8443/");
        assert_eq!(tls.database, "default");
        assert_eq!(tls.user, "default");
        assert!(tls.is_https());

        assert!(Endpoint::parse("postgres://x").is_err());
    }
}
