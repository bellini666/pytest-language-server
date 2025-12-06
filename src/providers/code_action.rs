//! Code action provider for pytest fixtures.

use super::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tracing::{info, warn};

impl Backend {
    /// Handle code_action request
    pub async fn handle_code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let context = params.context;

        info!(
            "code_action request: uri={:?}, diagnostics={}, only={:?}",
            uri,
            context.diagnostics.len(),
            context.only
        );

        // Check if the client is filtering by action kind
        if let Some(ref only_kinds) = context.only {
            if !only_kinds.iter().any(|k| {
                k == &CodeActionKind::QUICKFIX
                    || k.as_str().starts_with(CodeActionKind::QUICKFIX.as_str())
            }) {
                info!("Code action request filtered out by 'only' parameter");
                return Ok(None);
            }
        }

        if let Some(file_path) = self.uri_to_path(&uri) {
            let undeclared = self.fixture_db.get_undeclared_fixtures(&file_path);
            info!("Found {} undeclared fixtures in file", undeclared.len());
            let mut actions = Vec::new();

            // Process each diagnostic from the context
            for diagnostic in &context.diagnostics {
                info!(
                    "Processing diagnostic: code={:?}, range={:?}",
                    diagnostic.code, diagnostic.range
                );
                // Check if this is an undeclared-fixture diagnostic
                if let Some(NumberOrString::String(code)) = &diagnostic.code {
                    if code == "undeclared-fixture" {
                        // Find the corresponding undeclared fixture
                        let diag_line = Self::lsp_line_to_internal(diagnostic.range.start.line);
                        let diag_char = diagnostic.range.start.character as usize;

                        info!(
                            "Looking for undeclared fixture at line={}, char={}",
                            diag_line, diag_char
                        );

                        if let Some(fixture) = undeclared
                            .iter()
                            .find(|f| f.line == diag_line && f.start_char == diag_char)
                        {
                            info!("Found matching fixture: {}", fixture.name);
                            // Create a code action to add this fixture as a parameter
                            let function_line = Self::internal_line_to_lsp(fixture.function_line);

                            // Read the file to determine where to insert the parameter
                            if let Ok(content) = std::fs::read_to_string(&file_path) {
                                let lines: Vec<&str> = content.lines().collect();
                                // Use get() instead of direct indexing for safety
                                if let Some(func_line_content) = lines.get(function_line as usize) {
                                    // Find the closing parenthesis of the function signature
                                    // This is a simplified approach - works for single-line signatures
                                    if let Some(paren_pos) = func_line_content.find("):") {
                                        let insert_pos = if func_line_content[..paren_pos]
                                            .contains('(')
                                        {
                                            // Check if there are already parameters
                                            // Use find() result safely without unwrap
                                            let param_start = match func_line_content.find('(') {
                                                Some(pos) => pos + 1,
                                                None => {
                                                    warn!("Invalid function signature: missing opening parenthesis at {:?}:{}", file_path, function_line);
                                                    continue;
                                                }
                                            };
                                            let params_section =
                                                &func_line_content[param_start..paren_pos];

                                            if params_section.trim().is_empty() {
                                                // No parameters yet
                                                (function_line, (param_start as u32))
                                            } else {
                                                // Already has parameters, add after them
                                                (function_line, (paren_pos as u32))
                                            }
                                        } else {
                                            continue;
                                        };

                                        let has_params = !func_line_content[..paren_pos]
                                            .split('(')
                                            .next_back()
                                            .unwrap_or("")
                                            .trim()
                                            .is_empty();

                                        let text_to_insert = if has_params {
                                            format!(", {}", fixture.name)
                                        } else {
                                            fixture.name.clone()
                                        };

                                        let edit = WorkspaceEdit {
                                            changes: Some(
                                                vec![(
                                                    uri.clone(),
                                                    vec![TextEdit {
                                                        range: Self::create_point_range(
                                                            insert_pos.0,
                                                            insert_pos.1,
                                                        ),
                                                        new_text: text_to_insert,
                                                    }],
                                                )]
                                                .into_iter()
                                                .collect(),
                                            ),
                                            document_changes: None,
                                            change_annotations: None,
                                        };

                                        let action = CodeAction {
                                            title: format!(
                                                "Add '{}' fixture parameter",
                                                fixture.name
                                            ),
                                            kind: Some(CodeActionKind::QUICKFIX),
                                            diagnostics: Some(vec![diagnostic.clone()]),
                                            edit: Some(edit),
                                            command: None,
                                            is_preferred: Some(true),
                                            disabled: None,
                                            data: None,
                                        };

                                        info!(
                                            "Created code action: Add '{}' fixture parameter",
                                            fixture.name
                                        );
                                        actions.push(CodeActionOrCommand::CodeAction(action));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !actions.is_empty() {
                info!("Returning {} code actions", actions.len());
                return Ok(Some(actions));
            } else {
                info!("No code actions created");
            }
        }

        info!("Returning None for code_action request");
        Ok(None)
    }
}
