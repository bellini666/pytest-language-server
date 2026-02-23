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

/// Check whether a fixture should be excluded based on common rules
/// (excluded param names, already-declared params, and scope compatibility).
fn is_fixture_excluded(
    fixture: &FixtureDefinition,
    declared_params: Option<&[String]>,
    fixture_scope: Option<FixtureScope>,
) -> bool {
    // Skip special parameter names
    if EXCLUDED_PARAM_NAMES.contains(&fixture.name.as_str()) {
        return true;
    }

    // Skip fixtures that are already declared as parameters
    if let Some(params) = declared_params {
        if params.contains(&fixture.name) {
            return true;
        }
    }

    // Skip fixtures with incompatible scope
    if should_exclude_fixture(fixture, fixture_scope) {
        return true;
    }

    false
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
/// Format: `(scope) [origin]`
/// - scope is omitted when it's the default "function"
/// - origin tag is only added for plugin or third-party fixtures
fn make_fixture_detail(fixture: &FixtureDefinition) -> String {
    let mut parts = Vec::new();

    // Add scope if not the default "function"
    if fixture.scope != FixtureScope::Function {
        parts.push(format!("({})", fixture.scope.as_str()));
    }

    // Add origin tag
    if fixture.is_third_party {
        parts.push("[third-party]".to_string());
    } else if fixture.is_plugin {
        parts.push("[plugin]".to_string());
    }

    parts.join(" ")
}

/// A filtered and enriched fixture ready for completion item construction.
struct EnrichedFixture {
    fixture: FixtureDefinition,
    detail: String,
    sort_text: String,
}

/// Filter available fixtures according to common rules and enrich them with
/// detail/sort metadata.
fn filter_and_enrich_fixtures(
    available: Vec<FixtureDefinition>,
    file_path: &std::path::Path,
    declared_params: Option<&[String]>,
    fixture_scope: Option<FixtureScope>,
) -> Vec<EnrichedFixture> {
    available
        .into_iter()
        .filter(|f| !is_fixture_excluded(f, declared_params, fixture_scope))
        .map(|f| {
            let detail = make_fixture_detail(&f);
            let priority = fixture_sort_priority(&f, file_path);
            let sort_text = make_sort_text(priority, &f.name);
            EnrichedFixture {
                fixture: f,
                detail,
                sort_text,
            }
        })
        .collect()
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
        let enriched =
            filter_and_enrich_fixtures(available, file_path, Some(declared_params), fixture_scope);

        let items = enriched
            .into_iter()
            .map(|ef| {
                let documentation = Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: Self::format_fixture_documentation(&ef.fixture, workspace_root),
                }));

                CompletionItem {
                    label: ef.fixture.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(ef.detail),
                    documentation,
                    insert_text: Some(ef.fixture.name.clone()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    sort_text: Some(ef.sort_text),
                    ..Default::default()
                }
            })
            .collect();

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
        let enriched =
            filter_and_enrich_fixtures(available, file_path, Some(declared_params), fixture_scope);

        // Get insertion info for adding new parameters
        let insertion_info = self
            .fixture_db
            .get_function_param_insertion_info(file_path, function_line);

        let items = enriched
            .into_iter()
            .map(|ef| {
                let documentation = Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: Self::format_fixture_documentation(&ef.fixture, workspace_root),
                }));

                // Create additional text edit to add the fixture as a parameter
                let additional_text_edits = insertion_info.as_ref().map(|info| {
                    let text = if info.needs_comma {
                        format!(", {}", ef.fixture.name)
                    } else {
                        ef.fixture.name.clone()
                    };
                    let lsp_line = Self::internal_line_to_lsp(info.line);
                    vec![TextEdit {
                        range: Self::create_point_range(lsp_line, info.char_pos as u32),
                        new_text: text,
                    }]
                });

                CompletionItem {
                    label: ef.fixture.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(ef.detail),
                    documentation,
                    insert_text: Some(ef.fixture.name.clone()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    additional_text_edits,
                    sort_text: Some(ef.sort_text),
                    ..Default::default()
                }
            })
            .collect();

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
        // No scope filtering for string completions — pass None for fixture_scope
        let enriched = filter_and_enrich_fixtures(available, file_path, None, None);

        let items = enriched
            .into_iter()
            .map(|ef| {
                let documentation = Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: Self::format_fixture_documentation(&ef.fixture, workspace_root),
                }));

                CompletionItem {
                    label: ef.fixture.name.clone(),
                    kind: Some(CompletionItemKind::TEXT),
                    detail: Some(ef.detail),
                    documentation,
                    // Don't add quotes - user is already inside a string
                    insert_text: Some(ef.fixture.name.clone()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    sort_text: Some(ef.sort_text),
                    ..Default::default()
                }
            })
            .collect();

        CompletionResponse::Array(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Unit tests for should_exclude_fixture
    // =========================================================================

    fn make_fixture(name: &str, scope: FixtureScope) -> FixtureDefinition {
        FixtureDefinition {
            name: name.to_string(),
            file_path: PathBuf::from("/tmp/test/conftest.py"),
            line: 1,
            end_line: 5,
            start_char: 4,
            end_char: 10,
            docstring: None,
            return_type: None,
            is_third_party: false,
            is_plugin: false,
            dependencies: vec![],
            scope,
            yield_line: None,
            autouse: false,
        }
    }

    #[test]
    fn test_should_exclude_fixture_test_function_allows_all() {
        // Test functions (None scope) should see all fixtures
        for scope in [
            FixtureScope::Function,
            FixtureScope::Class,
            FixtureScope::Module,
            FixtureScope::Package,
            FixtureScope::Session,
        ] {
            let fixture = make_fixture("f", scope);
            assert!(
                !should_exclude_fixture(&fixture, None),
                "Test function should allow {:?}-scoped fixture",
                scope
            );
        }
    }

    #[test]
    fn test_should_exclude_fixture_session_excludes_narrower() {
        let session_scope = Some(FixtureScope::Session);

        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Function),
            session_scope
        ));
        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Class),
            session_scope
        ));
        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Module),
            session_scope
        ));
        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Package),
            session_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Session),
            session_scope
        ));
    }

    #[test]
    fn test_should_exclude_fixture_module_excludes_narrower() {
        let module_scope = Some(FixtureScope::Module);

        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Function),
            module_scope
        ));
        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Class),
            module_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Module),
            module_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Package),
            module_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Session),
            module_scope
        ));
    }

    #[test]
    fn test_should_exclude_fixture_function_allows_all() {
        let function_scope = Some(FixtureScope::Function);

        for scope in [
            FixtureScope::Function,
            FixtureScope::Class,
            FixtureScope::Module,
            FixtureScope::Package,
            FixtureScope::Session,
        ] {
            assert!(
                !should_exclude_fixture(&make_fixture("f", scope), function_scope),
                "Function-scoped fixture should allow {:?}-scoped dependency",
                scope
            );
        }
    }

    #[test]
    fn test_should_exclude_fixture_class_excludes_function() {
        let class_scope = Some(FixtureScope::Class);

        assert!(should_exclude_fixture(
            &make_fixture("f", FixtureScope::Function),
            class_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Class),
            class_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Module),
            class_scope
        ));
        assert!(!should_exclude_fixture(
            &make_fixture("f", FixtureScope::Session),
            class_scope
        ));
    }

    // =========================================================================
    // Unit tests for is_fixture_excluded (combined filtering)
    // =========================================================================

    #[test]
    fn test_is_fixture_excluded_filters_self_cls() {
        let self_fixture = make_fixture("self", FixtureScope::Function);
        let cls_fixture = make_fixture("cls", FixtureScope::Function);
        let normal_fixture = make_fixture("db", FixtureScope::Function);

        assert!(is_fixture_excluded(&self_fixture, None, None));
        assert!(is_fixture_excluded(&cls_fixture, None, None));
        assert!(!is_fixture_excluded(&normal_fixture, None, None));
    }

    #[test]
    fn test_is_fixture_excluded_filters_declared_params() {
        let fixture = make_fixture("db", FixtureScope::Function);
        let declared = vec!["db".to_string()];

        assert!(is_fixture_excluded(&fixture, Some(&declared), None));
        assert!(!is_fixture_excluded(&fixture, None, None));
        assert!(!is_fixture_excluded(
            &fixture,
            Some(&["other".to_string()]),
            None
        ));
    }

    #[test]
    fn test_is_fixture_excluded_combines_scope_and_params() {
        let func_fixture = make_fixture("db", FixtureScope::Function);
        let session_scope = Some(FixtureScope::Session);
        let declared = vec!["db".to_string()];

        // Both reasons to exclude
        assert!(is_fixture_excluded(
            &func_fixture,
            Some(&declared),
            session_scope
        ));

        // Only scope excludes
        let undeclared: Vec<String> = vec![];
        assert!(is_fixture_excluded(
            &func_fixture,
            Some(&undeclared),
            session_scope
        ));

        // Only declared params exclude
        assert!(is_fixture_excluded(
            &make_fixture("db", FixtureScope::Session),
            Some(&declared),
            session_scope
        ));

        // Neither excludes
        assert!(!is_fixture_excluded(
            &make_fixture("other", FixtureScope::Session),
            Some(&undeclared),
            session_scope
        ));
    }

    // =========================================================================
    // Unit tests for filter_and_enrich_fixtures
    // =========================================================================

    #[test]
    fn test_filter_and_enrich_excludes_scope_incompatible() {
        let file_path = PathBuf::from("/tmp/test/test_file.py");
        let fixtures = vec![
            make_fixture("func_fix", FixtureScope::Function),
            make_fixture("class_fix", FixtureScope::Class),
            make_fixture("module_fix", FixtureScope::Module),
            make_fixture("session_fix", FixtureScope::Session),
        ];

        // Session-scoped fixture context: only session-scoped should survive
        let enriched = filter_and_enrich_fixtures(
            fixtures.clone(),
            &file_path,
            Some(&[]),
            Some(FixtureScope::Session),
        );
        let names: Vec<&str> = enriched.iter().map(|e| e.fixture.name.as_str()).collect();
        assert_eq!(names, vec!["session_fix"]);

        // Module-scoped fixture context: module, package, session should survive
        let enriched = filter_and_enrich_fixtures(
            fixtures.clone(),
            &file_path,
            Some(&[]),
            Some(FixtureScope::Module),
        );
        let names: Vec<&str> = enriched.iter().map(|e| e.fixture.name.as_str()).collect();
        assert_eq!(names, vec!["module_fix", "session_fix"]);

        // Function-scoped fixture context: all should survive
        let enriched = filter_and_enrich_fixtures(
            fixtures.clone(),
            &file_path,
            Some(&[]),
            Some(FixtureScope::Function),
        );
        assert_eq!(enriched.len(), 4);

        // Test function context (None scope): all should survive
        let enriched = filter_and_enrich_fixtures(fixtures.clone(), &file_path, Some(&[]), None);
        assert_eq!(enriched.len(), 4);
    }

    #[test]
    fn test_filter_and_enrich_excludes_declared_params() {
        let file_path = PathBuf::from("/tmp/test/test_file.py");
        let fixtures = vec![
            make_fixture("db", FixtureScope::Function),
            make_fixture("client", FixtureScope::Function),
            make_fixture("app", FixtureScope::Function),
        ];

        let declared = vec!["db".to_string(), "client".to_string()];
        let enriched = filter_and_enrich_fixtures(fixtures, &file_path, Some(&declared), None);
        let names: Vec<&str> = enriched.iter().map(|e| e.fixture.name.as_str()).collect();
        assert_eq!(names, vec!["app"]);
    }

    #[test]
    fn test_filter_and_enrich_excludes_self_cls() {
        let file_path = PathBuf::from("/tmp/test/test_file.py");
        let mut fixtures = vec![
            make_fixture("self", FixtureScope::Function),
            make_fixture("cls", FixtureScope::Function),
            make_fixture("real_fixture", FixtureScope::Function),
        ];
        // Make self/cls look like they came from somewhere
        fixtures[0].name = "self".to_string();
        fixtures[1].name = "cls".to_string();

        let enriched = filter_and_enrich_fixtures(fixtures, &file_path, None, None);
        let names: Vec<&str> = enriched.iter().map(|e| e.fixture.name.as_str()).collect();
        assert_eq!(names, vec!["real_fixture"]);
    }

    // =========================================================================
    // Unit tests for fixture_sort_priority
    // =========================================================================

    #[test]
    fn test_fixture_sort_priority_same_file() {
        let current = PathBuf::from("/tmp/test/test_file.py");
        let mut fixture = make_fixture("f", FixtureScope::Function);
        fixture.file_path = current.clone();

        assert_eq!(fixture_sort_priority(&fixture, &current), 0);
    }

    #[test]
    fn test_fixture_sort_priority_conftest() {
        let current = PathBuf::from("/tmp/test/test_file.py");
        let mut fixture = make_fixture("f", FixtureScope::Function);
        fixture.file_path = PathBuf::from("/tmp/test/conftest.py");

        assert_eq!(fixture_sort_priority(&fixture, &current), 1);
    }

    #[test]
    fn test_fixture_sort_priority_plugin() {
        let current = PathBuf::from("/tmp/test/test_file.py");
        let mut fixture = make_fixture("f", FixtureScope::Function);
        fixture.file_path = PathBuf::from("/tmp/other/plugin.py");
        fixture.is_plugin = true;

        assert_eq!(fixture_sort_priority(&fixture, &current), 2);
    }

    #[test]
    fn test_fixture_sort_priority_third_party() {
        let current = PathBuf::from("/tmp/test/test_file.py");
        let mut fixture = make_fixture("f", FixtureScope::Function);
        fixture.file_path = PathBuf::from("/tmp/venv/lib/site-packages/pkg/fix.py");
        fixture.is_third_party = true;

        assert_eq!(fixture_sort_priority(&fixture, &current), 3);
    }

    #[test]
    fn test_fixture_sort_priority_third_party_trumps_plugin() {
        let current = PathBuf::from("/tmp/test/test_file.py");
        let mut fixture = make_fixture("f", FixtureScope::Function);
        fixture.file_path = PathBuf::from("/tmp/venv/lib/site-packages/pkg/fix.py");
        fixture.is_third_party = true;
        fixture.is_plugin = true;

        // Third-party check comes first, so priority is 3
        assert_eq!(fixture_sort_priority(&fixture, &current), 3);
    }

    // =========================================================================
    // Unit tests for make_fixture_detail
    // =========================================================================

    #[test]
    fn test_make_fixture_detail_default_scope() {
        let fixture = make_fixture("f", FixtureScope::Function);
        let detail = make_fixture_detail(&fixture);
        assert_eq!(detail, "");
    }

    #[test]
    fn test_make_fixture_detail_session_scope() {
        let fixture = make_fixture("f", FixtureScope::Session);
        let detail = make_fixture_detail(&fixture);
        assert_eq!(detail, "(session)");
    }

    #[test]
    fn test_make_fixture_detail_third_party() {
        let mut fixture = make_fixture("f", FixtureScope::Function);
        fixture.is_third_party = true;
        let detail = make_fixture_detail(&fixture);
        assert_eq!(detail, "[third-party]");
    }

    #[test]
    fn test_make_fixture_detail_plugin_with_scope() {
        let mut fixture = make_fixture("f", FixtureScope::Module);
        fixture.is_plugin = true;
        let detail = make_fixture_detail(&fixture);
        assert_eq!(detail, "(module) [plugin]");
    }

    #[test]
    fn test_make_fixture_detail_third_party_overrides_plugin() {
        let mut fixture = make_fixture("f", FixtureScope::Session);
        fixture.is_third_party = true;
        fixture.is_plugin = true;
        let detail = make_fixture_detail(&fixture);
        // Third-party tag takes precedence
        assert_eq!(detail, "(session) [third-party]");
    }

    // =========================================================================
    // Unit tests for make_sort_text
    // =========================================================================

    #[test]
    fn test_make_sort_text_ordering() {
        let same_file = make_sort_text(0, "zzz");
        let conftest = make_sort_text(1, "aaa");
        let third_party = make_sort_text(3, "aaa");

        // Same-file should sort before conftest even with later alpha name
        assert!(same_file < conftest);
        // Conftest should sort before third-party
        assert!(conftest < third_party);
    }

    #[test]
    fn test_make_sort_text_alpha_within_group() {
        let a = make_sort_text(1, "alpha");
        let b = make_sort_text(1, "beta");
        assert!(a < b);
    }
}
