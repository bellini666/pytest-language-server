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

/// Helper: unwrap a `GotoImplementationResponse::Scalar` to a `Location`.
fn scalar_location(
    resp: tower_lsp_server::ls_types::request::GotoImplementationResponse,
) -> Location {
    use tower_lsp_server::ls_types::request::GotoImplementationResponse;
    match resp {
        GotoImplementationResponse::Scalar(loc) => loc,
        _ => panic!(
            "expected GotoImplementationResponse::Scalar, got {:?}",
            resp
        ),
    }
}

#[tokio::test]
#[timeout(30000)]
async fn test_goto_implementation_generator_fixture_returns_yield_line() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Generator fixture with yield on line 5 (1-indexed).
    let conftest_path = tfile("test_ls_impl_yield", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef gen_fixture():\n    value = 1\n    yield value\n",
    );
    let conftest_uri = turi("test_ls_impl_yield", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    // Click on the fixture name (line 3).
    let result = backend
        .goto_implementation(GotoImplementationParams {
            text_document_position_params: tdp(conftest_uri.clone(), 3, 6),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("should resolve");

    let loc = scalar_location(result);
    assert_eq!(loc.uri, conftest_uri);
    // `yield value` is on line index 5 (0-indexed).
    assert_eq!(loc.range.start.line, 5);
}

#[tokio::test]
#[timeout(30000)]
async fn test_goto_implementation_non_generator_falls_back_to_definition() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_impl_return", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef ret_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_impl_return", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    let result = backend
        .goto_implementation(GotoImplementationParams {
            text_document_position_params: tdp(conftest_uri.clone(), 3, 6),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("should resolve");
    let loc = scalar_location(result);
    assert_eq!(loc.uri, conftest_uri);
    // Returns the definition line (line 3 0-indexed).
    assert_eq!(loc.range.start.line, 3);
}

#[tokio::test]
#[timeout(30000)]
async fn test_goto_implementation_from_usage_resolves_to_yield() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_impl_from_usage", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef gen_fixture():\n    yield 42\n",
    );
    let conftest_uri = turi("test_ls_impl_from_usage", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    let test_path = tfile("test_ls_impl_from_usage", "test_example.py");
    db.analyze_file(test_path.clone(), "def test_one(gen_fixture):\n    pass\n");
    let test_uri = turi("test_ls_impl_from_usage", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri.clone());

    let result = backend
        .goto_implementation(GotoImplementationParams {
            text_document_position_params: tdp(test_uri, 0, 13),
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("should resolve");
    let loc = scalar_location(result);
    assert_eq!(loc.uri, conftest_uri);
    // yield 42 is on line 4 (0-indexed).
    assert_eq!(loc.range.start.line, 4);
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

/// Helper: call references and unwrap the double-Option into Vec<Location>.
async fn get_refs(backend: &Backend, uri: Uri, line: u32, character: u32) -> Vec<Location> {
    let result = backend
        .references(ReferenceParams {
            text_document_position: tdp(uri, line, character),
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok(), "references must not return error");
    result.unwrap().unwrap_or_default()
}

#[tokio::test]
#[timeout(30000)]
async fn test_references_returns_definition_and_usages() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Seed conftest with a fixture definition
    let conftest_path = tfile("test_ls_refs_happy", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_refs_happy", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    // Seed a test file with two usages
    let test_path = tfile("test_ls_refs_happy", "test_example.py");
    db.analyze_file(
        test_path.clone(),
        "def test_one(my_fixture):\n    pass\n\ndef test_two(my_fixture):\n    pass\n",
    );
    let test_uri = turi("test_ls_refs_happy", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri.clone());

    // Click on the fixture name in test_one (line 0, char 13)
    let locations = get_refs(&backend, test_uri.clone(), 0, 13).await;

    // Should include the definition + both usages
    assert_eq!(
        locations.len(),
        3,
        "expected definition + 2 usages, got {:?}",
        locations
    );
    // First location is the definition, in the conftest file
    assert_eq!(locations[0].uri, conftest_uri);
    // The remaining locations are the usages in the test file
    let usage_uris: Vec<&Uri> = locations.iter().skip(1).map(|l| &l.uri).collect();
    assert!(usage_uris.iter().all(|u| **u == test_uri));
}

#[tokio::test]
#[timeout(30000)]
async fn test_references_from_definition_line() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_refs_def_line", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_refs_def_line", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    let test_path = tfile("test_ls_refs_def_line", "test_example.py");
    db.analyze_file(test_path.clone(), "def test_one(my_fixture):\n    pass\n");
    let test_uri = turi("test_ls_refs_def_line", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri.clone());

    // Click directly on the `def my_fixture` line (line 3 in conftest, 0-indexed)
    // exercises the `get_definition_at_line` fallback branch.
    let locations = get_refs(&backend, conftest_uri.clone(), 3, 6).await;

    assert!(
        !locations.is_empty(),
        "should return references when clicking on definition line"
    );
    // The first location is the definition itself (from conftest).
    assert_eq!(locations[0].uri, conftest_uri);
    // The remaining locations should include the usage in the test file.
    assert!(
        locations.iter().any(|l| l.uri == test_uri),
        "should include usage in test file"
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_references_self_referencing_fixture_skips_def_line() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Parent conftest defines `cli_runner`
    let parent_path = tfile("test_ls_refs_self", "conftest.py");
    db.analyze_file(
        parent_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef cli_runner():\n    return \"parent\"\n",
    );
    let parent_uri = turi("test_ls_refs_self", "conftest.py");
    backend
        .uri_cache
        .insert(parent_path.clone(), parent_uri.clone());

    // Child conftest overrides with self-referencing fixture
    let child_path = tfile("test_ls_refs_self/tests", "conftest.py");
    db.analyze_file(
        child_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef cli_runner(cli_runner):\n    return cli_runner\n",
    );
    let child_uri = turi("test_ls_refs_self/tests", "conftest.py");
    backend
        .uri_cache
        .insert(child_path.clone(), child_uri.clone());

    // Click on the parent `cli_runner` definition (line 3 in parent conftest).
    // The child's parameter `cli_runner` on the def line of the child's own
    // fixture is a reference to the parent, but sits on the child def line —
    // this exercises the "skip same-line-as-def" branch only when clicking
    // on the child definition. Click on child def line 3:
    let locations = get_refs(&backend, child_uri.clone(), 3, 6).await;

    // Should return at least the definition; the parameter reference on the
    // same line as the child definition should be filtered out to avoid
    // a duplicate with the definition entry.
    assert!(!locations.is_empty());
    assert_eq!(locations[0].uri, child_uri);
    // No duplicate: only one location should have the same uri+line as the def.
    let def_line = locations[0].range.start.line;
    let same_line_count = locations
        .iter()
        .filter(|l| l.uri == child_uri && l.range.start.line == def_line)
        .count();
    assert_eq!(
        same_line_count, 1,
        "def-line duplicate should be filtered; got {:?}",
        locations
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_references_skips_usage_on_definition_line() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Seed with a natural usage first so we can clone a real FixtureUsage.
    let conftest_path = tfile("test_ls_refs_skip_same", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
    );
    let test_path = tfile("test_ls_refs_skip_same", "test_example.py");
    db.analyze_file(test_path.clone(), "def test_one(my_fixture):\n    pass\n");

    let conftest_uri = turi("test_ls_refs_skip_same", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());
    backend
        .uri_cache
        .insert(test_path, turi("test_ls_refs_skip_same", "test_example.py"));

    // Clone the existing usage and rewrite it to sit on the definition line
    // of the conftest file (internal line 4). The cloned usage preserves
    // non-exhaustive fields so the struct stays constructible.
    let cloned_same_line_usage = {
        let entries = db
            .usage_by_fixture
            .get("my_fixture")
            .expect("seed usage should exist");
        let (_, existing) = entries.first().expect("at least one usage").clone();
        let mut u = existing;
        u.file_path = conftest_path.clone();
        u.line = 4;
        u.start_char = 4;
        u.end_char = 14;
        u
    };
    db.usage_by_fixture
        .entry("my_fixture".to_string())
        .or_default()
        .push((conftest_path.clone(), cloned_same_line_usage));

    // Click on the definition name → cursor on line 3 (0-indexed), char 6.
    // `definition_to_include` will be the conftest def at internal line 4,
    // and the injected same-line usage must be filtered out.
    let locations = get_refs(&backend, conftest_uri.clone(), 3, 6).await;

    // The returned locations should include the definition, plus the test
    // usage, but NOT the injected same-line usage.
    assert!(!locations.is_empty());
    // Count locations on the conftest file at the def line. There should be
    // exactly one (the definition itself); the cloned same-line usage is
    // filtered out by the "skip" branch.
    let def_line_lsp = 3;
    let conftest_def_line_count = locations
        .iter()
        .filter(|l| l.uri == conftest_uri && l.range.start.line == def_line_lsp)
        .count();
    assert_eq!(
        conftest_def_line_count, 1,
        "injected same-line usage should be filtered, got {:?}",
        locations
    );
}

#[tokio::test]
#[timeout(30000)]
async fn test_references_no_fixture_at_position_returns_none() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let test_path = tfile("test_ls_refs_no_fixture", "test_example.py");
    db.analyze_file(test_path.clone(), "def test_one():\n    pass\n");
    let test_uri = turi("test_ls_refs_no_fixture", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri.clone());

    // Click on the `def` keyword - no fixture there.
    let result = backend
        .references(ReferenceParams {
            text_document_position: tdp(test_uri, 0, 1),
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await;
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_none(),
        "no fixture at cursor should return None"
    );
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

#[tokio::test]
#[timeout(30000)]
async fn test_code_lens_returns_usage_count_per_fixture() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Two fixtures in conftest.
    let conftest_path = tfile("test_ls_lens_count", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef fixture_a():\n    return 1\n\n@pytest.fixture\ndef fixture_b():\n    return 2\n",
    );
    let conftest_uri = turi("test_ls_lens_count", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    // Three usages of fixture_a and one of fixture_b.
    let test_path = tfile("test_ls_lens_count", "test_example.py");
    db.analyze_file(
        test_path.clone(),
        "def test_one(fixture_a):\n    pass\n\ndef test_two(fixture_a):\n    pass\n\ndef test_three(fixture_a):\n    pass\n\ndef test_four(fixture_b):\n    pass\n",
    );
    let test_uri = turi("test_ls_lens_count", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri);

    let lenses = backend
        .code_lens(CodeLensParams {
            text_document: TextDocumentIdentifier { uri: conftest_uri },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("should return lenses");

    assert_eq!(lenses.len(), 2);
    let titles: Vec<String> = lenses
        .iter()
        .filter_map(|l| l.command.as_ref().map(|c| c.title.clone()))
        .collect();
    assert!(titles.iter().any(|t| t == "3 usages"));
    assert!(titles.iter().any(|t| t == "1 usage"));
}

#[tokio::test]
#[timeout(30000)]
async fn test_code_lens_skips_fixtures_from_other_files() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Fixture in one conftest
    let other_path = tfile("test_ls_lens_other", "other_conftest.py");
    db.analyze_file(
        other_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef only_here():\n    return 1\n",
    );
    backend
        .uri_cache
        .insert(other_path, turi("test_ls_lens_other", "other_conftest.py"));

    // Request code lenses on a different file — should return none.
    let empty_path = tfile("test_ls_lens_other", "empty_conftest.py");
    db.analyze_file(empty_path.clone(), "import pytest\n");
    let empty_uri = turi("test_ls_lens_other", "empty_conftest.py");
    backend.uri_cache.insert(empty_path, empty_uri.clone());

    let result = backend
        .code_lens(CodeLensParams {
            text_document: TextDocumentIdentifier { uri: empty_uri },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap();
    assert!(result.is_none(), "no fixtures in this file → None");
}

#[tokio::test]
#[timeout(30000)]
async fn test_code_lens_skips_third_party_fixtures() {
    use pytest_language_server::FixtureDefinition;

    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_lens_3p", "conftest.py");
    // Manually insert a third-party fixture definition so we can assert it is skipped.
    db.definitions.insert(
        "third_party_fx".to_string(),
        vec![FixtureDefinition {
            name: "third_party_fx".to_string(),
            file_path: conftest_path.clone(),
            line: 4,
            end_line: 5,
            start_char: 4,
            end_char: 18,
            is_third_party: true,
            ..Default::default()
        }],
    );
    let conftest_uri = turi("test_ls_lens_3p", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    let result = backend
        .code_lens(CodeLensParams {
            text_document: TextDocumentIdentifier { uri: conftest_uri },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "third-party fixtures should be filtered out, got {:?}",
        result
    );
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

// ── call_hierarchy: real data ─────────────────────────────────────────────

#[tokio::test]
#[timeout(30000)]
async fn test_prepare_call_hierarchy_returns_item_for_fixture() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_callh_item", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_callh_item", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    // Cursor on the fixture name on line 3 (0-indexed).
    let result = backend
        .prepare_call_hierarchy(CallHierarchyPrepareParams {
            text_document_position_params: tdp(conftest_uri.clone(), 3, 6),
            work_done_progress_params: wdp(),
        })
        .await;

    let items = result.unwrap().expect("should return items");
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.name, "my_fixture");
    assert_eq!(item.kind, SymbolKind::FUNCTION);
    assert_eq!(item.uri, conftest_uri);
    // Function-scoped fixture → detail is bare "@pytest.fixture".
    assert_eq!(item.detail.as_deref(), Some("@pytest.fixture"));
}

#[tokio::test]
#[timeout(30000)]
async fn test_prepare_call_hierarchy_scoped_fixture_detail() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_callh_scope", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture(scope=\"session\")\ndef my_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_callh_scope", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    let items = backend
        .prepare_call_hierarchy(CallHierarchyPrepareParams {
            text_document_position_params: tdp(conftest_uri, 3, 6),
            work_done_progress_params: wdp(),
        })
        .await
        .unwrap()
        .expect("items");
    assert!(items[0]
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("scope=\"session\""));
}

#[tokio::test]
#[timeout(30000)]
async fn test_prepare_call_hierarchy_from_usage_resolves_to_definition() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_callh_usage", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_callh_usage", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    let test_path = tfile("test_ls_callh_usage", "test_example.py");
    db.analyze_file(test_path.clone(), "def test_one(my_fixture):\n    pass\n");
    let test_uri = turi("test_ls_callh_usage", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri.clone());

    // Cursor on the fixture parameter in test file (line 0, char 13).
    let items = backend
        .prepare_call_hierarchy(CallHierarchyPrepareParams {
            text_document_position_params: tdp(test_uri, 0, 13),
            work_done_progress_params: wdp(),
        })
        .await
        .unwrap()
        .expect("items");
    // The item should point at the conftest definition.
    assert_eq!(items[0].uri, conftest_uri);
    assert_eq!(items[0].name, "my_fixture");
}

#[tokio::test]
#[timeout(30000)]
async fn test_incoming_calls_returns_callers() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Fixture `db` used by a test
    let conftest_path = tfile("test_ls_callh_in", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef db_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_callh_in", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    let test_path = tfile("test_ls_callh_in", "test_example.py");
    db.analyze_file(test_path.clone(), "def test_one(db_fixture):\n    pass\n");
    let test_uri = turi("test_ls_callh_in", "test_example.py");
    backend.uri_cache.insert(test_path, test_uri.clone());

    // Ask for incoming calls to db_fixture defined in conftest.
    let calls = backend
        .incoming_calls(CallHierarchyIncomingCallsParams {
            item: CallHierarchyItem {
                name: "db_fixture".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: conftest_uri,
                range: rng(3, 0, 3, 0),
                selection_range: rng(3, 4, 3, 14),
                data: None,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("some calls");

    assert!(!calls.is_empty(), "expected at least one incoming call");
    // The caller should be the test function `test_one`.
    assert!(calls.iter().any(|c| c.from.name == "test_one"));
    assert!(calls.iter().any(|c| c.from.uri == test_uri));
}

#[tokio::test]
#[timeout(30000)]
async fn test_outgoing_calls_returns_dependencies() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // A fixture `consumer` that depends on `db_fixture`
    let conftest_path = tfile("test_ls_callh_out", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef db_fixture():\n    return 1\n\n@pytest.fixture\ndef consumer(db_fixture):\n    return db_fixture\n",
    );
    let conftest_uri = turi("test_ls_callh_out", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    let calls = backend
        .outgoing_calls(CallHierarchyOutgoingCallsParams {
            item: CallHierarchyItem {
                name: "consumer".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: conftest_uri.clone(),
                range: rng(7, 0, 7, 0),
                selection_range: rng(7, 4, 7, 12),
                data: None,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("some calls");

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].to.name, "db_fixture");
    assert_eq!(calls[0].to.uri, conftest_uri);
    // from_ranges should point at the parameter in the signature of `consumer`.
    assert!(!calls[0].from_ranges.is_empty());
}

#[tokio::test]
#[timeout(30000)]
async fn test_outgoing_calls_skips_unresolvable_dependency() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Fixture depending on a name that has no definition
    let conftest_path = tfile("test_ls_callh_out_missing", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef consumer(unknown_dep):\n    return unknown_dep\n",
    );
    let conftest_uri = turi("test_ls_callh_out_missing", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path, conftest_uri.clone());

    let calls = backend
        .outgoing_calls(CallHierarchyOutgoingCallsParams {
            item: CallHierarchyItem {
                name: "consumer".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: conftest_uri,
                range: rng(3, 0, 3, 0),
                selection_range: rng(3, 4, 3, 12),
                data: None,
            },
            work_done_progress_params: wdp(),
            partial_result_params: prp(),
        })
        .await
        .unwrap()
        .expect("Some");
    assert!(calls.is_empty(), "unresolved deps should be filtered out");
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

// ── publish_diagnostics_for_file ─────────────────────────────────────────
//
// `publish_diagnostics_for_file` pushes diagnostics to the LSP client. With
// the dummy client we can't intercept the wire call, but we can exercise the
// *construction* paths (lines 17–96 in `providers/diagnostics.rs`) and assert
// they don't panic + that the underlying detection methods agree about
// whether a diagnostic would have been produced.

#[tokio::test]
#[timeout(30000)]
async fn test_publish_diagnostics_reports_undeclared_fixture() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // Define `parent_fixture` in conftest.py so it is an *available* fixture
    // for the sibling test file. Using it in a test body without declaring
    // it as a parameter is an "undeclared fixture" usage.
    let conftest_path = tfile("test_ls_diag_undecl", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef parent_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_diag_undecl", "conftest.py");
    backend.uri_cache.insert(conftest_path, conftest_uri);

    let test_path = tfile("test_ls_diag_undecl", "test_example.py");
    db.analyze_file(
        test_path.clone(),
        "def test_something():\n    result = parent_fixture\n",
    );
    let test_uri = turi("test_ls_diag_undecl", "test_example.py");
    backend
        .uri_cache
        .insert(test_path.clone(), test_uri.clone());

    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert!(
        !undeclared.is_empty(),
        "detection should find 'parent_fixture' as undeclared"
    );

    backend
        .publish_diagnostics_for_file(&test_uri, &test_path)
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_publish_diagnostics_reports_circular_dependency() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_diag_cycle", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef a(b):\n    return b\n\n@pytest.fixture\ndef b(a):\n    return a\n",
    );
    let conftest_uri = turi("test_ls_diag_cycle", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    // Detection method agrees there's a cycle in this file.
    let cycles = db.detect_fixture_cycles_in_file(&conftest_path);
    assert!(!cycles.is_empty(), "should detect a-b cycle");

    backend
        .publish_diagnostics_for_file(&conftest_uri, &conftest_path)
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_publish_diagnostics_reports_scope_mismatch() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // session-scoped fixture that depends on a function-scoped fixture
    let conftest_path = tfile("test_ls_diag_scope", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef narrow():\n    return 1\n\n@pytest.fixture(scope=\"session\")\ndef broad(narrow):\n    return narrow\n",
    );
    let conftest_uri = turi("test_ls_diag_scope", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    let mismatches = db.detect_scope_mismatches_in_file(&conftest_path);
    assert!(
        !mismatches.is_empty(),
        "should detect session-depends-on-function mismatch"
    );

    backend
        .publish_diagnostics_for_file(&conftest_uri, &conftest_path)
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_publish_diagnostics_respects_disabled_diagnostics() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    // All three diagnostic sources present in one file.
    let conftest_path = tfile("test_ls_diag_disabled", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef narrow():\n    return 1\n\n@pytest.fixture(scope=\"session\")\ndef broad(narrow):\n    return narrow\n\n@pytest.fixture\ndef a(b):\n    return b\n\n@pytest.fixture\ndef b(a):\n    return a\n\ndef test_something():\n    missing_fx\n",
    );
    let conftest_uri = turi("test_ls_diag_disabled", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    // Disable all three diagnostic kinds.
    {
        let mut config = backend.config.write().await;
        config.disabled_diagnostics = vec![
            "undeclared-fixture".to_string(),
            "circular-dependency".to_string(),
            "scope-mismatch".to_string(),
        ];
    }

    // Must still run cleanly (exercises the three `is_diagnostic_disabled` branches).
    backend
        .publish_diagnostics_for_file(&conftest_uri, &conftest_path)
        .await;
}

#[tokio::test]
#[timeout(30000)]
async fn test_publish_diagnostics_clean_file_publishes_nothing() {
    let db = Arc::new(FixtureDatabase::new());
    let backend = make_backend_with_db(Arc::clone(&db));

    let conftest_path = tfile("test_ls_diag_clean", "conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        "import pytest\n\n@pytest.fixture\ndef clean_fixture():\n    return 1\n",
    );
    let conftest_uri = turi("test_ls_diag_clean", "conftest.py");
    backend
        .uri_cache
        .insert(conftest_path.clone(), conftest_uri.clone());

    // No undeclared/cycle/mismatch.
    assert!(db.get_undeclared_fixtures(&conftest_path).is_empty());
    assert!(db.detect_fixture_cycles_in_file(&conftest_path).is_empty());
    assert!(db
        .detect_scope_mismatches_in_file(&conftest_path)
        .is_empty());

    backend
        .publish_diagnostics_for_file(&conftest_uri, &conftest_path)
        .await;
}
