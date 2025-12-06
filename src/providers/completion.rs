//! Completion provider for pytest fixtures.

use super::Backend;
use crate::fixtures::CompletionContext;
use std::path::PathBuf;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tracing::info;

impl Backend {
    /// Handle completion request
    pub async fn handle_completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        info!(
            "completion request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Some(file_path) = self.uri_to_path(&uri) {
            // Get the completion context
            if let Some(ctx) = self.fixture_db.get_completion_context(
                &file_path,
                position.line,
                position.character,
            ) {
                info!("Completion context: {:?}", ctx);

                // Get workspace root for formatting documentation
                let workspace_root = self.workspace_root.read().await.clone();

                match ctx {
                    CompletionContext::FunctionSignature {
                        declared_params, ..
                    } => {
                        // In function signature - suggest fixtures as parameters (filter already declared)
                        return Ok(Some(self.create_fixture_completions(
                            &file_path,
                            &declared_params,
                            workspace_root.as_ref(),
                        )));
                    }
                    CompletionContext::FunctionBody {
                        function_line,
                        declared_params,
                        ..
                    } => {
                        // In function body - suggest fixtures with auto-add to parameters
                        return Ok(Some(self.create_fixture_completions_with_auto_add(
                            &file_path,
                            &declared_params,
                            function_line,
                            workspace_root.as_ref(),
                        )));
                    }
                    CompletionContext::UsefixuturesDecorator
                    | CompletionContext::ParametrizeIndirect => {
                        // In decorator - suggest fixture names as strings
                        return Ok(Some(self.create_string_fixture_completions(
                            &file_path,
                            workspace_root.as_ref(),
                        )));
                    }
                }
            } else {
                info!("No completion context found");
            }
        }

        Ok(None)
    }

    /// Create completion items for fixtures (for function signature context)
    /// Filters out already-declared parameters
    pub fn create_fixture_completions(
        &self,
        file_path: &std::path::Path,
        declared_params: &[String],
        workspace_root: Option<&PathBuf>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let mut items = Vec::new();

        for fixture in available {
            // Skip fixtures that are already declared as parameters
            if declared_params.contains(&fixture.name) {
                continue;
            }

            let detail = fixture
                .file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());

            let doc_content = Self::format_fixture_documentation(&fixture, workspace_root);
            let documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: doc_content,
            }));

            items.push(CompletionItem {
                label: fixture.name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail,
                documentation,
                insert_text: Some(fixture.name.clone()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            });
        }

        CompletionResponse::Array(items)
    }

    /// Create completion items for fixtures with auto-add to function parameters
    /// When a completion is confirmed, it also inserts the fixture as a parameter
    pub fn create_fixture_completions_with_auto_add(
        &self,
        file_path: &std::path::Path,
        declared_params: &[String],
        function_line: usize,
        workspace_root: Option<&PathBuf>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let mut items = Vec::new();

        // Get insertion info for adding new parameters
        let insertion_info = self
            .fixture_db
            .get_function_param_insertion_info(file_path, function_line);

        for fixture in available {
            // Skip fixtures that are already declared as parameters
            if declared_params.contains(&fixture.name) {
                continue;
            }

            let detail = fixture
                .file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());

            let doc_content = Self::format_fixture_documentation(&fixture, workspace_root);
            let documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: doc_content,
            }));

            // Create additional text edit to add the fixture as a parameter
            let additional_text_edits = insertion_info.as_ref().map(|info| {
                let text = if info.needs_comma {
                    format!(", {}", fixture.name)
                } else {
                    fixture.name.clone()
                };
                let lsp_line = Self::internal_line_to_lsp(info.line);
                vec![TextEdit {
                    range: Self::create_point_range(lsp_line, info.char_pos as u32),
                    new_text: text,
                }]
            });

            items.push(CompletionItem {
                label: fixture.name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail,
                documentation,
                insert_text: Some(fixture.name.clone()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                additional_text_edits,
                ..Default::default()
            });
        }

        CompletionResponse::Array(items)
    }

    /// Create completion items for fixture names as strings (for decorators)
    /// Used in @pytest.mark.usefixtures("...") and @pytest.mark.parametrize(..., indirect=["..."])
    pub fn create_string_fixture_completions(
        &self,
        file_path: &std::path::Path,
        workspace_root: Option<&PathBuf>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let mut items = Vec::new();

        for fixture in available {
            let detail = fixture
                .file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());

            let doc_content = Self::format_fixture_documentation(&fixture, workspace_root);
            let documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: doc_content,
            }));

            items.push(CompletionItem {
                label: fixture.name.clone(),
                kind: Some(CompletionItemKind::TEXT),
                detail,
                documentation,
                // Don't add quotes - user is already inside a string
                insert_text: Some(fixture.name.clone()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            });
        }

        CompletionResponse::Array(items)
    }
}
