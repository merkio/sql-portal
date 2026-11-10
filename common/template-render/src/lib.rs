//! Template rendering on minijinja.
//!
//! A [`Templates`] set is compiled once from `(name, source)` pairs, typically
//! `include_str!`ed so a binary needs no template files at runtime. HTML
//! autoescaping is on for every template whose name ends in `.html`; trusted
//! fragments are passed through with `|safe`.

use std::hash::{Hash, Hasher};

use minijinja::Environment;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("template {name} is invalid: {source}")]
    Invalid {
        name: String,
        #[source]
        source: minijinja::Error,
    },
    #[error("template {name} could not be rendered: {source}")]
    Render {
        name: String,
        #[source]
        source: minijinja::Error,
    },
}

/// A compiled set of templates that may include and extend each other.
#[derive(Debug)]
pub struct Templates {
    env: Environment<'static>,
}

impl Templates {
    /// Compiles every template. Fails on the first one with a syntax error.
    pub fn new(templates: &[(&'static str, &'static str)]) -> Result<Self, TemplateError> {
        let mut env = Environment::new();
        for (name, source) in templates {
            env.add_template(name, source)
                .map_err(|source| TemplateError::Invalid {
                    name: (*name).to_string(),
                    source,
                })?;
        }
        Ok(Self { env })
    }

    /// Renders `name` with `context`. An unknown name is a render error.
    pub fn render<S: Serialize>(&self, name: &str, context: S) -> Result<String, TemplateError> {
        self.env
            .get_template(name)
            .and_then(|t| t.render(context))
            .map_err(|source| TemplateError::Render {
                name: name.to_string(),
                source,
            })
    }
}

/// A short version string derived from content, for cache-busting URLs such
/// as `app.css?v=…`. Stable for one build; not meant to be persisted.
pub fn content_version(parts: &[&str]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minijinja::context;

    fn templates() -> Templates {
        Templates::new(&[
            ("layout.html", "<main>{% block body %}{% endblock %}</main>"),
            (
                "page.html",
                "{% extends \"layout.html\" %}{% block body %}{{ text }}|{{ trusted|safe }}{% endblock %}",
            ),
            ("plain.txt", "{{ text }}"),
        ])
        .unwrap()
    }

    #[test]
    fn html_templates_escape_and_extend() {
        let out = templates()
            .render(
                "page.html",
                context! { text => "<b>", trusted => "<i>ok</i>" },
            )
            .unwrap();
        assert_eq!(out, "<main>&lt;b&gt;|<i>ok</i></main>");
    }

    #[test]
    fn non_html_templates_are_not_escaped() {
        let out = templates()
            .render("plain.txt", context! { text => "<b>" })
            .unwrap();
        assert_eq!(out, "<b>");
    }

    #[test]
    fn invalid_template_is_reported_by_name() {
        let err = Templates::new(&[("broken.html", "{% if %}")]).unwrap_err();
        assert!(matches!(err, TemplateError::Invalid { ref name, .. } if name == "broken.html"));
    }

    #[test]
    fn unknown_template_is_a_render_error() {
        let err = templates().render("missing.html", context! {}).unwrap_err();
        assert!(matches!(err, TemplateError::Render { ref name, .. } if name == "missing.html"));
    }

    #[test]
    fn content_version_follows_content() {
        assert_eq!(content_version(&["a", "b"]), content_version(&["a", "b"]));
        assert_ne!(content_version(&["a", "b"]), content_version(&["a", "c"]));
    }
}
