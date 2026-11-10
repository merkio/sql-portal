//! Template environment. Templates are compiled into the binary, so the
//! container needs no template files at runtime. HTML autoescaping is on for
//! every `.html` template; trusted fragments are passed with `|safe`.

use std::sync::OnceLock;

use minijinja::Environment;
use serde::Serialize;

const TEMPLATES: &[(&str, &str)] = &[
    ("layout.html", include_str!("templates/layout.html")),
    ("index.html", include_str!("templates/index.html")),
    ("login.html", include_str!("templates/login.html")),
    ("message.html", include_str!("templates/message.html")),
    ("fields.html", include_str!("templates/fields.html")),
    ("script.html", include_str!("templates/script.html")),
];

/// The bundled stylesheet and script, by content. Used as `?v=` in the page
/// URLs, so a new build never runs with a stylesheet cached from an older one.
pub fn asset_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        include_str!("assets/app.css").hash(&mut hasher);
        include_str!("assets/app.js").hash(&mut hasher);
        format!("{:x}", hasher.finish())
    })
}

fn env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();
        for (name, source) in TEMPLATES {
            env.add_template(name, source)
                .unwrap_or_else(|e| panic!("bundled template {name} is invalid: {e}"));
        }
        env
    })
}

/// Renders a bundled template. A failing render is logged and answered with a
/// generic message rather than leaking template details to the browser.
pub fn render<S: Serialize>(name: &str, context: S) -> String {
    env()
        .get_template(name)
        .and_then(|t| t.render(context))
        .unwrap_or_else(|e| {
            tracing::error!(template = name, error = %e, "template render failed");
            "<p class=\"alert\" role=\"alert\">The page could not be shown.</p>".to_string()
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_bundled_template_compiles() {
        super::env();
    }
}
