//! Workspace symbols provider for pytest fixtures.
//!
//! Provides the workspace/symbol LSP feature, enabling fuzzy search for
//! fixtures across the entire workspace.

use super::Backend;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Handle workspace/symbol request.
    ///
    /// Returns all fixture definitions matching the query string.
    /// This enables "Go to Symbol in Workspace" (Cmd+T / Ctrl+T) in editors.
    #[allow(deprecated)] // SymbolInformation::deprecated is required by LSP spec
    pub async fn handle_workspace_symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();

        info!("workspace_symbol request: query={:?}", query);

        let mut symbols: Vec<SymbolInformation> = Vec::new();

        // Iterate over all fixture definitions
        for entry in self.fixture_db.definitions.iter() {
            for definition in entry.value() {
                // Skip third-party fixtures
                if definition.is_third_party {
                    continue;
                }

                // If query is empty, return all fixtures; otherwise filter by name
                if !query.is_empty() && !definition.name.to_lowercase().contains(&query) {
                    continue;
                }

                let Some(uri) = self.path_to_uri(&definition.file_path) else {
                    continue;
                };

                let line = Self::internal_line_to_lsp(definition.line);
                let start_char = self.to_lsp_col(
                    &definition.file_path,
                    definition.line,
                    definition.start_char,
                );
                let end_char =
                    self.to_lsp_col(&definition.file_path, definition.line, definition.end_char);

                let location = Location {
                    uri,
                    range: Self::create_range(line, start_char, line, end_char),
                };

                let symbol = SymbolInformation {
                    name: definition.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    location,
                    container_name: definition
                        .file_path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .map(|s| s.to_string()),
                };

                symbols.push(symbol);
            }
        }

        // Sort by name for consistent ordering
        symbols.sort_by(|a, b| a.name.cmp(&b.name));

        info!("Returning {} workspace symbols", symbols.len());

        if symbols.is_empty() {
            Ok(None)
        } else {
            Ok(Some(symbols))
        }
    }
}
