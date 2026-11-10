use serde::Serialize;

/// A scalar value flowing in and out of SQL scripts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Value {
    /// Numeric view of the value, used by charts.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) if f.is_finite() => Some(*f),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
            _ => None,
        }
    }

    /// Human readable representation (NULL renders as an empty string).
    pub fn display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Text(s) => s.clone(),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_view() {
        assert_eq!(Value::Int(3).as_f64(), Some(3.0));
        assert_eq!(Value::Float(2.5).as_f64(), Some(2.5));
        assert_eq!(Value::Text(" 7.5 ".into()).as_f64(), Some(7.5));
        assert_eq!(Value::Float(f64::NAN).as_f64(), None);
        assert_eq!(Value::Text("abc".into()).as_f64(), None);
        assert_eq!(Value::Null.as_f64(), None);
        assert_eq!(Value::Bool(true).as_f64(), Some(1.0));
    }

    #[test]
    fn display_formats() {
        assert_eq!(Value::Null.display(), "");
        assert_eq!(Value::Float(2.0).display(), "2.0");
        assert_eq!(Value::Float(2.25).display(), "2.25");
        assert_eq!(Value::Int(-4).display(), "-4");
        assert_eq!(Value::Text("x".into()).display(), "x");
    }

    #[test]
    fn serializes_untagged() {
        let json = serde_json::to_string(&vec![
            Value::Null,
            Value::Bool(true),
            Value::Int(1),
            Value::Float(1.5),
            Value::Text("a".into()),
        ])
        .unwrap();
        assert_eq!(json, r#"[null,true,1,1.5,"a"]"#);
    }
}
