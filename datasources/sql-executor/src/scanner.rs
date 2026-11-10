//! Understanding of `:name` placeholders in SQL script templates.
//!
//! Placeholders are only recognised outside string literals, quoted
//! identifiers and comments, and `::` (Postgres casts) is never a placeholder.

/// A `:name` placeholder found in a SQL text. `start..end` is the byte range
/// including the leading colon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub fn scan_placeholders(sql: &str) -> Vec<Placeholder> {
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let mut found = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let (_, c) = chars[i];
        match c {
            '\'' | '"' | '`' => {
                // Quoted literal / identifier; a doubled quote is an escape.
                i += 1;
                while i < chars.len() {
                    if chars[i].1 == c {
                        if i + 1 < chars.len() && chars[i + 1].1 == c {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            '-' if i + 1 < chars.len() && chars[i + 1].1 == '-' => {
                while i < chars.len() && chars[i].1 != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1].1 == '*' => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i].1 == '*' && chars[i + 1].1 == '/') {
                    i += 1;
                }
                i += 2;
            }
            ':' => {
                let prev = if i > 0 { Some(chars[i - 1].1) } else { None };
                let next = chars.get(i + 1).map(|(_, ch)| *ch);
                if next == Some(':') {
                    // Postgres cast `::type`
                    i += 2;
                    continue;
                }
                let prev_blocks = prev.is_some_and(|p| is_ident_char(p) || p == ':');
                if !prev_blocks && next.is_some_and(is_ident_start) {
                    let start = chars[i].0;
                    let mut j = i + 1;
                    while j < chars.len() && is_ident_char(chars[j].1) {
                        j += 1;
                    }
                    let end = chars.get(j).map(|(b, _)| *b).unwrap_or(sql.len());
                    found.push(Placeholder {
                        name: sql[start + 1..end].to_string(),
                        start,
                        end,
                    });
                    i = j;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    found
}
