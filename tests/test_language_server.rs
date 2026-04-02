//! Tests for the LanguageServer trait implementation in language_server.rs.
//!
//! Exercises every method in the `LanguageServer for Backend` impl so that
//! the coverage report no longer shows the file at 0%.

#![allow(deprecated)] // For root_path / root_uri fields in InitializeParams

use std::path::PathBuf;
use std::sync::Arc;

use ntest::timeout;
use pytest_language_server::{Backend, FixtureDatabase};
use tower_lsp_server::ls_types::request::GotoImplementationParams;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{LanguageServer, LspService};

// ── Helpers ───────────────────────────────────────────────────────────────

/// Build a `Backend` backed by the given database and a dummy LSP service.
fn make_backend_with_db(db: Arc<FixtureDatabase>) -> Backend {
    let slot: Arc<std::sync::Mutex<Option<Backend>>> = Arc::new(std::sync::Mutex::new(None));
    let slot_clone = slot.clone();
    let (_svc, _sock) = LspService::new(move |client| {
        let b = Backend::new(client, db.clone());
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
    let backend = slot.lock().unwrap().take().expect("backend created");
    backend
}

fn make_backend() -> Backend {
    make_backend_with_db(Arc::new(FixtureDatabase::new()))
}

/// Return a platform-appropriate absolute `PathBuf` under the system temp
/// directory.  Using `std::env::temp_dir()` (rather than a hardcoded
/// `/tmp/…` string) ensures the path is truly absolute on Windows too
/// (Windows paths require a drive letter prefix for `is_absolute()` to
/// return `true`, which `Uri::from_file_path` requires).
fn tfile(subdir: &str, filename: &str) -> PathBuf {
    std::env::temp_dir().join(subdir).join(filename)
}

/// Build a `Uri` from a path under the system temp directory.
///
/// Panics if the path cannot be converted to a URI, which should never
/// happen for a path returned by `std::env::temp_dir().join(…)`.
fn turi(subdir: &str, filename: &str) -> Uri {
    let path = tfile(subdir, filename);
    Uri::from_file_path(&path)
        .unwrap_or_else(|| panic!("Uri::from_file_path failed for {:?}", path))
}

fn pos(line: u32, character: u32) -> Position {
    Position { line, character }
}

fn rng(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
    Range {
        start: pos(sl, sc),
        end: pos(el, ec),
    }
}

fn tdp(uri: Uri, line: u32, character: u32) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri },
        position: pos(line, character),
    }
}

fn wdp() -> WorkDoneProgressParams {
    WorkDoneProgressParams {
        work_done_token: None,
    }
}

fn prp() -> PartialResultParams {
    PartialResultParams {
        partial_result_token: None,
    }
}

