//! PostgreSQL dialect.
//!
//! Three rules make typed values work over the wire:
//!
//! * **Parameters.** When the server declared a parameter's type and the type
//!   has a cast here, the placeholder becomes `CAST($n AS type)` and the value
//!   is bound natively. Otherwise the server types the parameter as text and
//!   the value is bound in its text form.
//! * **Results.** [`wrap_result`] puts the statement in a subquery and converts
//!   each column: booleans, integers and floats stay native; numerics become
//!   text, parsed back into numbers by [`numeric_cell`]; every other type
//!   (dates, timestamps, uuids, json, arrays, bytea, intervals...) becomes its
//!   text form. Columns are renamed by position, so duplicate or unnamed
//!   result columns stay addressable.
//! * **Statements.** A trailing semicolon is removed, since the wrapper cannot
//!   contain it.

use sql_executor::{lookup, scan_placeholders, Cell, Dialect, DialectError, Statement};

#[derive(Debug, Clone, Copy, Default)]
pub struct Postgres;

impl Dialect for Postgres {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn prepare(
        &self,
        sql: &str,
        params: &[(String, Cell)],
        param_types: Option<&[String]>,
    ) -> Result<Statement, DialectError> {
        let sql = sql.trim().trim_end_matches(';').trim_end();
        let mut out = String::with_capacity(sql.len());
        let mut names: Vec<String> = Vec::new();
        let mut casted: Vec<bool> = Vec::new();
        let mut last = 0;

        for p in scan_placeholders(sql) {
            out.push_str(&sql[last..p.start]);
            let index = match names.iter().position(|n| n == &p.name) {
                Some(i) => i + 1,
                None => {
                    let cast = param_types
                        .and_then(|types| types.get(names.len()))
                        .and_then(|t| cast_type(t));
                    names.push(p.name.clone());
                    casted.push(cast.is_some());
                    match cast {
                        Some(ty) => out.push_str(&format!("CAST(${} AS {ty})", names.len())),
                        None => out.push_str(&format!("${}", names.len())),
                    }
                    last = p.end;
                    continue;
                }
            };
            out.push_str(&format!("${index}"));
            last = p.end;
        }
        out.push_str(&sql[last..]);

        let mut bind = Vec::with_capacity(names.len());
        for (name, cast) in names.iter().zip(&casted) {
            let value = lookup(params, name)?;
            bind.push(if *cast {
                value.clone()
            } else {
                text_form(value)
            });
        }
        Ok(Statement {
            sql: out,
            bind,
            query_params: Vec::new(),
        })
    }
}

/// How a result column is converted and decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Numeric,
    Text,
}

impl Kind {
    /// Kind of a column from its PostgreSQL type name.
    pub fn of(type_name: &str) -> Kind {
        match type_name {
            "BOOL" => Kind::Bool,
            "INT2" | "INT4" | "INT8" => Kind::Int,
            "FLOAT4" | "FLOAT8" => Kind::Float,
            "NUMERIC" => Kind::Numeric,
            _ => Kind::Text,
        }
    }

    /// The expression that converts the column `internal` of this kind, under
    /// the result name `output`.
    fn select_expr(self, internal: &str, output: &str) -> String {
        let source = quote_ident(internal);
        let output = quote_ident(output);
        match self {
            Kind::Bool => format!("{source} AS {output}"),
            Kind::Int => format!("{source}::int8 AS {output}"),
            Kind::Float => format!("{source}::float8 AS {output}"),
            Kind::Numeric | Kind::Text => format!("{source}::text AS {output}"),
        }
    }
}

/// Cast target for a parameter of the given PostgreSQL type, when one is needed.
pub fn cast_type(type_name: &str) -> Option<&'static str> {
    Some(match type_name {
        "BOOL" => "bool",
        "INT2" => "int2",
        "INT4" => "int4",
        "INT8" => "int8",
        "FLOAT4" => "float4",
        "FLOAT8" => "float8",
        "NUMERIC" => "numeric",
        "DATE" => "date",
        "TIME" => "time",
        "TIMESTAMP" => "timestamp",
        "TIMESTAMPTZ" => "timestamptz",
        "UUID" => "uuid",
        "JSON" => "json",
        "JSONB" => "jsonb",
        "INTERVAL" => "interval",
        _ => return None,
    })
}

/// A statement wrapped so that its columns decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    pub sql: String,
    /// One kind per result column, in order.
    pub kinds: Vec<Kind>,
}

