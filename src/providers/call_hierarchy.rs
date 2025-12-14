//! Call hierarchy provider for pytest fixtures.
//!
//! Provides fixture dependency visualization:
//! - Incoming calls: fixtures/tests that use this fixture
//! - Outgoing calls: fixtures this fixture depends on

use super::Backend;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::info;

impl Backend {
    /// Handle prepareCallHierarchy request.
    ///
    /// Returns a CallHierarchyItem for the fixture at the cursor position.
    pub async fn handle_prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        info!(
            "prepareCallHierarchy request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Some(file_path) = self.uri_to_path(&uri) {
            // Find the fixture at the cursor position
            if let Some(definition) = self.fixture_db.find_fixture_definition(
                &file_path,
                position.line,
                position.character,
            ) {
                let Some(def_uri) = self.path_to_uri(&definition.file_path) else {
                    return Ok(None);
                };

                let def_line = Self::internal_line_to_lsp(definition.line);
                let selection_range = Range {
                    start: Position {
                        line: def_line,
                        character: definition.start_char as u32,
                    },
                    end: Position {
                        line: def_line,
                        character: definition.end_char as u32,
                    },
                };

                // Range covers the whole fixture definition line
                let range = Self::create_point_range(def_line, 0);

                let item = CallHierarchyItem {
                    name: definition.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: Some(format!(
                        "@pytest.fixture{}",
                        if definition.scope != crate::fixtures::types::FixtureScope::Function {
                            format!("(scope=\"{}\")", definition.scope.as_str())
                        } else {
                            String::new()
                        }
                    )),
                    uri: def_uri,
                    range,
                    selection_range,
                    data: None,
                };

                info!("Returning call hierarchy item: {:?}", item);
                return Ok(Some(vec![item]));
            }
        }

        Ok(None)
    }

    /// Handle callHierarchy/incomingCalls request.
    ///
    /// Returns all fixtures and tests that use the given fixture.
    pub async fn handle_incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        let item = &params.item;
        info!("incomingCalls request for: {}", item.name);

        let Some(file_path) = self.uri_to_path(&item.uri) else {
            return Ok(None);
        };

        // Get the fixture definition
        let Some(defs) = self.fixture_db.definitions.get(&item.name) else {
            return Ok(None);
        };

        // Find the matching definition by file path
        let Some(definition) = defs.iter().find(|d| d.file_path == file_path) else {
            return Ok(None);
        };

        // Find all references to this fixture
        let references = self.fixture_db.find_references_for_definition(definition);

        let mut incoming_calls: Vec<CallHierarchyIncomingCall> = Vec::new();

        for usage in references {
            // Skip self-references (the definition itself)
            if usage.file_path == definition.file_path && usage.line == definition.line {
                continue;
            }

            let Some(usage_uri) = self.path_to_uri(&usage.file_path) else {
                continue;
            };

            let usage_line = Self::internal_line_to_lsp(usage.line);
            let from_range = Range {
                start: Position {
                    line: usage_line,
                    character: usage.start_char as u32,
                },
                end: Position {
                    line: usage_line,
                    character: usage.end_char as u32,
                },
            };

            // Try to find what fixture/test this usage is in
            let caller_name = self
                .fixture_db
                .find_containing_function(&usage.file_path, usage.line)
                .unwrap_or_else(|| "<unknown>".to_string());

            let from_item = CallHierarchyItem {
                name: caller_name,
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: Some(usage.file_path.display().to_string()),
                uri: usage_uri,
                range: from_range,
                selection_range: from_range,
                data: None,
            };

            incoming_calls.push(CallHierarchyIncomingCall {
                from: from_item,
                from_ranges: vec![from_range],
            });
        }

        info!("Found {} incoming calls", incoming_calls.len());
        Ok(Some(incoming_calls))
    }

    /// Handle callHierarchy/outgoingCalls request.
    ///
    /// Returns all fixtures that the given fixture depends on.
    pub async fn handle_outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        let item = &params.item;
        info!("outgoingCalls request for: {}", item.name);

        let Some(file_path) = self.uri_to_path(&item.uri) else {
            return Ok(None);
        };

        // Get the fixture definition
        let Some(defs) = self.fixture_db.definitions.get(&item.name) else {
            return Ok(None);
        };

        // Find the matching definition by file path
        let Some(definition) = defs.iter().find(|d| d.file_path == file_path) else {
            return Ok(None);
        };

        let mut outgoing_calls: Vec<CallHierarchyOutgoingCall> = Vec::new();

        // Each dependency is an outgoing call
        for dep_name in &definition.dependencies {
            // Resolve the dependency to its definition
            if let Some(dep_def) = self
                .fixture_db
                .resolve_fixture_for_file(&file_path, dep_name)
            {
                let Some(dep_uri) = self.path_to_uri(&dep_def.file_path) else {
                    continue;
                };

                let dep_line = Self::internal_line_to_lsp(dep_def.line);
                let to_range = Range {
                    start: Position {
                        line: dep_line,
                        character: dep_def.start_char as u32,
                    },
                    end: Position {
                        line: dep_line,
                        character: dep_def.end_char as u32,
                    },
                };

                let to_item = CallHierarchyItem {
                    name: dep_def.name.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: Some(format!(
                        "@pytest.fixture{}",
                        if dep_def.scope != crate::fixtures::types::FixtureScope::Function {
                            format!("(scope=\"{}\")", dep_def.scope.as_str())
                        } else {
                            String::new()
                        }
                    )),
                    uri: dep_uri,
                    range: Self::create_point_range(dep_line, 0),
                    selection_range: to_range,
                    data: None,
                };

                // Find where in the fixture the dependency is referenced
                // (parameter position in the signature)
                let from_ranges = self
                    .find_parameter_ranges(&file_path, definition.line, dep_name)
                    .unwrap_or_else(|| vec![to_range]);

                outgoing_calls.push(CallHierarchyOutgoingCall {
                    to: to_item,
                    from_ranges,
                });
            }
        }

        info!("Found {} outgoing calls", outgoing_calls.len());
        Ok(Some(outgoing_calls))
    }

    /// Find the range(s) where a parameter name appears in a function signature.
    fn find_parameter_ranges(
        &self,
        file_path: &std::path::Path,
        line: usize,
        param_name: &str,
    ) -> Option<Vec<Range>> {
        let content = self.fixture_db.file_cache.get(file_path)?;
        let lines: Vec<&str> = content.lines().collect();

        // Get the line (0-indexed internally, but definition.line is 1-indexed)
        let line_content = lines.get(line.saturating_sub(1))?;

        // Find the parameter in the line
        if let Some(start) = line_content.find(param_name) {
            let lsp_line = Self::internal_line_to_lsp(line);
            let range = Range {
                start: Position {
                    line: lsp_line,
                    character: start as u32,
                },
                end: Position {
                    line: lsp_line,
                    character: (start + param_name.len()) as u32,
                },
            };
            return Some(vec![range]);
        }

        None
    }
}
