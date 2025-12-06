//! Hover provider for pytest fixtures.

use super::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tracing::info;

impl Backend {
    /// Handle hover request
    pub async fn handle_hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        info!(
            "hover request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Some(file_path) = self.uri_to_path(&uri) {
            info!(
                "Looking for fixture at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            if let Some(definition) = self.fixture_db.find_fixture_definition(
                &file_path,
                position.line,
                position.character,
            ) {
                info!("Found fixture definition for hover: {:?}", definition.name);

                // Get workspace root for formatting documentation
                let workspace_root = self.workspace_root.read().await.clone();

                // Build hover content using shared formatter
                let content =
                    Self::format_fixture_documentation(&definition, workspace_root.as_ref());

                info!("Returning hover with content");
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: content,
                    }),
                    range: None,
                }));
            } else {
                info!("No fixture found for hover");
            }
        }

        Ok(None)
    }
}
