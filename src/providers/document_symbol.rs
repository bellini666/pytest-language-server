//! Document symbols provider for pytest fixtures.
//!
//! Provides the textDocument/documentSymbol LSP feature, enabling file outline
//! and breadcrumb navigation for fixtures in the editor.

use super::Backend;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Handle textDocument/documentSymbol request.
    ///
    /// Returns all fixture definitions in the document as symbols.
    /// This enables outline view and breadcrumb navigation in editors.
    pub async fn handle_document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        info!("document_symbol request: uri={:?}", uri);

        let Some(file_path) = self.uri_to_path(&uri) else {
            return Ok(None);
        };

        // Collect all fixture definitions for this file
        let mut symbols: Vec<DocumentSymbol> = Vec::new();

        // Iterate over all fixture definitions
        for entry in self.fixture_db.definitions.iter() {
            for definition in entry.value() {
                // Only include fixtures from this file
                if definition.file_path != file_path {
                    continue;
                }

                // Skip third-party fixtures (they're from site-packages, not user files)
                if definition.is_third_party {
                    continue;
                }

                let line = Self::internal_line_to_lsp(definition.line);
                let start_char = definition.start_char as u32;
                let end_char = definition.end_char as u32;

                // Selection range is the fixture name
                let selection_range = Self::create_range(line, start_char, line, end_char);

                // Full range includes the entire function body
                let end_line = Self::internal_line_to_lsp(definition.end_line);
                let range = Self::create_range(line, 0, end_line, 0);

                // Build detail string with return type if available
                let detail = definition
                    .return_type
                    .as_ref()
                    .map(|rt| format!("-> {}", rt));

                #[allow(deprecated)] // deprecated field is required by LSP spec
                let symbol = DocumentSymbol {
                    name: definition.name.clone(),
                    detail,
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range,
                    children: None,
                };

                symbols.push(symbol);
            }
        }

        // Sort symbols by line number for consistent ordering
        symbols.sort_by_key(|s| s.range.start.line);

        info!(
            "Returning {} document symbols for {:?}",
            symbols.len(),
            file_path
        );

        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        }
    }
}
