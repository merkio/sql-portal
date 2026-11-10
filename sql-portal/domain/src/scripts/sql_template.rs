//! Placeholder scanning lives in the `sql-dialect` crate; domain validation
//! and read-only checks stay here.

pub use sql_executor::scanner::{scan_placeholders, Placeholder};

fn code_only(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' | '`' => {
                i += 1;
                while i < chars.len() {
                    if chars[i] == c {
                        if i + 1 < chars.len() && chars[i + 1] == c {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
                out.push(c);
                out.push(c);
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                out.push(' ');
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                out.push(' ');
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

const WRITE_KEYWORDS: &[&str] = &[
    "insert", "update", "delete", "drop", "alter", "create", "truncate", "grant", "revoke",
    "merge", "replace", "attach", "detach", "pragma", "vacuum", "copy", "call", "exec",
];

/// Conservative check that a script is a single read-only `SELECT`/`WITH`
/// statement. It is a safety net, not a substitute for a read-only database
/// account.
pub fn is_read_only_select(sql: &str) -> bool {
    let code = code_only(sql);
    let statements: Vec<&str> = code
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if statements.len() != 1 {
        return false;
    }
    let statement = statements[0].to_ascii_lowercase();
    let mut words = statement
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty());
    match words.next() {
        Some("select") | Some("with") => {}
        _ => return false,
    }
    !statement
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|w| WRITE_KEYWORDS.contains(&w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sql: &str) -> Vec<String> {
        scan_placeholders(sql).into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn finds_simple_placeholders_in_order() {
        assert_eq!(
            names("SELECT * FROM t WHERE a = :a AND b >= :b_2 OR a = :a"),
            vec!["a", "b_2", "a"]
        );
    }

    #[test]
    fn records_byte_ranges() {
        let sql = "x = :abc;";
        let p = &scan_placeholders(sql)[0];
        assert_eq!(&sql[p.start..p.end], ":abc");
    }

    #[test]
    fn ignores_string_literals_and_quoted_identifiers() {
        assert!(names("SELECT ':nope', \":nope\", 'it''s :nope'").is_empty());
    }

    #[test]
    fn ignores_comments() {
        assert!(names("SELECT 1 -- :nope\n/* :nope */").is_empty());
        assert_eq!(names("SELECT 1 -- c\n WHERE a = :yes"), vec!["yes"]);
    }

    #[test]
    fn ignores_postgres_casts() {
        assert_eq!(names("SELECT :a::int, x::text"), vec!["a"]);
    }

    #[test]
    fn ignores_array_slices_and_digits() {
        assert!(names("SELECT arr[1:2], '12:30'").is_empty());
    }

    #[test]
    fn handles_unicode_text() {
        let sql = "SELECT 'é', :ü_not, :ok";
        // `ü` is not an ASCII identifier start, so only :ok matches.
        assert_eq!(names(sql), vec!["ok"]);
    }

    #[test]
    fn placeholder_at_end_of_input() {
        assert_eq!(names("WHERE a=:z"), vec!["z"]);
    }

    #[test]
    fn adjacent_to_punctuation() {
        assert_eq!(names("IN (:a,:b)"), vec!["a", "b"]);
    }

    #[test]
    fn read_only_accepts_select_and_with() {
        assert!(is_read_only_select("SELECT 1"));
        assert!(is_read_only_select(
            "  -- c\n select * from t where a = :a;  "
        ));
        assert!(is_read_only_select("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(is_read_only_select("SELECT 'drop table x; delete' AS txt"));
        assert!(is_read_only_select("SELECT updated_at, created FROM t"));
    }

    #[test]
    fn read_only_rejects_writes_and_multiple_statements() {
        assert!(!is_read_only_select("DELETE FROM t"));
        assert!(!is_read_only_select("SELECT 1; DROP TABLE t"));
        assert!(!is_read_only_select(
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d"
        ));
        assert!(!is_read_only_select("PRAGMA writable_schema = 1"));
        assert!(!is_read_only_select(""));
        assert!(!is_read_only_select("-- only a comment"));
    }
}
