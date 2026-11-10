use chrono::NaiveDate;
use regex::Regex;

use super::error::FieldError;
use crate::common::{DomainError, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Text,
    Integer,
    Number,
    Boolean,
    Date,
    Select,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// Definition of one script parameter; it drives both the HTML form and the
/// validation/binding of submitted values.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDefinition {
    pub name: String,
    pub label: String,
    pub kind: ParamKind,
    pub required: bool,
    pub default: Option<String>,
    pub help: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub max_length: usize,
    pub pattern: Option<String>,
    pub options: Vec<SelectOption>,
}

pub const DEFAULT_MAX_LENGTH: usize = 255;

fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl ParamDefinition {
    pub fn new(name: impl Into<String>, kind: ParamKind) -> Self {
        let name = name.into();
        Self {
            label: name.clone(),
            name,
            kind,
            required: false,
            default: None,
            help: None,
            min: None,
            max: None,
            max_length: DEFAULT_MAX_LENGTH,
            pattern: None,
            options: vec![],
        }
    }

    /// Checks the definition itself (called while building a script).
    pub fn validate(&self) -> Result<(), DomainError> {
        if !is_valid_name(&self.name) {
            return Err(DomainError::invalid(format!(
                "parameter name '{}' must match [A-Za-z_][A-Za-z0-9_]*",
                self.name
            )));
        }
        if self.kind == ParamKind::Select && self.options.is_empty() {
            return Err(DomainError::invalid(format!(
                "select parameter '{}' needs at least one option",
                self.name
            )));
        }
        if let (Some(min), Some(max)) = (self.min, self.max) {
            if min > max {
                return Err(DomainError::invalid(format!(
                    "parameter '{}' has min greater than max",
                    self.name
                )));
            }
        }
        if let Some(pattern) = &self.pattern {
            Regex::new(pattern).map_err(|e| {
                DomainError::invalid(format!(
                    "parameter '{}' has an invalid pattern: {e}",
                    self.name
                ))
            })?;
        }
        // The default must itself be valid.
        if self.default.is_some() {
            self.parse(None).map_err(|e| {
                DomainError::invalid(format!(
                    "default of parameter '{}' is invalid: {}",
                    self.name, e.message
                ))
            })?;
        }
        Ok(())
    }

    /// Whether the form can be submitted without the user typing anything.
    pub fn is_satisfied_without_input(&self) -> bool {
        !self.required || self.default.is_some() || self.kind == ParamKind::Boolean
    }

    /// Validates a submitted raw value and converts it into a typed [`Value`].
    pub fn parse(&self, raw: Option<&str>) -> Result<Value, FieldError> {
        let err = |message: String| FieldError {
            param: self.name.clone(),
            message,
        };

        if self.kind == ParamKind::Boolean {
            let effective = raw.map(str::trim).or(self.default.as_deref()).unwrap_or("");
            return match effective.to_ascii_lowercase().as_str() {
                "true" | "1" | "on" | "yes" => Ok(Value::Bool(true)),
                "false" | "0" | "off" | "no" | "" => Ok(Value::Bool(false)),
                other => Err(err(format!("'{other}' is not a valid boolean"))),
            };
        }

        let effective = match raw.map(str::trim) {
            Some(s) if !s.is_empty() => Some(s),
            _ => self.default.as_deref(),
        };
        let Some(text) = effective else {
            return if self.required {
                Err(err("this field is required".into()))
            } else {
                Ok(Value::Null)
            };
        };

        match self.kind {
            ParamKind::Text => {
                if text.chars().count() > self.max_length {
                    return Err(err(format!(
                        "must be at most {} characters",
                        self.max_length
                    )));
                }
                if let Some(pattern) = &self.pattern {
                    let re = Regex::new(pattern).map_err(|e| err(format!("bad pattern: {e}")))?;
                    if !re.is_match(text) {
                        return Err(err("does not match the expected format".into()));
                    }
                }
                Ok(Value::Text(text.to_string()))
            }
            ParamKind::Integer => {
                let n: i64 = text
                    .parse()
                    .map_err(|_| err(format!("'{text}' is not an integer")))?;
                self.check_range(n as f64).map_err(err)?;
                Ok(Value::Int(n))
            }
            ParamKind::Number => {
                let n: f64 = text
                    .parse()
                    .map_err(|_| err(format!("'{text}' is not a number")))?;
                if !n.is_finite() {
                    return Err(err("must be a finite number".into()));
                }
                self.check_range(n).map_err(err)?;
                Ok(Value::Float(n))
            }
            ParamKind::Date => {
                let d = NaiveDate::parse_from_str(text, "%Y-%m-%d")
                    .map_err(|_| err(format!("'{text}' is not a date (YYYY-MM-DD)")))?;
                Ok(Value::Text(d.format("%Y-%m-%d").to_string()))
            }
            ParamKind::Select => {
                if self.options.iter().any(|o| o.value == text) {
                    Ok(Value::Text(text.to_string()))
                } else {
                    Err(err(format!("'{text}' is not an allowed option")))
                }
            }
            ParamKind::Boolean => unreachable!("handled above"),
        }
    }

    fn check_range(&self, n: f64) -> Result<(), String> {
        if let Some(min) = self.min {
            if n < min {
                return Err(format!("must be at least {min}"));
            }
        }
        if let Some(max) = self.max {
            if n > max {
                return Err(format!("must be at most {max}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(kind: ParamKind) -> ParamDefinition {
        ParamDefinition::new("p", kind)
    }

    #[test]
    fn required_without_value_fails() {
        let mut d = def(ParamKind::Text);
        d.required = true;
        assert_eq!(d.parse(None).unwrap_err().message, "this field is required");
        assert!(d.parse(Some("  ")).is_err());
    }

    #[test]
    fn optional_without_value_is_null() {
        assert_eq!(def(ParamKind::Text).parse(None).unwrap(), Value::Null);
    }

    #[test]
    fn default_is_used_when_blank() {
        let mut d = def(ParamKind::Integer);
        d.default = Some("5".into());
        assert_eq!(d.parse(None).unwrap(), Value::Int(5));
        assert_eq!(d.parse(Some("")).unwrap(), Value::Int(5));
        assert_eq!(d.parse(Some("9")).unwrap(), Value::Int(9));
    }

    #[test]
    fn integer_parsing_and_range() {
        let mut d = def(ParamKind::Integer);
        d.min = Some(1.0);
        d.max = Some(10.0);
        assert_eq!(d.parse(Some("3")).unwrap(), Value::Int(3));
        assert!(d.parse(Some("0")).unwrap_err().message.contains("at least"));
        assert!(d.parse(Some("11")).unwrap_err().message.contains("at most"));
        assert!(d.parse(Some("x")).unwrap_err().message.contains("integer"));
        assert!(d.parse(Some("1.5")).is_err());
    }

    #[test]
    fn number_parsing() {
        let d = def(ParamKind::Number);
        assert_eq!(d.parse(Some("2.5")).unwrap(), Value::Float(2.5));
        assert!(d.parse(Some("NaN")).is_err());
        assert!(d.parse(Some("inf")).is_err());
        assert!(d.parse(Some("abc")).is_err());
    }

    #[test]
    fn boolean_semantics() {
        let d = def(ParamKind::Boolean);
        assert_eq!(d.parse(None).unwrap(), Value::Bool(false));
        assert_eq!(d.parse(Some("on")).unwrap(), Value::Bool(true));
        assert_eq!(d.parse(Some("TRUE")).unwrap(), Value::Bool(true));
        assert_eq!(d.parse(Some("0")).unwrap(), Value::Bool(false));
        assert!(d.parse(Some("maybe")).is_err());
    }

    #[test]
    fn boolean_default_true_applies_when_absent() {
        let mut d = def(ParamKind::Boolean);
        d.default = Some("true".into());
        assert_eq!(d.parse(None).unwrap(), Value::Bool(true));
    }

    #[test]
    fn date_parsing() {
        let d = def(ParamKind::Date);
        assert_eq!(
            d.parse(Some("2024-02-29")).unwrap(),
            Value::Text("2024-02-29".into())
        );
        assert!(d.parse(Some("2023-02-29")).is_err());
        assert!(d.parse(Some("29/02/2024")).is_err());
    }

    #[test]
    fn select_only_accepts_listed_options() {
        let mut d = def(ParamKind::Select);
        d.options = vec![SelectOption {
            value: "a".into(),
            label: "A".into(),
        }];
        assert_eq!(d.parse(Some("a")).unwrap(), Value::Text("a".into()));
        assert!(d.parse(Some("b")).is_err());
    }

    #[test]
    fn text_length_and_pattern() {
        let mut d = def(ParamKind::Text);
        d.max_length = 3;
        assert!(d.parse(Some("abcd")).is_err());
        d.max_length = 10;
        d.pattern = Some("^[a-z]+$".into());
        assert!(d.parse(Some("abc")).is_ok());
        assert!(d.parse(Some("ab1")).is_err());
    }

    #[test]
    fn validate_checks_name_options_range_pattern_and_default() {
        assert!(ParamDefinition::new("1bad", ParamKind::Text)
            .validate()
            .is_err());
        assert!(ParamDefinition::new("has space", ParamKind::Text)
            .validate()
            .is_err());
        assert!(def(ParamKind::Select).validate().is_err());

        let mut d = def(ParamKind::Integer);
        d.min = Some(5.0);
        d.max = Some(1.0);
        assert!(d.validate().is_err());

        let mut d = def(ParamKind::Text);
        d.pattern = Some("(".into());
        assert!(d.validate().is_err());

        let mut d = def(ParamKind::Integer);
        d.default = Some("abc".into());
        assert!(d.validate().is_err());

        let mut d = def(ParamKind::Integer);
        d.default = Some("3".into());
        assert!(d.validate().is_ok());
    }

    #[test]
    fn satisfied_without_input() {
        let mut d = def(ParamKind::Text);
        assert!(d.is_satisfied_without_input());
        d.required = true;
        assert!(!d.is_satisfied_without_input());
        d.default = Some("x".into());
        assert!(d.is_satisfied_without_input());
    }
}
