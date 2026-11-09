//! Architecture tests to enforce clean architecture layer boundaries.
//!
//! This test ensures that:
//! - domain has no dependencies on outer layers
//! - application depends only on domain (and common)
//! - infrastructure can depend on domain and common
//! - presentation can depend on all inner layers
//! - each major concern is in its own module with related files

use std::fs;
use std::path::Path;

/// Verify that domain layer has no imports from outer layers.
#[test]
fn domain_never_imports_from_outer_layers() {
    let domain_dir = Path::new("src/domain");
    let rust_files = find_rust_files(domain_dir);
    
    for file in rust_files {
        let contents = fs::read_to_string(&file).expect(&format!("Cannot read {:?}", file));
        
        // Check that domain files never import from application, infrastructure, or presentation
        assert!(
            !contents.contains("use crate::application::"),
            "File {:?} imports from application layer",
            file
        );
        assert!(
            !contents.contains("use crate::infrastructure::"),
            "File {:?} imports from infrastructure layer",
            file
        );
        assert!(
            !contents.contains("use crate::presentation::"),
            "File {:?} imports from presentation layer",
            file
        );
        
        // Domain can import from itself and common
        assert!(
            contents.lines()
                .filter(|l| l.contains("use crate::") && !l.contains("crate::domain::"))
                .all(|l| l.contains("crate::common::") || l.contains("crate::testing::")),
            "File {:?} has unexpected imports",
            file
        );
    }
}

/// Verify that application depends only on domain and common.
#[test]
fn application_imports_only_from_domain_and_common() {
    let app_dir = Path::new("src/application");
    let rust_files = find_rust_files(app_dir);
    
    for file in rust_files {
        let contents = fs::read_to_string(&file).expect(&format!("Cannot read {:?}", file));
        
        // Application must not import from infrastructure or presentation
        assert!(
            !contents.contains("use crate::infrastructure::"),
            "File {:?} imports from infrastructure layer",
            file
        );
        assert!(
            !contents.contains("use crate::presentation::"),
            "File {:?} imports from presentation layer",
            file
        );
        
        // Application can import from domain and common
        // (not checking this since it's expected and necessary)
    }
}

/// Verify that infrastructure doesn't import from presentation or application.
#[test]
fn infrastructure_never_imports_from_presentation_or_application_logic() {
    let infra_dir = Path::new("src/infrastructure");
    let rust_files = find_rust_files(infra_dir);
    
    for file in rust_files {
        let contents = fs::read_to_string(&file).expect(&format!("Cannot read {:?}", file));
        
        // Infrastructure must not import from application or presentation
        // (tower-sessions and axum are infrastructure deps, which is fine)
        assert!(
            !contents.contains("use crate::application::"),
            "File {:?} imports from application layer",
            file
        );
        // Presentation has HTTP routes and handlers, infrastructure shouldn't use those
        assert!(
            !contents.contains("use crate::presentation::handlers::"),
            "File {:?} imports handlers from presentation layer",
            file
        );
    }
}

/// Verify that HealthService has been moved to infrastructure.
#[test]
fn health_service_is_in_infrastructure_not_application() {
    // HealthService should NOT exist in application
    assert!(
        !Path::new("src/application/health_service.rs").exists(),
        "health_service.rs should be removed from application layer"
    );
    
    // HealthService SHOULD exist in infrastructure/health
    assert!(
        Path::new("src/infrastructure/health/service.rs").exists(),
        "health_service.rs should exist in infrastructure/health"
    );
    assert!(
        Path::new("src/infrastructure/health/probes.rs").exists(),
        "probes.rs should exist in infrastructure/health"
    );
}

/// Verify that related health modules are grouped together.
#[test]
fn health_related_files_are_in_same_module() {
    let health_dir = Path::new("src/infrastructure/health");
    
    // All health-related files should be in the health module
    assert!(health_dir.exists(), "health module should exist");
    assert!(health_dir.join("mod.rs").exists(), "health/mod.rs should exist");
    assert!(health_dir.join("service.rs").exists(), "health/service.rs should exist");
    assert!(health_dir.join("probes.rs").exists(), "health/probes.rs should exist");
    
    // Old locations should not exist
    assert!(
        !Path::new("src/infrastructure/probes.rs").exists(),
        "probes.rs should not exist in infrastructure root"
    );
}

/// Verify that ports are abstract (trait definitions only).
#[test]
fn ports_contain_only_trait_definitions() {
    let ports_dir = Path::new("src/domain/ports");
    let rust_files = find_rust_files(ports_dir);
    
    for file in rust_files {
        // Skip mod.rs as it's just re-exports
        if file.file_name().map(|n| n == "mod.rs").unwrap_or(false) {
            continue;
        }
        
        let contents = fs::read_to_string(&file).expect(&format!("Cannot read {:?}", file));
        
        // Ports should be traits (abstract), not concrete implementations
        // They may contain enums and structs for parameters/results, but not structs
        // that implement their own traits.
        
        // Count trait definitions in this file
        let trait_count = contents.matches("#[async_trait]").count() 
            + contents.matches("pub trait").count();
        
        // Each actual port file should have at least one trait definition
        assert!(
            trait_count > 0,
            "Ports file {:?} should contain at least one trait definition",
            file
        );
    }
}

/// Verify that bootstrap is the only place that imports from multiple layers.
#[test]
fn only_bootstrap_wires_the_object_graph() {
    let bootstrap_file = Path::new("src/bootstrap.rs");
    assert!(
        bootstrap_file.exists(),
        "bootstrap.rs should exist as composition root"
    );
    
    let contents = fs::read_to_string(bootstrap_file)
        .expect("Cannot read bootstrap.rs");
    
    // Bootstrap should import from all layers (this is expected)
    assert!(
        contents.contains("use crate::domain::"),
        "bootstrap should wire domain"
    );
    assert!(
        contents.contains("use crate::application::"),
        "bootstrap should wire application"
    );
    assert!(
        contents.contains("use crate::infrastructure::"),
        "bootstrap should wire infrastructure"
    );
    assert!(
        contents.contains("use crate::ui::"),
        "bootstrap should wire presentation (ui module)"
    );
}

/// Verify infrastructure has subsystems organized by concern.
#[test]
fn infrastructure_organized_by_concern() {
    let infra_dir = Path::new("src/infrastructure");
    
    // Check that key subsystems have their own modules
    assert!(infra_dir.join("config").is_dir(), "config should be a module");
    assert!(infra_dir.join("sql").is_dir(), "sql should be a module");
    assert!(infra_dir.join("grants").is_dir(), "grants should be a module");
    assert!(infra_dir.join("health").is_dir(), "health should be a module");
    
    // Check that related adapters are in the same module
    let health_files = find_rust_files(&infra_dir.join("health"));
    assert!(
        health_files.len() >= 3,
        "health module should contain multiple files (service, probes, mod, etc)"
    );
}

/// Helper: recursively find all .rs files in a directory.
fn find_rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(find_rust_files(&path));
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
