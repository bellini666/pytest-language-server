//! Code Lens provider for pytest fixtures.
//!
//! Shows "N usages" above fixture definitions.

use super::Backend;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Handle code lens request - returns lenses for all fixtures in the file
    pub async fn handle_code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = &params.text_document.uri;

        info!("code_lens request: uri={:?}", uri);

        let Some(file_path) = self.uri_to_path(uri) else {
            return Ok(None);
        };

        // Get all definitions in this file using the file_definitions reverse
        // index (avoids scanning the whole workspace).
        let mut lenses = Vec::new();

        let fixture_names: Vec<String> = self
            .fixture_db
            .file_definitions
            .get(&file_path)
            .map(|entry| entry.value().iter().cloned().collect())
            .unwrap_or_default();

        for name in &fixture_names {
            // Snapshot matching definitions so no map guard is held across the
            // find_references_for_definition call below (it reads this map too).
            let defs: Vec<_> = self
                .fixture_db
                .definitions
                .get(name)
                .map(|entry| {
                    entry
                        .value()
                        .iter()
                        .filter(|def| def.file_path == file_path && !def.is_third_party)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            for def in &defs {
                // Count usages for this definition
                let references = self.fixture_db.find_references_for_definition(def);
                let usage_count = references.len();

                let line = Self::internal_line_to_lsp(def.line);
                let range = Self::create_range(line, 0, line, 0);

                let title = if usage_count == 1 {
                    "1 usage".to_string()
                } else {
                    format!("{} usages", usage_count)
                };

                // Build command arguments - these serializations should not fail
                // for simple types like strings and numbers
                let arguments = match (
                    serde_json::to_value(uri.to_string()),
                    serde_json::to_value(line),
                    serde_json::to_value(self.to_lsp_col(&def.file_path, def.line, def.start_char)),
                ) {
                    (Ok(uri_val), Ok(line_val), Ok(char_val)) => {
                        Some(vec![uri_val, line_val, char_val])
                    }
                    _ => {
                        tracing::warn!(
                            "Failed to serialize code lens arguments for fixture: {}",
                            def.name
                        );
                        continue;
                    }
                };

                let lens = CodeLens {
                    range,
                    command: Some(Command {
                        title,
                        command: "pytest-lsp.findReferences".to_string(),
                        arguments,
                    }),
                    data: None,
                };

                lenses.push(lens);
            }
        }

        info!("Returning {} code lenses for {:?}", lenses.len(), file_path);

        if lenses.is_empty() {
            Ok(None)
        } else {
            Ok(Some(lenses))
        }
    }
}
