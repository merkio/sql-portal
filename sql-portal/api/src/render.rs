//! Template environment. Templates are compiled into the binary, so the
//! container needs no template files at runtime. HTML autoescaping is on for
//! every `.html` template; trusted fragments are passed with `|safe`.

use std::sync::OnceLock;

use serde::Serialize;
use template_render::{content_version, Templates};

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
        content_version(&[
            include_str!("assets/app.css"),
            include_str!("assets/app.js"),
        ])
    })
}

fn templates() -> &'static Templates {
    static TEMPLATES_SET: OnceLock<Templates> = OnceLock::new();
    TEMPLATES_SET.get_or_init(|| Templates::new(TEMPLATES).unwrap_or_else(|e| panic!("{e}")))
}

/// Renders a bundled template. A failing render is logged and answered with a
/// generic message rather than leaking template details to the browser.
pub fn render<S: Serialize>(name: &str, context: S) -> String {
    templates().render(name, context).unwrap_or_else(|e| {
        tracing::error!(template = name, error = %e, "template render failed");
        "<p class=\"alert\" role=\"alert\">The page could not be shown.</p>".to_string()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_bundled_template_compiles() {
        super::templates();
    }
}