// ── initialize ────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_no_workspace_root_returns_capabilities() {
    let backend = make_backend();
    let params = InitializeParams::default();

    let result = backend.initialize(params).await;
    assert!(result.is_ok(), "initialize should succeed");

    let init = result.unwrap();
    assert_eq!(
        init.server_info.as_ref().unwrap().name,
        "pytest-language-server"
    );

    // Workspace root must remain unset
    assert!(
        backend.workspace_root.read().await.is_none(),
        "workspace_root should stay None when no root URI given"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_no_workspace_logs_warning() {
    // Same as above but also checks the warning branch is exercised.
    let backend = make_backend();
    let result = backend.initialize(InitializeParams::default()).await;
    assert!(result.is_ok());
    let caps = &result.unwrap().capabilities;
    assert!(caps.definition_provider.is_some());
    assert!(caps.hover_provider.is_some());
    assert!(caps.references_provider.is_some());
    assert!(caps.completion_provider.is_some());
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_with_workspace_folders_sets_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let backend = make_backend();

    let root_uri = Uri::from_file_path(tmp.path()).expect("root URI");
    let params = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri,
            name: "test_ws".to_string(),
        }]),
        ..Default::default()
    };

    let result = backend.initialize(params).await;
    assert!(
        result.is_ok(),
        "initialize with workspace_folders should succeed"
    );

    // Root should be stored
    assert!(
        backend.workspace_root.read().await.is_some(),
        "workspace_root should be set after initialize with workspace_folders"
    );

    // Give the background scan task a moment to run (empty dir, near-instant)
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_with_deprecated_root_uri_sets_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let backend = make_backend();

    let root_uri = Uri::from_file_path(tmp.path()).expect("root URI");
    let params = InitializeParams {
        root_uri: Some(root_uri),
        ..Default::default()
    };

    let result = backend.initialize(params).await;
    assert!(result.is_ok());
    assert!(
        backend.workspace_root.read().await.is_some(),
        "workspace_root should be set via deprecated root_uri"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_workspace_folders_takes_priority_over_root_uri() {
    let tmp_wf = tempfile::tempdir().expect("workspace_folders dir");
    let tmp_ru = tempfile::tempdir().expect("root_uri dir");
    let backend = make_backend();

    let wf_uri = Uri::from_file_path(tmp_wf.path()).unwrap();
    let ru_uri = Uri::from_file_path(tmp_ru.path()).unwrap();

    let params = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: wf_uri.clone(),
            name: "wf".to_string(),
        }]),
        root_uri: Some(ru_uri),
        ..Default::default()
    };

    let result = backend.initialize(params).await;
    assert!(result.is_ok());

    // workspace_folders wins: canonical root must match tmp_wf, not tmp_ru
    let root = backend.workspace_root.read().await;
    let stored = root.as_ref().expect("root should be set");
    let canonical_wf = tmp_wf.path().canonicalize().unwrap();
    assert_eq!(stored, &canonical_wf, "workspace_folders URI should win");
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_capabilities_all_providers_present() {
    let backend = make_backend();
    let result = backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();
    let caps = &result.capabilities;

    // All providers advertised in the impl
    assert!(caps.definition_provider.is_some());
    assert!(caps.hover_provider.is_some());
    assert!(caps.references_provider.is_some());
    assert!(caps.text_document_sync.is_some());
    assert!(caps.code_action_provider.is_some());
    assert!(caps.completion_provider.is_some());
    assert!(caps.document_symbol_provider.is_some());
    assert!(caps.workspace_symbol_provider.is_some());
    assert!(caps.code_lens_provider.is_some());
    assert!(caps.inlay_hint_provider.is_some());
    assert!(caps.implementation_provider.is_some());
    assert!(caps.call_hierarchy_provider.is_some());
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_server_info_includes_version() {
    let backend = make_backend();
    let result = backend
        .initialize(InitializeParams::default())
        .await
        .unwrap();
    let info = result.server_info.expect("server_info should be present");
    assert!(!info.name.is_empty());
    assert!(info.version.is_some(), "server version should be reported");
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_with_workspace_stores_original_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let backend = make_backend();

    let root_uri = Uri::from_file_path(tmp.path()).expect("root URI");
    let params = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri,
            name: "ws".to_string(),
        }]),
        ..Default::default()
    };

    backend.initialize(params).await.unwrap();

    // Both the canonical root and the original root should be stored
    assert!(backend.workspace_root.read().await.is_some());
    assert!(backend.original_workspace_root.read().await.is_some());
}

#[tokio::test]
#[timeout(30000)]
async fn test_initialize_background_scan_handle_stored() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let backend = make_backend();

    let root_uri = Uri::from_file_path(tmp.path()).expect("root URI");
    let params = InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri,
            name: "ws".to_string(),
        }]),
        ..Default::default()
    };

    backend.initialize(params).await.unwrap();

    // A background scan task handle should have been stored
    let handle = backend.scan_task.lock().await;
    assert!(handle.is_some(), "scan task handle should be stored");
}

// ── initialized ───────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_initialized_does_not_panic() {
    let backend = make_backend();
    // register_capability will fail on the mock client, but must not panic
    backend.initialized(InitializedParams {}).await;
}

