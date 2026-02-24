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
    current_fixture_name: Option<&str>,
) -> bool {
    // Skip special parameter names
    if EXCLUDED_PARAM_NAMES.contains(&fixture.name.as_str()) {
        return true;
    }

    // Skip the fixture currently being edited (don't suggest yourself)
    if let Some(name) = current_fixture_name {
        if fixture.name == name {
            return true;
        }
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
    current_fixture_name: Option<&str>,
) -> Vec<EnrichedFixture> {
    available
        .into_iter()
        .filter(|f| !is_fixture_excluded(f, declared_params, fixture_scope, current_fixture_name))
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
                        function_name,
                        is_fixture,
                        declared_params,
                        fixture_scope,
                        ..
                    } => {
                        // In function signature - suggest fixtures as parameters (filter already declared)
                        // When editing a fixture, exclude itself from suggestions
                        let current_fixture_name = if is_fixture {
                            Some(function_name.as_str())
                        } else {
                            None
                        };
                        return Ok(Some(self.create_fixture_completions(
                            &file_path,
                            &declared_params,
                            workspace_root.as_ref(),
                            fixture_scope,
                            current_fixture_name,
                        )));
                    }
                    CompletionContext::FunctionBody {
                        function_name,
                        function_line,
                        is_fixture,
                        declared_params,
                        fixture_scope,
                        ..
                    } => {
                        // In function body - suggest fixtures with auto-add to parameters
                        let current_fixture_name = if is_fixture {
                            Some(function_name.as_str())
                        } else {
                            None
                        };
                        return Ok(Some(self.create_fixture_completions_with_auto_add(
                            &file_path,
                            &declared_params,
                            function_line,
                            workspace_root.as_ref(),
                            fixture_scope,
                            current_fixture_name,
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
        current_fixture_name: Option<&str>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let enriched = filter_and_enrich_fixtures(
            available,
            file_path,
            Some(declared_params),
            fixture_scope,
            current_fixture_name,
        );

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
        current_fixture_name: Option<&str>,
    ) -> CompletionResponse {
        let available = self.fixture_db.get_available_fixtures(file_path);
        let enriched = filter_and_enrich_fixtures(
            available,
            file_path,
            Some(declared_params),
            fixture_scope,
            current_fixture_name,
        );

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
        let enriched = filter_and_enrich_fixtures(available, file_path, None, None, None);

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

        assert!(is_fixture_excluded(&self_fixture, None, None, None));
        assert!(is_fixture_excluded(&cls_fixture, None, None, None));
        assert!(!is_fixture_excluded(&normal_fixture, None, None, None));
    }

    #[test]
    fn test_is_fixture_excluded_filters_declared_params() {
        let fixture = make_fixture("db", FixtureScope::Function);
        let declared = vec!["db".to_string()];

        assert!(is_fixture_excluded(&fixture, Some(&declared), None, None));
        assert!(!is_fixture_excluded(&fixture, None, None, None));
        assert!(!is_fixture_excluded(
            &fixture,
            Some(&["other".to_string()]),
            None,
            None,
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
            session_scope,
            None,
        ));

        // Only scope excludes
        let undeclared: Vec<String> = vec![];
        assert!(is_fixture_excluded(
            &func_fixture,
            Some(&undeclared),
            session_scope,
            None,
        ));

        // Only declared params exclude
        assert!(is_fixture_excluded(
            &make_fixture("db", FixtureScope::Session),
            Some(&declared),
            session_scope,
            None,
        ));

        // Neither excludes
        assert!(!is_fixture_excluded(
            &make_fixture("other", FixtureScope::Session),
            Some(&undeclared),
            session_scope,
            None,
        ));
    }

    // =========================================================================
    // Unit tests for filter_and_enrich_fixtures
    // =========================================================================

    #[test]
    fn test_filter_and_enrich_excludes_current_fixture() {
        let file = std::path::Path::new("/tmp/test/conftest.py");
        let fixtures = vec![
            make_fixture("my_fixture", FixtureScope::Function),
            make_fixture("other_fixture", FixtureScope::Function),
        ];

        // When editing my_fixture, it should be excluded
        let enriched = filter_and_enrich_fixtures(
            fixtures.clone(),
            file,
            None,
            Some(FixtureScope::Function),
            Some("my_fixture"),
        );
        assert_eq!(enriched.len(), 1);
        assert_eq!(enriched[0].fixture.name, "other_fixture");

        // When editing a test (no current_fixture_name), both should be included
        let enriched = filter_and_enrich_fixtures(fixtures, file, None, None, None);
        assert_eq!(enriched.len(), 2);
    }

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
            None,
        );
        let names: Vec<&str> = enriched.iter().map(|e| e.fixture.name.as_str()).collect();
        assert_eq!(names, vec!["session_fix"]);

        // Module-scoped fixture context: module, package, session should survive
        let enriched = filter_and_enrich_fixtures(
            fixtures.clone(),
            &file_path,
            Some(&[]),
            Some(FixtureScope::Module),
            None,
        );
        let names: Vec<&str> = enriched.iter().map(|e| e.fixture.name.as_str()).collect();
        assert_eq!(names, vec!["module_fix", "session_fix"]);

        // Function-scoped fixture context: all should survive
        let enriched = filter_and_enrich_fixtures(
            fixtures.clone(),
            &file_path,
            Some(&[]),
            Some(FixtureScope::Function),
            None,
        );
        assert_eq!(enriched.len(), 4);

        // Test function context (None scope): all should survive
        let enriched =
            filter_and_enrich_fixtures(fixtures.clone(), &file_path, Some(&[]), None, None);
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
        let enriched =
            filter_and_enrich_fixtures(fixtures, &file_path, Some(&declared), None, None);
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

        let enriched = filter_and_enrich_fixtures(fixtures, &file_path, None, None, None);
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

    // =========================================================================
    // Integration tests for Backend completion methods
    // =========================================================================

    use crate::fixtures::FixtureDatabase;
    use std::sync::Arc;
    use tower_lsp_server::LspService;

    /// Create a Backend instance for testing by using LspService to obtain a Client.
    /// We capture a clone of the Backend from inside the LspService::new closure.
    fn make_backend_with_db(db: Arc<FixtureDatabase>) -> Backend {
        let backend_slot: Arc<std::sync::Mutex<Option<Backend>>> =
            Arc::new(std::sync::Mutex::new(None));
        let slot_clone = backend_slot.clone();
        let (_svc, _sock) = LspService::new(move |client| {
            let b = Backend::new(client, db.clone());
            // Clone all Arc fields to capture a usable Backend outside
            *slot_clone.lock().unwrap() = Some(Backend {
                client: b.client.clone(),
                fixture_db: b.fixture_db.clone(),
                workspace_root: b.workspace_root.clone(),
                original_workspace_root: b.original_workspace_root.clone(),
                scan_task: b.scan_task.clone(),
                uri_cache: b.uri_cache.clone(),
                config: b.config.clone(),
            });
            b
        });
        let result = backend_slot
            .lock()
            .unwrap()
            .take()
            .expect("Backend should have been created");
        result
    }

    /// Helper: build a test Backend with fixtures pre-loaded.
    fn setup_backend_with_fixtures() -> (Backend, PathBuf) {
        let db = Arc::new(FixtureDatabase::new());

        let conftest_content = r#"
import pytest

@pytest.fixture
def func_fixture():
    return "func"

@pytest.fixture(scope="session")
def session_fixture():
    """A session-scoped fixture."""
    return "session"

@pytest.fixture(scope="module")
def module_fixture():
    return "module"
"#;

        let test_content = r#"
import pytest

@pytest.fixture(scope="session")
def local_session_fixture():
    pass

def test_something(func_fixture):
    pass
"#;

        let conftest_path = PathBuf::from("/tmp/test_backend/conftest.py");
        let test_path = PathBuf::from("/tmp/test_backend/test_example.py");

        db.analyze_file(conftest_path, conftest_content);
        db.analyze_file(test_path.clone(), test_content);

        let backend = make_backend_with_db(db);
        (backend, test_path)
    }

    fn extract_items(response: &CompletionResponse) -> &Vec<CompletionItem> {
        match response {
            CompletionResponse::Array(items) => items,
            _ => panic!("Expected CompletionResponse::Array"),
        }
    }

    // ---- create_fixture_completions ----

    #[test]
    fn test_create_fixture_completions_returns_items() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        let response = backend.create_fixture_completions(&test_path, &declared, None, None, None);
        let items = extract_items(&response);
        assert!(!items.is_empty(), "Should return completion items");
        // All items should have VARIABLE kind
        for item in items {
            assert_eq!(item.kind, Some(CompletionItemKind::VARIABLE));
            assert!(item.insert_text.is_some());
            assert!(item.sort_text.is_some());
            assert!(item.detail.is_some());
        }
    }

    #[test]
    fn test_create_fixture_completions_filters_declared() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec!["func_fixture".to_string()];
        let response = backend.create_fixture_completions(&test_path, &declared, None, None, None);
        let items = extract_items(&response);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            !labels.contains(&"func_fixture"),
            "func_fixture should be filtered out since it's declared"
        );
    }

    #[test]
    fn test_create_fixture_completions_scope_filtering() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        // Session scope: only session-scoped fixtures should appear
        let response = backend.create_fixture_completions(
            &test_path,
            &declared,
            None,
            Some(FixtureScope::Session),
            None,
        );
        let items = extract_items(&response);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            !labels.contains(&"func_fixture"),
            "func_fixture should be excluded for session scope"
        );
        assert!(
            labels.contains(&"session_fixture"),
            "session_fixture should be included, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_create_fixture_completions_detail_and_sort() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        let response = backend.create_fixture_completions(&test_path, &declared, None, None, None);
        let items = extract_items(&response);

        // Find the session_fixture — it should have scope in detail
        let session_item = items.iter().find(|i| i.label == "session_fixture");
        assert!(session_item.is_some(), "Should find session_fixture");
        let session_item = session_item.unwrap();
        assert!(
            session_item.detail.as_ref().unwrap().contains("session"),
            "session_fixture detail should contain scope, got: {:?}",
            session_item.detail
        );

        // Find the func_fixture — default scope should not appear
        let func_item = items.iter().find(|i| i.label == "func_fixture");
        assert!(func_item.is_some(), "Should find func_fixture");
        let func_item = func_item.unwrap();
        assert!(
            !func_item.detail.as_ref().unwrap().contains("function"),
            "func_fixture detail should not contain 'function' (default scope), got: {:?}",
            func_item.detail
        );
    }

    #[test]
    fn test_create_fixture_completions_documentation() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        let response = backend.create_fixture_completions(&test_path, &declared, None, None, None);
        let items = extract_items(&response);

        // All items should have documentation
        for item in items {
            assert!(
                item.documentation.is_some(),
                "Item '{}' should have documentation",
                item.label
            );
        }
    }

    #[test]
    fn test_create_fixture_completions_with_workspace_root() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        let workspace_root = PathBuf::from("/tmp/test_backend");
        let response = backend.create_fixture_completions(
            &test_path,
            &declared,
            Some(&workspace_root),
            None,
            None,
        );
        let items = extract_items(&response);
        assert!(!items.is_empty());
    }

    // ---- create_fixture_completions_with_auto_add ----

    #[test]
    fn test_create_fixture_completions_with_auto_add_returns_items() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        // function_line is 1-based internal line of `def test_something(func_fixture):`
        // In test_content, test_something is at line 8 (1-indexed)
        let response = backend.create_fixture_completions(&test_path, &declared, None, None, None);
        let items = extract_items(&response);
        assert!(!items.is_empty(), "Should return completion items");
        for item in items {
            assert_eq!(item.kind, Some(CompletionItemKind::VARIABLE));
            assert!(item.sort_text.is_some());
            assert!(item.detail.is_some());
        }
    }

    #[test]
    fn test_create_fixture_completions_with_auto_add_has_text_edits() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec!["func_fixture".to_string()];
        // Line 8 has: def test_something(func_fixture):
        let response = backend
            .create_fixture_completions_with_auto_add(&test_path, &declared, 8, None, None, None);
        let items = extract_items(&response);
        // Items should have additional_text_edits to add parameter
        for item in items {
            assert!(
                item.additional_text_edits.is_some(),
                "Item '{}' should have additional_text_edits for auto-add",
                item.label
            );
            let edits = item.additional_text_edits.as_ref().unwrap();
            assert_eq!(edits.len(), 1, "Should have exactly one text edit");
        }
    }

    #[test]
    fn test_create_fixture_completions_with_auto_add_scope_filter() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec![];
        let response = backend.create_fixture_completions_with_auto_add(
            &test_path,
            &declared,
            8,
            None,
            Some(FixtureScope::Session),
            None,
        );
        let items = extract_items(&response);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            !labels.contains(&"func_fixture"),
            "func_fixture should be excluded for session scope"
        );
    }

    #[test]
    fn test_create_fixture_completions_with_auto_add_filters_declared() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let declared = vec!["session_fixture".to_string(), "func_fixture".to_string()];
        let response = backend
            .create_fixture_completions_with_auto_add(&test_path, &declared, 8, None, None, None);
        let items = extract_items(&response);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            !labels.contains(&"func_fixture"),
            "func_fixture should be filtered"
        );
        assert!(
            !labels.contains(&"session_fixture"),
            "session_fixture should be filtered"
        );
    }

    #[test]
    fn test_create_fixture_completions_with_auto_add_filters_current_fixture() {
        let (backend, file_path) = setup_backend_with_fixtures();
        // When editing func_fixture, it should not appear in completions
        let response = backend.create_fixture_completions(
            &file_path,
            &[],
            None,
            Some(FixtureScope::Function),
            Some("func_fixture"),
        );
        let items = extract_items(&response);
        assert!(
            !items.iter().any(|i| i.label == "func_fixture"),
            "Current fixture should be excluded from completions"
        );
        // Other fixtures should still appear
        assert!(items.iter().any(|i| i.label == "session_fixture"));
    }

    #[test]
    fn test_create_fixture_completions_with_auto_add_no_existing_params() {
        // Test the needs_comma = false branch: function with no existing parameters
        let db = Arc::new(FixtureDatabase::new());

        let conftest_content = r#"
import pytest

@pytest.fixture
def db_fixture():
    return "db"
"#;

        let test_content = r#"
def test_empty_params():
    pass
"#;

        let conftest_path = PathBuf::from("/tmp/test_no_params/conftest.py");
        let test_path = PathBuf::from("/tmp/test_no_params/test_file.py");

        db.analyze_file(conftest_path, conftest_content);
        db.analyze_file(test_path.clone(), test_content);

        let backend = make_backend_with_db(db);
        let declared: Vec<String> = vec![];
        // Line 2 (1-indexed) is `def test_empty_params():`
        let response = backend
            .create_fixture_completions_with_auto_add(&test_path, &declared, 2, None, None, None);
        let items = extract_items(&response);
        assert!(!items.is_empty(), "Should return completion items");

        // The text edit should NOT have a comma since there are no existing params
        let item = items.iter().find(|i| i.label == "db_fixture");
        assert!(item.is_some(), "Should find db_fixture");
        let item = item.unwrap();
        let edits = item.additional_text_edits.as_ref().unwrap();
        assert_eq!(edits.len(), 1);
        // The new_text should be just the fixture name (no comma prefix)
        assert_eq!(
            edits[0].new_text, "db_fixture",
            "Should insert fixture name without comma for empty params"
        );
    }

    // ---- create_string_fixture_completions ----

    #[test]
    fn test_create_string_fixture_completions_returns_items() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let response = backend.create_string_fixture_completions(&test_path, None);
        let items = extract_items(&response);
        assert!(!items.is_empty(), "Should return string completion items");
        // String completions use TEXT kind
        for item in items {
            assert_eq!(
                item.kind,
                Some(CompletionItemKind::TEXT),
                "String completions should use TEXT kind"
            );
            assert!(item.sort_text.is_some());
            assert!(item.detail.is_some());
            assert!(item.documentation.is_some());
        }
    }

    #[test]
    fn test_create_string_fixture_completions_no_scope_filtering() {
        let (backend, test_path) = setup_backend_with_fixtures();
        // String completions should NOT filter by scope
        let response = backend.create_string_fixture_completions(&test_path, None);
        let items = extract_items(&response);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Both function and session scoped fixtures should be present
        assert!(
            labels.contains(&"func_fixture"),
            "func_fixture should be in string completions, got: {:?}",
            labels
        );
        assert!(
            labels.contains(&"session_fixture"),
            "session_fixture should be in string completions, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_create_string_fixture_completions_with_workspace_root() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let workspace_root = PathBuf::from("/tmp/test_backend");
        let response = backend.create_string_fixture_completions(&test_path, Some(&workspace_root));
        let items = extract_items(&response);
        assert!(!items.is_empty());
    }

    #[test]
    fn test_create_string_fixture_completions_has_detail_and_sort() {
        let (backend, test_path) = setup_backend_with_fixtures();
        let response = backend.create_string_fixture_completions(&test_path, None);
        let items = extract_items(&response);

        let session_item = items.iter().find(|i| i.label == "session_fixture");
        assert!(session_item.is_some());
        let session_item = session_item.unwrap();
        assert!(
            session_item.detail.as_ref().unwrap().contains("session"),
            "session_fixture should have scope in detail"
        );
        // sort_text should be present and start with priority digit
        let sort = session_item.sort_text.as_ref().unwrap();
        assert!(
            sort.starts_with('1') || sort.starts_with('0'),
            "Sort text should start with priority digit, got: {}",
            sort
        );
    }

    // ---- Edge case: empty fixture database ----

    #[test]
    fn test_create_fixture_completions_empty_db() {
        let db = Arc::new(FixtureDatabase::new());
        let backend = make_backend_with_db(db);
        let path = PathBuf::from("/tmp/empty/test_file.py");
        let response = backend.create_fixture_completions(&path, &[], None, None, None);
        let items = extract_items(&response);
        assert!(items.is_empty(), "Empty DB should return no completions");
    }

    #[test]
    fn test_create_fixture_completions_with_auto_add_empty_db() {
        let db = Arc::new(FixtureDatabase::new());
        let backend = make_backend_with_db(db);
        let path = PathBuf::from("/tmp/empty/test_file.py");
        let response =
            backend.create_fixture_completions_with_auto_add(&path, &[], 1, None, None, None);
        let items = extract_items(&response);
        assert!(items.is_empty(), "Empty DB should return no completions");
    }

    #[test]
    fn test_create_string_fixture_completions_empty_db() {
        let db = Arc::new(FixtureDatabase::new());
        let backend = make_backend_with_db(db);
        let path = PathBuf::from("/tmp/empty/test_file.py");
        let response = backend.create_string_fixture_completions(&path, None);
        let items = extract_items(&response);
        assert!(items.is_empty(), "Empty DB should return no completions");
    }
}
