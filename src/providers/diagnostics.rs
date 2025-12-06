//! Diagnostics provider for pytest fixtures.

use super::Backend;
use tower_lsp::lsp_types::*;
use tracing::info;

impl Backend {
    /// Publish diagnostics for undeclared fixtures in a file
    pub async fn publish_diagnostics_for_file(&self, uri: &Url, file_path: &std::path::Path) {
        let undeclared = self.fixture_db.get_undeclared_fixtures(file_path);

        let diagnostics: Vec<Diagnostic> = undeclared
            .into_iter()
            .map(|fixture| {
                let line = Self::internal_line_to_lsp(fixture.line);
                Diagnostic {
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
                }
            })
            .collect();

        info!("Publishing {} diagnostics for {:?}", diagnostics.len(), uri);
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}
