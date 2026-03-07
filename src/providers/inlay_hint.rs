//! Inlay hints provider for pytest fixtures.
//!
//! Shows fixture return types inline for fixture parameters in test functions
//! when the fixture has an explicit return type annotation.

use super::Backend;
use crate::fixtures::string_utils::parameter_has_annotation;
use std::collections::HashMap;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Handle inlay hints request.
    ///
    /// Returns type hints for fixture parameters when the fixture has an explicit
    /// return type annotation. This helps developers understand what type each
    /// fixture provides without having to navigate to its definition.
    ///
    /// Skips parameters that already have a type annotation to avoid redundancy.
    pub async fn handle_inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;

        info!("inlay_hint request: uri={:?}", uri);

        let Some(file_path) = self.uri_to_path(&uri) else {
            return Ok(None);
        };

        let Some(usages) = self.fixture_db.usages.get(&file_path) else {
            return Ok(None);
        };

        // Get current file content to check for existing annotations.
        // The file_cache is updated on every `textDocument/didChange` notification,
        // which editors send before requesting inlay hints. This ensures we check
        // against the current buffer state, not stale disk content.
        // Note: If an editor doesn't follow the LSP spec and requests hints before
        // sending didChange, hints might be shown/hidden incorrectly until the next sync.
        let content = self
            .fixture_db
            .file_cache
            .get(&file_path)
            .map(|c| c.clone());
        let lines: Vec<&str> = content
            .as_ref()
            .map(|c| c.lines().collect())
            .unwrap_or_default();

        // Pre-compute a map of fixture name -> definition for O(1) lookup.
        // This avoids calling find_closest_definition for each usage.
        let available = self.fixture_db.get_available_fixtures(&file_path);
        let fixture_map: HashMap<&str, &str> = available
            .iter()
            .filter_map(|def| {
                def.return_type
                    .as_ref()
                    .map(|rt| (def.name.as_str(), rt.as_str()))
            })
            .collect();

        // Early return if no fixtures have return types
        if fixture_map.is_empty() {
            return Ok(Some(Vec::new()));
        }

        // Note: we intentionally ignore `params.range` and return hints for the entire file.
        //
        // The LSP spec allows servers to return hints outside the requested range; editors
        // will cache and re-use them. Restricting by the viewport range causes flakiness for
        // multi-line function signatures: when a signature spans the top or bottom edge of the
        // visible area, parameters outside the viewport are silently dropped, causing hints to
        // appear/disappear as the user scrolls.
        //
        // This handler is invoked for any opened Python file,
        // but the hint count is naturally bounded: only parameter usages of fixtures that carry
        // an explicit return-type annotation produce a hint, so the result set stays small even
        // for large test modules.

        let mut hints = Vec::new();

        for usage in usages.iter() {
            // Only show hints for function parameter usages, not string literals
            // inside decorators like @pytest.mark.usefixtures("name")
            if !usage.is_parameter {
                continue;
            }

            // Look up return type from pre-computed map
            if let Some(&return_type) = fixture_map.get(usage.name.as_str()) {
                // Check if this parameter already has a type annotation
                // by looking at the text after the parameter name in the current buffer
                if parameter_has_annotation(&lines, usage.line, usage.end_char) {
                    continue;
                }

                let lsp_line = Self::internal_line_to_lsp(usage.line);

                hints.push(InlayHint {
                    position: Position {
                        line: lsp_line,
                        character: usage.end_char as u32,
                    },
                    label: InlayHintLabel::String(format!(": {}", return_type)),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::String(format!(
                        "Fixture '{}' returns {}",
                        usage.name, return_type
                    ))),
                    padding_left: Some(false),
                    padding_right: Some(false),
                    data: None,
                });
            }
        }

        info!("Returning {} inlay hints", hints.len());
        Ok(Some(hints))
    }
}
