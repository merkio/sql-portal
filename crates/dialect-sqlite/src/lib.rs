//! SQLite dialect: `:name` placeholders become positional `?` binds, one per
//! occurrence, with the values in the order they appear.

use sql_dialect::{lookup, scan_placeholders, Cell, Dialect, DialectError, Statement};

#[derive(Debug, Clone, Copy, Default)]
pub struct Sqlite;

impl Dialect for Sqlite {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn prepare(
        &self,
        sql: &str,
        params: &[(String, Cell)],
        _param_types: Option<&[String]>,
    ) -> Result<Statement, DialectError> {
        let mut out = String::with_capacity(sql.len());
        let mut bind = Vec::new();
        let mut last = 0;
        for p in scan_placeholders(sql) {
            out.push_str(&sql[last..p.start]);
            out.push('?');
            bind.push(lookup(params, &p.name)?.clone());
            last = p.end;
        }
        out.push_str(&sql[last..]);
        Ok(Statement {
            sql: out,
            bind,
            query_params: Vec::new(),
        })
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
    fn placeholders_become_positional_binds_in_order() {
        let st = Sqlite
            .prepare(
                "SELECT :a, :b, :a -- :c",
                &named(&[("a", Cell::Int(1)), ("b", Cell::Text("x".into()))]),
                None,
            )
            .unwrap();
        assert_eq!(st.sql, "SELECT ?, ?, ? -- :c");
        assert_eq!(
            st.bind,
            vec![Cell::Int(1), Cell::Text("x".into()), Cell::Int(1)]
        );
    }

    #[test]
    fn missing_values_are_reported() {
        assert!(Sqlite.prepare("SELECT :z", &[], None).is_err());
    }

    #[test]
    fn literals_and_casts_are_untouched() {
        let st = Sqlite
            .prepare("SELECT ':a', x::text FROM t", &[], None)
            .unwrap();
        assert_eq!(st.sql, "SELECT ':a', x::text FROM t");
        assert!(st.bind.is_empty());
    }
}