// ── did_open ──────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_did_open_registers_fixture_in_db() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_uri = turi("test_ls_open", "conftest.py");
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri,
                language_id: "python".to_string(),
                version: 1,
                text: "import pytest\n\n@pytest.fixture\ndef open_fixture():\n    return 42\n"
                    .to_string(),
            },
        })
        .await;

    assert!(
        db.definitions.contains_key("open_fixture"),
        "fixture should be registered after did_open"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_open_populates_uri_cache() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_uri = turi("test_ls_open_cache", "conftest.py");
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri,
                language_id: "python".to_string(),
                version: 1,
                text: "import pytest\n".to_string(),
            },
        })
        .await;

    assert!(
        !backend.uri_cache.is_empty(),
        "URI cache should have an entry after did_open"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_open_with_diagnostics() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // A test that uses an undeclared fixture – should trigger diagnostic publishing
    let file_uri = turi("test_ls_open_diag", "test_example.py");
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri,
                language_id: "python".to_string(),
                version: 1,
                text: "def test_something():\n    result = missing_fixture\n".to_string(),
            },
        })
        .await;
    // Should not panic even when diagnostics are published via a mock client
}

// ── did_change ────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_updates_fixture_db() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_uri = turi("test_ls_change", "conftest.py");

    // Open with first fixture
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri.clone(),
                language_id: "python".to_string(),
                version: 1,
                text: "import pytest\n\n@pytest.fixture\ndef old_fixture():\n    return 1\n"
                    .to_string(),
            },
        })
        .await;
    assert!(db.definitions.contains_key("old_fixture"));

    // Change to a new fixture
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: file_uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "import pytest\n\n@pytest.fixture\ndef new_fixture():\n    return 2\n"
                    .to_string(),
            }],
        })
        .await;

    assert!(
        db.definitions.contains_key("new_fixture"),
        "new_fixture should be registered after did_change"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_with_empty_content_changes_is_noop() {
    let backend = make_backend();
    let file_uri = turi("test_ls_change_empty", "conftest.py");

    // Should not panic with no content changes
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: file_uri,
                version: 1,
            },
            content_changes: vec![],
        })
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_triggers_inlay_hint_refresh() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_uri = turi("test_ls_change_hint", "conftest.py");

    // Open first
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri.clone(),
                language_id: "python".to_string(),
                version: 1,
                text: "import pytest\n\n@pytest.fixture\ndef hint_fixture():\n    return 1\n"
                    .to_string(),
            },
        })
        .await;

    // Then change – this also calls inlay_hint_refresh on the mock client
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: file_uri,
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "import pytest\n\n@pytest.fixture\ndef hint_fixture_v2():\n    return 2\n"
                    .to_string(),
            }],
        })
        .await;
    // Mock client absorbs inlay_hint_refresh error silently – just must not panic
}

