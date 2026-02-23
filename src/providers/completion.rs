//! Completion provider for pytest fixtures.

use super::Backend;
use crate::fixtures::types::FixtureScope;
use crate::fixtures::CompletionContext;
use crate::fixtures::FixtureDefinition;
use std::path::PathBuf;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::info;

/// Parameter names that should never appear in fixture completions, they should be handled by another lsp.
const EXCLUDED_PARAM_NAMES: &[&str] = &["self", "cls"];

/// Check whether a fixture should be excluded from completions based on scope rules.
/// A fixture with a broader scope cannot depend on a fixture with a narrower scope.
fn should_exclude_fixture(
    fixture: &FixtureDefinition,
    current_scope: Option<FixtureScope>,
) -> bool {
    // If current function is a test (None scope), allow everything
    let Some(scope) = current_scope else {
        return false;
    };
    // FixtureScope ordering: Function(0) < Class(1) < Module(2) < Package(3) < Session(4)
    // Exclude candidates whose scope is narrower than the current fixture's scope
    fixture.scope < scope
}

/// Compute a sort priority for a fixture based on its proximity to the current file.
/// Lower values = higher priority (shown first in completion list).
fn fixture_sort_priority(fixture: &FixtureDefinition, current_file: &std::path::Path) -> u8 {
    if fixture.file_path == current_file {
        0 // Same file
    } else if fixture.is_third_party {
        3 // Third-party (check before is_plugin since some are both)
    } else if fixture.is_plugin {
        2 // Plugin
    } else {
        1 // Conftest or other project files
    }
}

/// Build a sort_text string that groups fixtures by proximity priority,
/// then sorts alphabetically within each group.
fn make_sort_text(priority: u8, fixture_name: &str) -> String {
    format!("{}_{}", priority, fixture_name)
}

/// Build a detail string for a fixture completion item.
/// Format: `filename (scope) [origin]`
/// - scope is omitted when it's the default "function"
/// - origin tag is only added for plugin or third-party fixtures
fn make_fixture_detail(fixture: &FixtureDefinition) -> String {
    let filename = fixture
        .file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into());

    let mut detail = filename;

    // Add scope if not the default "function"
    if fixture.scope != FixtureScope::Function {
        detail.push_str(&format!(" ({})", fixture.scope.as_str()));
    }

    // Add origin tag
    if fixture.is_third_party {
        detail.push_str(" [third-party]");
    } else if fixture.is_plugin {
        detail.push_str(" [plugin]");
    }

    detail
}

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
                        declared_params,
                        fixture_scope,
                        ..
                    } => {
                        // In function signature - suggest fixtures as parameters (filter already declared)
                        return Ok(Some(self.create_fixture_completions(
                            &file_path,
                            &declared_params,
                            workspace_root.as_ref(),
                            fixture_scope,
                        )));
                    }
                    CompletionContext::FunctionBody {
                        function_line,
                        declared_params,
                        fixture_scope,
                        ..
                    } => {
                        // In function body - suggest fixtures with auto-add to parameters
                        return Ok(Some(self.create_fixture_completions_with_auto_add(
                            &file_path,
                            &declared_params,
                            function_line,
                            workspace_root.as_ref(),
                            fixture_scope,
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
    /// Filters out already-declared parameters and scope-incompatible fixtures
    pub fn create_fixture_completions(
        &self,
        file_path: &std::path::Path,
        declared_params: &[String],
        workspace_root: Option<&PathBuf>,
        fixture_scope: Option<FixtureScope>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let mut items = Vec::new();

        for fixture in available {
            // Skip fixtures that are already declared as parameters
            if declared_params.contains(&fixture.name) {
                continue;
            }

            // Skip special parameter names
            if EXCLUDED_PARAM_NAMES.contains(&fixture.name.as_str()) {
                continue;
            }

            // Skip fixtures with incompatible scope
            if should_exclude_fixture(&fixture, fixture_scope) {
                continue;
            }

            let detail = Some(make_fixture_detail(&fixture));
            let priority = fixture_sort_priority(&fixture, file_path);

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
                sort_text: Some(make_sort_text(priority, &fixture.name)),
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
        fixture_scope: Option<FixtureScope>,
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

            // Skip special parameter names
            if EXCLUDED_PARAM_NAMES.contains(&fixture.name.as_str()) {
                continue;
            }

            // Skip fixtures with incompatible scope
            if should_exclude_fixture(&fixture, fixture_scope) {
                continue;
            }

            let detail = Some(make_fixture_detail(&fixture));
            let priority = fixture_sort_priority(&fixture, file_path);

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
                sort_text: Some(make_sort_text(priority, &fixture.name)),
                ..Default::default()
            });
        }

        CompletionResponse::Array(items)
    }

    /// Create completion items for fixture names as strings (for decorators)
    /// Used in @pytest.mark.usefixtures("...") and @pytest.mark.parametrize(..., indirect=["..."])
    /// No scope filtering applied here (decision #3).
    pub fn create_string_fixture_completions(
        &self,
        file_path: &std::path::Path,
        workspace_root: Option<&PathBuf>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let mut items = Vec::new();

        for fixture in available {
            // Skip special parameter names
            if EXCLUDED_PARAM_NAMES.contains(&fixture.name.as_str()) {
                continue;
            }

            let detail = Some(make_fixture_detail(&fixture));
            let priority = fixture_sort_priority(&fixture, file_path);

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
                sort_text: Some(make_sort_text(priority, &fixture.name)),
                ..Default::default()
            });
        }

        CompletionResponse::Array(items)
    }
}