/// Wraps `sql` so every column in `columns` (name, PostgreSQL type name)
/// arrives in a decodable form. Returns `None` when there are no columns.
pub fn wrap_result(sql: &str, columns: &[(String, String)]) -> Option<Wrapped> {
    if columns.is_empty() {
        return None;
    }
    let internal: Vec<String> = (0..columns.len()).map(|i| format!("c{i}")).collect();
    let kinds: Vec<Kind> = columns.iter().map(|(_, t)| Kind::of(t)).collect();
    let select_list: Vec<String> = columns
        .iter()
        .zip(&internal)
        .zip(&kinds)
        .map(|(((name, _), inner), kind)| kind.select_expr(inner, name))
        .collect();
    Some(Wrapped {
        sql: format!(
            "SELECT {} FROM (\n{}\n) AS portal_result({})",
            select_list.join(", "),
            sql,
            internal.join(", ")
        ),
        kinds,
    })
}

/// A `NUMERIC` value as a number: integers stay integers. Precision beyond an
/// `f64` is not kept, which is fine for display and charts.
pub fn numeric_cell(text: &str) -> Cell {
    if let Ok(i) = text.parse::<i64>() {
        Cell::Int(i)
    } else if let Ok(f) = text.parse::<f64>() {
        Cell::Float(f)
    } else {
        Cell::Text(text.to_string())
    }
}

/// Text form of a value, as the server reads it for an untyped parameter.
fn text_form(value: &Cell) -> Cell {
    match value {
        Cell::Null => Cell::Null,
        Cell::Bool(b) => Cell::Text(b.to_string()),
        Cell::Int(i) => Cell::Text(i.to_string()),
        Cell::Float(f) => Cell::Text(f.to_string()),
        Cell::Text(s) => Cell::Text(s.clone()),
    }
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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
    fn untyped_parameters_are_positional_and_bound_as_text() {
        let st = Postgres
            .prepare(
                "SELECT :a, :b, :a",
                &named(&[("a", Cell::Bool(true)), ("b", Cell::Int(7))]),
                None,
            )
            .unwrap();
        assert_eq!(st.sql, "SELECT $1, $2, $1");
        assert_eq!(
            st.bind,
            vec![Cell::Text("true".into()), Cell::Text("7".into())]
        );
    }

    #[test]
    fn typed_parameters_are_cast_and_keep_native_values() {
        let types = vec!["DATE".to_string(), "TEXT".to_string()];
        let st = Postgres
            .prepare(
                "SELECT :d, :t",
                &named(&[
                    ("d", Cell::Text("2024-01-02".into())),
                    ("t", Cell::Text("x".into())),
                ]),
                Some(&types),
            )
            .unwrap();
        assert_eq!(st.sql, "SELECT CAST($1 AS date), $2");
        assert_eq!(
            st.bind,
            vec![Cell::Text("2024-01-02".into()), Cell::Text("x".into())]
        );
    }

    #[test]
    fn trailing_semicolons_are_dropped_and_literals_untouched() {
        let st = Postgres
            .prepare("SELECT ':a', 1::int ;  ", &[], None)
            .unwrap();
        assert_eq!(st.sql, "SELECT ':a', 1::int");
    }

    #[test]
    fn wrapper_renames_columns_and_converts_kinds() {
        let w = wrap_result(
            "SELECT 1",
            &[
                ("numeric".into(), "NUMERIC".into()),
                ("numeric".into(), "NUMERIC".into()),
                ("flag".into(), "BOOL".into()),
                ("when".into(), "TIMESTAMPTZ".into()),
            ],
        )
        .unwrap();
        assert!(w.sql.starts_with("SELECT \"c0\"::text AS \"numeric\", \"c1\"::text AS \"numeric\", \"c2\" AS \"flag\", \"c3\"::text AS \"when\" FROM ("));
        assert!(w.sql.ends_with("AS portal_result(c0, c1, c2, c3)"));
        assert_eq!(
            w.kinds,
            vec![Kind::Numeric, Kind::Numeric, Kind::Bool, Kind::Text]
        );
        assert!(wrap_result("SELECT", &[]).is_none());
    }

    #[test]
    fn numerics_keep_integers_as_integers() {
        assert_eq!(numeric_cell("42"), Cell::Int(42));
        assert_eq!(numeric_cell("12.50"), Cell::Float(12.5));
        assert!(matches!(numeric_cell("NaN"), Cell::Float(f) if f.is_nan()));
    }

    #[test]
    fn only_known_types_are_cast() {
        assert_eq!(cast_type("UUID"), Some("uuid"));
        assert_eq!(cast_type("TEXT"), None);
        assert_eq!(cast_type("_INT4"), None);
    }
}
