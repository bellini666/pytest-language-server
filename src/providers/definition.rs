//! Go-to-definition provider for pytest fixtures.

use super::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tracing::info;

impl Backend {
    /// Handle goto_definition request
    pub async fn handle_goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        info!(
            "goto_definition request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Some(file_path) = self.uri_to_path(&uri) {
            info!(
                "Looking for fixture definition at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            if let Some(definition) = self.fixture_db.find_fixture_definition(
                &file_path,
                position.line,
                position.character,
            ) {
                info!("Found definition: {:?}", definition);
                let Some(def_uri) = self.path_to_uri(&definition.file_path) else {
                    return Ok(None);
                };

                let def_line = Self::internal_line_to_lsp(definition.line);
                let location = Location {
                    uri: def_uri.clone(),
                    range: Self::create_point_range(def_line, 0),
                };
                info!("Returning location: {:?}", location);
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            } else {
                info!("No fixture definition found");
            }
        }

        Ok(None)
    }
}