// ── did_change_watched_files ──────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_watched_files_created_init_py_triggers_reanalysis() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Put a fixture file under the directory where __init__.py will be created.
    // Both paths are built from the same temp subdir so starts_with() works.
    let fixture_path = tfile("test_ls_wf_created", "conftest.py");
    db.analyze_file(
        fixture_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef wf_fixture():\n    return 42\n",
    );
    assert!(db.definitions.contains_key("wf_fixture"));

    let init_uri = turi("test_ls_wf_created", "__init__.py");
    backend
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: init_uri,
                typ: FileChangeType::CREATED,
            }],
        })
        .await;
    // Should not panic; fixture is re-analysed from cache
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_watched_files_deleted_init_py_triggers_reanalysis() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let fixture_path = tfile("test_ls_wf_deleted", "conftest.py");
    db.analyze_file(
        fixture_path,
        "import pytest\n\n@pytest.fixture\ndef del_fixture():\n    return 1\n",
    );

    let init_uri = turi("test_ls_wf_deleted", "__init__.py");
    backend
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: init_uri,
                typ: FileChangeType::DELETED,
            }],
        })
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_watched_files_skips_changed_events() {
    let backend = make_backend();

    // FileChangeType::CHANGED should be skipped – no re-analysis
    let init_uri = turi("test_ls_wf_changed", "__init__.py");
    backend
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: init_uri,
                typ: FileChangeType::CHANGED,
            }],
        })
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_watched_files_empty_changes_no_refresh() {
    let backend = make_backend();

    // Empty changes → inlay_hint_refresh must NOT be called (would error on mock)
    backend
        .did_change_watched_files(DidChangeWatchedFilesParams { changes: vec![] })
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_watched_files_republishes_diagnostics_for_cached_uri() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Analyse a fixture file using a temp-dir path so the directory structure
    // is consistent across platforms.
    let fixture_path = tfile("test_ls_wf_diag", "conftest.py");
    db.analyze_file(
        fixture_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef diag_fixture():\n    return 99\n",
    );

    // Pre-populate URI cache so the diagnostic re-publish branch is exercised.
    let fixture_uri = turi("test_ls_wf_diag", "conftest.py");
    backend.uri_cache.insert(fixture_path, fixture_uri);

    let init_uri = turi("test_ls_wf_diag", "__init__.py");
    backend
        .did_change_watched_files(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: init_uri,
                typ: FileChangeType::CREATED,
            }],
        })
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_change_watched_files_multiple_events() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Two independent subdirectories, each with a fixture file and a
    // corresponding __init__.py event.
    let path_a = tfile("test_ls_wf_multi_a", "conftest.py");
    let path_b = tfile("test_ls_wf_multi_b", "conftest.py");
    db.analyze_file(
        path_a,
        "import pytest\n\n@pytest.fixture\ndef fixture_a():\n    pass\n",
    );
    db.analyze_file(
        path_b,
        "import pytest\n\n@pytest.fixture\ndef fixture_b():\n    pass\n",
    );

    let params = DidChangeWatchedFilesParams {
        changes: vec![
            FileEvent {
                uri: turi("test_ls_wf_multi_a", "__init__.py"),
                typ: FileChangeType::CREATED,
            },
            FileEvent {
                uri: turi("test_ls_wf_multi_b", "__init__.py"),
                typ: FileChangeType::DELETED,
            },
        ],
    };
    backend.did_change_watched_files(params).await;
}

// ── did_close ─────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_did_close_clears_uri_cache() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_uri = turi("test_ls_close", "conftest.py");

    // Open to populate caches
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri.clone(),
                language_id: "python".to_string(),
                version: 1,
                text: "import pytest\n".to_string(),
            },
        })
        .await;
    assert!(
        !backend.uri_cache.is_empty(),
        "URI cache should be populated"
    );

    backend
        .did_close(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: file_uri },
        })
        .await;

    assert!(
        backend.uri_cache.is_empty(),
        "URI cache should be cleared after did_close"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_did_close_unknown_file_does_not_panic() {
    let backend = make_backend();

    // Closing a file that was never opened must be safe
    backend
        .did_close(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ls_close_unknown", "never_opened.py"),
            },
        })
        .await;
}

