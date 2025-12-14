//! Go-to-implementation provider for pytest fixtures.
//!
//! For generator fixtures (those with yield), "implementation" refers to
//! the yield statement where the fixture value is produced.

use super::Backend;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::request::{GotoImplementationParams, GotoImplementationResponse};
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Handle goto_implementation request.
    ///
    /// For fixtures, "implementation" is the yield statement (if present).
    /// This allows jumping to where the fixture value is actually produced
    /// in generator fixtures.
    pub async fn handle_goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        info!(
            "goto_implementation request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Some(file_path) = self.uri_to_path(&uri) {
            info!(
                "Looking for fixture implementation at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            // First find the fixture definition
            if let Some(definition) = self.fixture_db.find_fixture_definition(
                &file_path,
                position.line,
                position.character,
            ) {
                info!("Found definition: {:?}", definition);

                // Check if the fixture has a yield line (generator fixture)
                if let Some(yield_line) = definition.yield_line {
                    let Some(def_uri) = self.path_to_uri(&definition.file_path) else {
                        return Ok(None);
                    };

                    // Convert to LSP line (0-based)
                    let lsp_line = Self::internal_line_to_lsp(yield_line);
                    let location = Location {
                        uri: def_uri.clone(),
                        range: Self::create_point_range(lsp_line, 0),
                    };
                    info!("Returning yield location: {:?}", location);
                    return Ok(Some(GotoImplementationResponse::Scalar(location)));
                } else {
                    info!("Fixture has no yield statement (not a generator fixture)");
                    // For non-generator fixtures, fall back to definition
                    let Some(def_uri) = self.path_to_uri(&definition.file_path) else {
                        return Ok(None);
                    };

                    let def_line = Self::internal_line_to_lsp(definition.line);
                    let location = Location {
                        uri: def_uri.clone(),
                        range: Self::create_point_range(def_line, 0),
                    };
                    info!("Returning definition location (no yield): {:?}", location);
                    return Ok(Some(GotoImplementationResponse::Scalar(location)));
                }
            } else {
                info!("No fixture definition found");
            }
        }

        Ok(None)
    }
}
