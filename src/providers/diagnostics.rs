//! Diagnostics provider for pytest fixtures.

use super::Backend;
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Publish diagnostics for undeclared fixtures and circular dependencies in a file
    pub async fn publish_diagnostics_for_file(&self, uri: &Uri, file_path: &std::path::Path) {
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        // Get config to check for disabled diagnostics
        let config = self.config.read().await;
        let config = &*config; // Dereference the RwLockReadGuard

        // Collect undeclared fixture diagnostics (if not disabled)
        if !config.is_diagnostic_disabled("undeclared-fixture") {
            let undeclared = self.fixture_db.get_undeclared_fixtures(file_path);
            for fixture in undeclared {
                let line = Self::internal_line_to_lsp(fixture.line);
                diagnostics.push(Diagnostic {
                    range: Self::create_range(
                        line,
                        fixture.start_char as u32,
                        line,
                        fixture.end_char as u32,
                    ),
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("undeclared-fixture".to_string())),
                    code_description: None,
                    source: Some("pytest-lsp".to_string()),
                    message: format!(
                        "Fixture '{}' is used but not declared as a parameter",
                        fixture.name
                    ),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        // Collect circular dependency diagnostics (if not disabled)
        if !config.is_diagnostic_disabled("circular-dependency") {
            let cycles = self.fixture_db.detect_fixture_cycles_in_file(file_path);
            for cycle in cycles {
                let line = Self::internal_line_to_lsp(cycle.fixture.line);
                let cycle_str = cycle.cycle_path.join(" → ");
                diagnostics.push(Diagnostic {
                    range: Self::create_range(
                        line,
                        cycle.fixture.start_char as u32,
                        line,
                        cycle.fixture.end_char as u32,
                    ),
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: Some(NumberOrString::String("circular-dependency".to_string())),
                    code_description: None,
                    source: Some("pytest-lsp".to_string()),
                    message: format!("Circular fixture dependency detected: {}", cycle_str),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        // Collect scope mismatch diagnostics (if not disabled)
        if !config.is_diagnostic_disabled("scope-mismatch") {
            let mismatches = self.fixture_db.detect_scope_mismatches_in_file(file_path);
            for mismatch in mismatches {
                let line = Self::internal_line_to_lsp(mismatch.fixture.line);
                diagnostics.push(Diagnostic {
                    range: Self::create_range(
                        line,
                        mismatch.fixture.start_char as u32,
                        line,
                        mismatch.fixture.end_char as u32,
                    ),
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("scope-mismatch".to_string())),
                    code_description: None,
                    source: Some("pytest-lsp".to_string()),
                    message: format!(
                        "{}-scoped fixture '{}' depends on {}-scoped fixture '{}'",
                        mismatch.fixture.scope.as_str(),
                        mismatch.fixture.name,
                        mismatch.dependency.scope.as_str(),
                        mismatch.dependency.name
                    ),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        info!("Publishing {} diagnostics for {:?}", diagnostics.len(), uri);
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}