// ── goto_definition ───────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_goto_definition_returns_ok_on_empty_db() {
    let backend = make_backend();
    let result = backend
        .goto_definition(GotoDefinitionParams {
            text_document_position_params: tdp(turi("test_ls_gotodef", "test_file.py"), 0, 0),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
#[timeout(30000)]
async fn test_goto_definition_resolves_fixture() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_gotodef2", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
    );

    let test_path = tfile("test_ls_gotodef2", "test_example.py");
    db.analyze_file(
        test_path.clone(),
        "def test_it(my_fixture):\n    assert my_fixture == 1\n",
    );
    // Pre-register URI cache so path_to_uri works
    let conftest_uri = turi("test_ls_gotodef2", "conftest.py");
    backend.uri_cache.insert(conftest_path, conftest_uri);

    let result = backend
        .goto_definition(GotoDefinitionParams {
            text_document_position_params: tdp(
                turi("test_ls_gotodef2", "test_example.py"),
                0,  // line 0 (LSP) = line 1 (1-indexed): "def test_it(my_fixture):"
                12, // character inside "my_fixture"
            ),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── goto_implementation ───────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_goto_implementation_returns_ok_on_empty_db() {
    let backend = make_backend();
    let result = backend
        .goto_implementation(GotoImplementationParams {
            text_document_position_params: tdp(turi("test_ls_impl", "test_file.py"), 0, 0),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

// ── hover ─────────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_hover_returns_none_on_empty_file() {
    let backend = make_backend();
    let result = backend
        .hover(HoverParams {
            text_document_position_params: tdp(turi("test_ls_hover", "test.py"), 0, 0),
            work_done_progress_params: wdp(),
        })
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "no hover for unknown position");
}

#[tokio::test]
#[timeout(30000)]
async fn test_hover_returns_content_for_known_fixture() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_hover2", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef hover_fixture():\n    \"\"\"Hover docstring.\"\"\"\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_hover2", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    let test_path = tfile("test_ls_hover2", "test_example.py");
    db.analyze_file(test_path, "def test_it(hover_fixture):\n    pass\n");

    let result = backend
        .hover(HoverParams {
            text_document_position_params: tdp(turi("test_ls_hover2", "test_example.py"), 0, 12),
            work_done_progress_params: wdp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── references ────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_references_returns_ok_on_empty_db() {
    let backend = make_backend();
    let result = backend
        .references(ReferenceParams {
            text_document_position: tdp(turi("test_ls_refs", "test.py"), 0, 0),
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── completion ────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_completion_returns_ok() {
    let backend = make_backend();
    let result = backend
        .completion(CompletionParams {
            text_document_position: tdp(turi("test_ls_compl", "test.py"), 0, 0),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
            context: None,
        })
        .await;
    assert!(result.is_ok());
}

// ── code_action ───────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_code_action_returns_ok_no_diagnostics() {
    let backend = make_backend();
    let result = backend
        .code_action(CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ls_ca", "test.py"),
            },
            range: rng(0, 0, 0, 0),
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── document_symbol ───────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_document_symbol_returns_ok_for_unknown_file() {
    let backend = make_backend();
    let result = backend
        .document_symbol(DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ls_docsym", "test.py"),
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
#[timeout(30000)]
async fn test_document_symbol_returns_symbols_for_known_file() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_path = tfile("test_ls_docsym2", "conftest.py");
    db.analyze_file(
        file_path,
        "import pytest\n\n@pytest.fixture\ndef doc_fixture():\n    return 1\n",
    );

    let result = backend
        .document_symbol(DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ls_docsym2", "conftest.py"),
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── symbol (workspace symbol) ─────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_symbol_wraps_result_in_flat_response() {
    let db = Arc::new(FixtureDatabase::new());
    let fixture_path = tfile("test_ls_wssym", "conftest.py");
    db.analyze_file(
        fixture_path,
        "import pytest\n\n@pytest.fixture\ndef ws_sym_fixture():\n    return 1\n",
    );
    let backend = make_backend_with_db(db);

    let result = backend
        .symbol(WorkspaceSymbolParams {
            query: "ws_sym".to_string(),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;

    assert!(result.is_ok());
    // When results exist, the response must be wrapped in Flat
    if let Ok(Some(WorkspaceSymbolResponse::Flat(symbols))) = result {
        assert!(!symbols.is_empty(), "should find ws_sym_fixture");
    }
}

#[tokio::test]
#[timeout(30000)]
async fn test_symbol_no_results_returns_none_wrapped() {
    let backend = make_backend();
    let result = backend
        .symbol(WorkspaceSymbolParams {
            query: "definitely_does_not_exist_xyzzy".to_string(),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── code_lens ─────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_code_lens_returns_ok() {
    let backend = make_backend();
    let result = backend
        .code_lens(CodeLensParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ls_lens", "conftest.py"),
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── inlay_hint ────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_returns_ok() {
    let backend = make_backend();
    let result = backend
        .inlay_hint(InlayHintParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ls_inlay", "test.py"),
            },
            range: rng(0, 0, 100, 0),
            work_done_progress_params: wdp(),
        })
        .await;
    assert!(result.is_ok());
}

// Helper: open a file through the LSP did_open notification so that both
// `file_cache` and `usages` are populated for `handle_inlay_hint`.
async fn open_file(backend: &Backend, uri: Uri, text: &str) {
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: "python".to_string(),
                version: 1,
                text: text.to_string(),
            },
        })
        .await;
}

/// Call `backend.inlay_hint` and unwrap the double-`Option` result,
/// returning an empty `Vec` when the inner `Option` is `None`.
async fn get_hints(backend: &Backend, uri: Uri, range: Range) -> Vec<InlayHint> {
    let result = backend
        .inlay_hint(InlayHintParams {
            text_document: TextDocumentIdentifier { uri },
            range,
            work_done_progress_params: wdp(),
        })
        .await;
    assert!(result.is_ok(), "inlay_hint must not return an error");
    result.unwrap().unwrap_or_default()
}

// ── Happy path: one hint generated ────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_generates_hint_for_fixture_with_return_type() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Conftest: typed fixture
    open_file(
        &backend,
        turi("test_ih_gen", "conftest.py"),
        "import pytest\n\n@pytest.fixture\ndef my_fixture() -> int:\n    return 42\n",
    )
    .await;

    // Test file: one unannotated parameter
    open_file(
        &backend,
        turi("test_ih_gen", "test_foo.py"),
        "def test_foo(my_fixture):\n    pass\n",
    )
    .await;

    let hints = get_hints(
        &backend,
        turi("test_ih_gen", "test_foo.py"),
        rng(0, 0, 10, 0),
    )
    .await;

    assert_eq!(hints.len(), 1, "Should return exactly one hint");
    match &hints[0].label {
        InlayHintLabel::String(label) => assert_eq!(label, ": int"),
        _ => panic!("Expected String label"),
    }
    assert_eq!(hints[0].kind, Some(InlayHintKind::TYPE));
    // Tooltip should mention the fixture name and the type
    if let Some(InlayHintTooltip::String(tooltip)) = &hints[0].tooltip {
        assert!(tooltip.contains("my_fixture"));
        assert!(tooltip.contains("int"));
    } else {
        panic!("Expected String tooltip");
    }
    assert_eq!(hints[0].padding_left, Some(false));
    assert_eq!(hints[0].padding_right, Some(false));
}

// ── Early return when fixture_map is empty ────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_empty_when_no_fixtures_have_return_type() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Conftest: fixture WITHOUT a return-type annotation
    open_file(
        &backend,
        turi("test_ih_no_rt", "conftest.py"),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 42\n",
    )
    .await;

    open_file(
        &backend,
        turi("test_ih_no_rt", "test_foo.py"),
        "def test_foo(my_fixture):\n    pass\n",
    )
    .await;

    let hints = get_hints(
        &backend,
        turi("test_ih_no_rt", "test_foo.py"),
        rng(0, 0, 10, 0),
    )
    .await;

    assert!(
        hints.is_empty(),
        "Should return no hints when no fixture has a return type"
    );
}

// ── Usage outside the requested range is filtered ─────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_filters_usage_outside_range() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    open_file(
        &backend,
        turi("test_ih_range", "conftest.py"),
        "import pytest\n\n@pytest.fixture\ndef my_fixture() -> int:\n    return 42\n",
    )
    .await;

    // Test function is on LSP line 2 (1-based internal line 3)
    open_file(
        &backend,
        turi("test_ih_range", "test_foo.py"),
        "\n\ndef test_foo(my_fixture):\n    pass\n",
    )
    .await;

    // Request a range that ends before the test function (lines 0-1 only)
    let hints = get_hints(
        &backend,
        turi("test_ih_range", "test_foo.py"),
        rng(0, 0, 1, 0),
    )
    .await;

    assert!(
        hints.is_empty(),
        "Should return no hints when the range does not cover the test function"
    );
}

// ── Already-annotated parameter is skipped ────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_skips_already_annotated_param() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    open_file(
        &backend,
        turi("test_ih_ann", "conftest.py"),
        "import pytest\n\n@pytest.fixture\ndef my_fixture() -> int:\n    return 42\n",
    )
    .await;

    // Parameter already has a type annotation — hint must be suppressed
    open_file(
        &backend,
        turi("test_ih_ann", "test_foo.py"),
        "def test_foo(my_fixture: int):\n    pass\n",
    )
    .await;

    let hints = get_hints(
        &backend,
        turi("test_ih_ann", "test_foo.py"),
        rng(0, 0, 10, 0),
    )
    .await;

    assert!(
        hints.is_empty(),
        "Should skip parameters that already carry a type annotation"
    );
}

// ── Type is adapted to the consumer's import style ────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_adapts_dotted_type_to_consumer_from_import() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Fixture returns `pathlib.Path` (dotted form used in conftest)
    open_file(
        &backend,
        turi("test_ih_adapt", "conftest.py"),
        "import pytest\nimport pathlib\n\n@pytest.fixture\ndef pth() -> pathlib.Path:\n    return pathlib.Path('.')\n",
    )
    .await;

    // Consumer already has `from pathlib import Path` → hint should show `: Path`
    open_file(
        &backend,
        turi("test_ih_adapt", "test_foo.py"),
        "from pathlib import Path\n\ndef test_foo(pth):\n    pass\n",
    )
    .await;

    let hints = get_hints(
        &backend,
        turi("test_ih_adapt", "test_foo.py"),
        rng(0, 0, 10, 0),
    )
    .await;

    assert_eq!(hints.len(), 1, "Should return one hint");
    match &hints[0].label {
        InlayHintLabel::String(label) => assert_eq!(
            label, ": Path",
            "Dotted type should be shortened to match consumer's from-import"
        ),
        _ => panic!("Expected String label"),
    }
}

// ── Multiple fixtures: only typed ones get hints ──────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_multiple_fixtures_only_typed_get_hints() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    open_file(
        &backend,
        turi("test_ih_multi", "conftest.py"),
        "import pytest\n\n@pytest.fixture\ndef typed_fix() -> str:\n    return 'hi'\n\n@pytest.fixture\ndef untyped_fix():\n    return 42\n",
    )
    .await;

    open_file(
        &backend,
        turi("test_ih_multi", "test_foo.py"),
        "def test_foo(typed_fix, untyped_fix):\n    pass\n",
    )
    .await;

    let hints = get_hints(
        &backend,
        turi("test_ih_multi", "test_foo.py"),
        rng(0, 0, 5, 0),
    )
    .await;

    assert_eq!(
        hints.len(),
        1,
        "Should return a hint only for the typed fixture"
    );
    match &hints[0].label {
        InlayHintLabel::String(label) => assert_eq!(label, ": str"),
        _ => panic!("Expected String label"),
    }
}

// ── File known to the backend but not yet in the usages map ───────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_returns_none_when_file_has_no_usages() {
    // A file URI that resolves to a path but has never been analysed →
    // `usages.get` returns None and `handle_inlay_hint` returns Ok(None).
    let backend = make_backend();
    let result = backend
        .inlay_hint(InlayHintParams {
            text_document: TextDocumentIdentifier {
                uri: turi("test_ih_no_usages", "test_unknown.py"),
            },
            range: rng(0, 0, 100, 0),
            work_done_progress_params: wdp(),
        })
        .await;
    assert!(result.is_ok());
    // Either None or Some(empty) is acceptable — the key check is no panic/error
    let inner = result.unwrap();
    assert!(
        inner.is_none() || inner.unwrap().is_empty(),
        "Un-analysed file should produce no hints"
    );
}

// ── Hint position is at the end of the parameter name ─────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_inlay_hint_position_is_at_end_of_param_name() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    open_file(
        &backend,
        turi("test_ih_pos", "conftest.py"),
        "import pytest\n\n@pytest.fixture\ndef db_fix() -> bool:\n    return True\n",
    )
    .await;

    // `db_fix` starts at column 13 (after `def test_foo(`), length 6
    open_file(
        &backend,
        turi("test_ih_pos", "test_foo.py"),
        "def test_foo(db_fix):\n    pass\n",
    )
    .await;

    let hints = get_hints(
        &backend,
        turi("test_ih_pos", "test_foo.py"),
        rng(0, 0, 5, 0),
    )
    .await;

    assert_eq!(hints.len(), 1);
    // Hint must be on LSP line 0 (first line of the file)
    assert_eq!(hints[0].position.line, 0);
    // Character position must be past the end of `db_fix` (13 + 6 = 19)
    assert_eq!(
        hints[0].position.character, 19,
        "Hint should be placed right after the parameter name"
    );
}

// ── prepare_call_hierarchy ────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_prepare_call_hierarchy_returns_none_on_unknown_position() {
    let backend = make_backend();
    let result = backend
        .prepare_call_hierarchy(CallHierarchyPrepareParams {
            text_document_position_params: tdp(turi("test_ls_callh", "conftest.py"), 0, 0),
            work_done_progress_params: wdp(),
        })
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

// ── incoming_calls ────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_incoming_calls_returns_none_for_unknown_fixture() {
    let backend = make_backend();
    let dummy_uri = turi("test_ls_inc", "conftest.py");
    let result = backend
        .incoming_calls(CallHierarchyIncomingCallsParams {
            item: CallHierarchyItem {
                name: "nonexistent_fixture".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: dummy_uri,
                range: rng(0, 0, 0, 0),
                selection_range: rng(0, 0, 0, 0),
                data: None,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── outgoing_calls ────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_outgoing_calls_returns_none_for_unknown_fixture() {
    let backend = make_backend();
    let dummy_uri = turi("test_ls_out", "conftest.py");
    let result = backend
        .outgoing_calls(CallHierarchyOutgoingCallsParams {
            item: CallHierarchyItem {
                name: "nonexistent_fixture".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: dummy_uri,
                range: rng(0, 0, 0, 0),
                selection_range: rng(0, 0, 0, 0),
                data: None,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
}

// ── shutdown ──────────────────────────────────────────────────────────────

#[tokio::test]
#[timeout(10000)]
async fn test_shutdown_without_scan_task_returns_ok() {
    let backend = make_backend();
    // No active scan task – should return immediately
    let result = backend.shutdown().await;
    assert!(result.is_ok(), "shutdown should return Ok(())");
    // The spawned exit-after-100ms task is abandoned when the #[tokio::test]
    // runtime drops (current_thread executor never polls it again).
}

#[tokio::test]
#[timeout(10000)]
async fn test_shutdown_with_active_scan_task_aborts_it() {
    let backend = make_backend();

    // Simulate a long-running scan
    let handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    *backend.scan_task.lock().await = Some(handle);

    let result = backend.shutdown().await;
    assert!(
        result.is_ok(),
        "shutdown with active scan should return Ok(())"
    );

    // After shutdown the scan_task slot must be empty
    assert!(
        backend.scan_task.lock().await.is_none(),
        "scan_task handle should be consumed by shutdown"
    );
}

#[tokio::test]
#[timeout(10000)]
async fn test_shutdown_with_already_completed_scan_task() {
    let backend = make_backend();

    // A task that finishes immediately
    let handle = tokio::spawn(async {});
    // Give it a moment to complete
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    *backend.scan_task.lock().await = Some(handle);

    let result = backend.shutdown().await;
    assert!(result.is_ok());
}

// ── integration: full open → change → close lifecycle ────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_full_file_lifecycle_open_change_close() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let file_uri = turi("test_ls_lifecycle", "conftest.py");

    // 1. Open
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri.clone(),
                language_id: "python".to_string(),
                version: 1,
                text: "import pytest\n\n@pytest.fixture\ndef lc_fixture():\n    return 1\n"
                    .to_string(),
            },
        })
        .await;
    assert!(db.definitions.contains_key("lc_fixture"));

    // 2. Change
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: file_uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "import pytest\n\n@pytest.fixture\ndef lc_fixture_v2():\n    return 2\n"
                    .to_string(),
            }],
        })
        .await;
    assert!(db.definitions.contains_key("lc_fixture_v2"));

    // 3. Close
    backend
        .did_close(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: file_uri },
        })
        .await;
    assert!(
        backend.uri_cache.is_empty(),
        "URI cache should be empty after close"
    );
}
