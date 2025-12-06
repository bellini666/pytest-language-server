//! Find-references provider for pytest fixtures.

use super::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tracing::{debug, info};

impl Backend {
    /// Handle references request
    pub async fn handle_references(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        info!(
            "references request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Some(file_path) = self.uri_to_path(&uri) {
            info!(
                "Looking for fixture references at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            // First, find which fixture we're looking at (definition or usage)
            if let Some(fixture_name) = self.fixture_db.find_fixture_at_position(
                &file_path,
                position.line,
                position.character,
            ) {
                info!(
                    "Found fixture: {}, determining which definition to use",
                    fixture_name
                );

                let current_line = Self::lsp_line_to_internal(position.line);
                info!(
                    "Current cursor position: line {} (1-indexed), char {}",
                    current_line, position.character
                );

                // Determine which specific definition the user is referring to
                // This could be a usage (resolve to definition) or clicking on a definition itself
                let target_definition = self.fixture_db.find_fixture_definition(
                    &file_path,
                    position.line,
                    position.character,
                );

                let (references, definition_to_include) = if let Some(definition) =
                    target_definition
                {
                    info!(
                        "Found definition via usage at {:?}:{}, finding references that resolve to it",
                        definition.file_path, definition.line
                    );
                    // Find only references that resolve to this specific definition
                    let refs = self.fixture_db.find_references_for_definition(&definition);
                    (refs, Some(definition))
                } else {
                    // find_fixture_definition returns None if cursor is on a definition line (not a usage)
                    // Check if we're on a fixture definition line
                    let target_line = Self::lsp_line_to_internal(position.line);
                    if let Some(definition_at_line) = self.fixture_db.get_definition_at_line(
                        &file_path,
                        target_line,
                        &fixture_name,
                    ) {
                        info!(
                            "Found definition at cursor position {:?}:{}, finding references that resolve to it",
                            file_path, target_line
                        );
                        let refs = self
                            .fixture_db
                            .find_references_for_definition(&definition_at_line);
                        (refs, Some(definition_at_line))
                    } else {
                        info!(
                            "No specific definition found at cursor, finding all references by name"
                        );
                        // Fallback to finding all references by name (shouldn't normally happen)
                        (self.fixture_db.find_fixture_references(&fixture_name), None)
                    }
                };

                if references.is_empty() && definition_to_include.is_none() {
                    info!("No references found for fixture: {}", fixture_name);
                    return Ok(None);
                }

                info!(
                    "Found {} references for fixture: {}",
                    references.len(),
                    fixture_name
                );

                // Log all references to help debug
                for (i, r) in references.iter().enumerate() {
                    debug!(
                        "  Reference {}: {:?}:{} (chars {}-{})",
                        i,
                        r.file_path.file_name(),
                        r.line,
                        r.start_char,
                        r.end_char
                    );
                }

                // Check if current position is in the references
                let has_current_position = references
                    .iter()
                    .any(|r| r.file_path == file_path && r.line == current_line);
                info!(
                    "Current position (line {}) in references: {}",
                    current_line, has_current_position
                );

                // Convert references to LSP Locations
                let mut locations = Vec::new();

                // First, add the definition if we have one (LSP spec: includeDeclaration)
                if let Some(ref def) = definition_to_include {
                    let Some(def_uri) = self.path_to_uri(&def.file_path) else {
                        return Ok(None);
                    };

                    let def_line = Self::internal_line_to_lsp(def.line);
                    let def_location = Location {
                        uri: def_uri,
                        range: Self::create_point_range(def_line, 0),
                    };
                    locations.push(def_location);
                }

                // Then add all the usage references
                // Skip references that are on the same line as the definition (to avoid duplicates)
                let mut skipped_count = 0;
                for reference in &references {
                    // Check if this reference is the same location as the definition
                    if let Some(ref def) = definition_to_include {
                        if reference.file_path == def.file_path && reference.line == def.line {
                            debug!(
                                "Skipping reference at {:?}:{} (same as definition location)",
                                reference.file_path, reference.line
                            );
                            skipped_count += 1;
                            continue;
                        }
                    }

                    let Some(ref_uri) = self.path_to_uri(&reference.file_path) else {
                        continue;
                    };

                    let ref_line = Self::internal_line_to_lsp(reference.line);
                    let location = Location {
                        uri: ref_uri,
                        range: Self::create_range(
                            ref_line,
                            reference.start_char as u32,
                            ref_line,
                            reference.end_char as u32,
                        ),
                    };
                    debug!(
                        "Adding reference location: {:?}:{} (chars {}-{})",
                        reference.file_path.file_name(),
                        reference.line,
                        reference.start_char,
                        reference.end_char
                    );
                    locations.push(location);
                }

                info!(
                    "Returning {} locations (definition: {}, references: {}/{}, skipped: {})",
                    locations.len(),
                    if definition_to_include.is_some() {
                        1
                    } else {
                        0
                    },
                    references.len() - skipped_count,
                    references.len(),
                    skipped_count
                );
                return Ok(Some(locations));
            } else {
                info!("No fixture found at this position");
            }
        }

        Ok(None)
    }
}
