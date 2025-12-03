// E2E Integration Tests
// These tests verify the full system behavior including CLI commands,
// workspace scanning, and LSP functionality using the test_project.

#![allow(deprecated)]

use assert_cmd::Command;
use insta::assert_snapshot;
use predicates::prelude::*;
use pytest_language_server::FixtureDatabase;
use std::path::PathBuf;

// Helper function to normalize paths in output for cross-platform testing
fn normalize_path_in_output(output: &str) -> String {
    // Get the absolute path to tests/test_project
    let test_project_path = std::env::current_dir()
        .unwrap()
        .join("tests/test_project")
        .canonicalize()
        .unwrap();

    // Replace the absolute path with a placeholder
    output.replace(
        &test_project_path.to_string_lossy().to_string(),
        "<TEST_PROJECT_PATH>",
    )
}

// MARK: CLI E2E Tests

#[test]
fn test_cli_fixtures_list_full_output() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg("tests/test_project")
        .output()
        .expect("Failed to execute command");

    // Should succeed
    assert!(output.status.success());

    // Convert output to string and normalize for snapshot testing
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Normalize path for cross-platform snapshot testing
    let normalized = normalize_path_in_output(&stdout);

    // Snapshot the output (colors will be in the output)
    assert_snapshot!("cli_fixtures_list_full", normalized);
}

#[test]
fn test_cli_fixtures_list_skip_unused() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg("tests/test_project")
        .arg("--skip-unused")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = normalize_path_in_output(&stdout);
    assert_snapshot!("cli_fixtures_list_skip_unused", normalized);
}

#[test]
fn test_cli_fixtures_list_only_unused() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg("tests/test_project")
        .arg("--only-unused")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized = normalize_path_in_output(&stdout);
    assert_snapshot!("cli_fixtures_list_only_unused", normalized);
}

#[test]
fn test_cli_fixtures_list_nonexistent_path() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("fixtures")
        .arg("list")
        .arg("/nonexistent/path/to/project")
        .assert()
        .failure();
}

#[test]
fn test_cli_fixtures_list_empty_directory() {
    let temp_dir = std::env::temp_dir().join("empty_test_dir");
    std::fs::create_dir_all(&temp_dir).ok();

    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg(&temp_dir)
        .output()
        .expect("Failed to execute command");

    // Should succeed but show no fixtures
    assert!(output.status.success());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_cli_help_message() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Language Server Protocol"))
        .stdout(predicate::str::contains("fixtures"))
        .stdout(predicate::str::contains("Fixture-related"));
}

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_cli_fixtures_help() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("fixtures")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("List all fixtures"));
}

#[test]
fn test_cli_invalid_subcommand() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("invalid").assert().failure();
}

#[test]
fn test_cli_conflicting_flags() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("fixtures")
        .arg("list")
        .arg("tests/test_project")
        .arg("--skip-unused")
        .arg("--only-unused")
        .assert()
        .failure();
}

// MARK: Workspace Scanning E2E Tests

#[test]
fn test_e2e_scan_expanded_test_project() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Verify fixtures from root conftest
    assert!(db.definitions.get("sample_fixture").is_some());

    // Verify fixtures from api/conftest.py
    assert!(db.definitions.get("api_client").is_some());
    assert!(db.definitions.get("api_token").is_some());
    assert!(db.definitions.get("mock_response").is_some());

    // Verify fixtures from database/conftest.py
    assert!(db.definitions.get("db_connection").is_some());
    assert!(db.definitions.get("db_cursor").is_some());
    assert!(db.definitions.get("transaction").is_some());

    // Verify fixtures from utils/conftest.py
    assert!(db.definitions.get("temp_file").is_some());
    assert!(db.definitions.get("temp_dir").is_some());
    assert!(db.definitions.get("auto_cleanup").is_some());

    // Verify fixtures from integration/test_scopes.py
    assert!(db.definitions.get("session_fixture").is_some());
    assert!(db.definitions.get("module_fixture").is_some());

    // Verify fixture from api/test_endpoints.py
    assert!(db.definitions.get("local_fixture").is_some());
}

#[test]
fn test_e2e_fixture_hierarchy_resolution() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Test file in api/ should see fixtures from api/conftest.py and root conftest.py
    let test_file = project_path.join("api/test_endpoints.py");
    let test_file_canonical = test_file.canonicalize().unwrap();
    let available = db.get_available_fixtures(&test_file_canonical);

    let names: Vec<&str> = available.iter().map(|f| f.name.as_str()).collect();

    // Should have access to api fixtures
    assert!(names.contains(&"api_client"));
    assert!(names.contains(&"api_token"));

    // Should have access to root fixtures
    assert!(names.contains(&"sample_fixture"));

    // Should NOT have access to database fixtures (different branch)
    assert!(!names.contains(&"db_connection"));
}

#[test]
fn test_e2e_fixture_dependency_chain() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Verify 3-level dependency chain: transaction -> db_cursor -> db_connection
    let transaction = db.definitions.get("transaction").unwrap();
    assert_eq!(transaction.len(), 1);

    let db_cursor = db.definitions.get("db_cursor").unwrap();
    assert_eq!(db_cursor.len(), 1);

    let db_connection = db.definitions.get("db_connection").unwrap();
    assert_eq!(db_connection.len(), 1);
}

#[test]
fn test_e2e_autouse_fixture_detection() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Should detect the autouse fixture
    let autouse = db.definitions.get("auto_cleanup");
    assert!(autouse.is_some());
}

#[test]
fn test_e2e_scoped_fixtures() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Should detect session and module scoped fixtures
    assert!(db.definitions.get("session_fixture").is_some());
    assert!(db.definitions.get("module_fixture").is_some());
}

#[test]
fn test_e2e_fixture_usage_in_test_file() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Check usages in api/test_endpoints.py (path will be canonicalized)
    let test_file = project_path.join("api/test_endpoints.py");
    let test_file_canonical = test_file.canonicalize().unwrap();
    let usages = db.usages.get(&test_file_canonical);

    assert!(
        usages.is_some(),
        "No usages found for {:?}",
        test_file_canonical
    );
    let usages = usages.unwrap();

    // Should have multiple fixture usages
    assert!(
        usages.len() >= 3,
        "Expected at least 3 usages, found {}",
        usages.len()
    ); // api_client, api_token, mock_response, local_fixture

    let usage_names: Vec<&str> = usages.iter().map(|u| u.name.as_str()).collect();
    assert!(usage_names.contains(&"api_client"));
    assert!(usage_names.contains(&"api_token"));
}

#[test]
fn test_e2e_find_references_across_project() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Find all references to api_client
    let references = db.find_fixture_references("api_client");

    // Should find usages in test files
    assert!(!references.is_empty());
}

#[test]
fn test_e2e_fixture_override_in_subdirectory() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Check if override fixture exists (from existing test_project structure)
    let test_file = project_path.join("subdir/test_override.py");

    if test_file.exists() {
        let test_file_canonical = test_file.canonicalize().unwrap();
        let available = db.get_available_fixtures(&test_file_canonical);

        // Should have fixtures from both root and subdir conftest
        let names: Vec<&str> = available.iter().map(|f| f.name.as_str()).collect();
        assert!(!names.is_empty());
    }
}

// MARK: Performance E2E Tests

#[test]
fn test_e2e_scan_performance() {
    use std::time::Instant;

    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    let start = Instant::now();
    db.scan_workspace(&project_path);
    let duration = start.elapsed();

    // Scanning should be fast (less than 1 second for small project)
    assert!(
        duration.as_secs() < 1,
        "Scanning took too long: {:?}",
        duration
    );
}

#[test]
fn test_e2e_repeated_analysis() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    // Scan twice - second scan should be fast due to caching
    db.scan_workspace(&project_path);

    use std::time::Instant;
    let start = Instant::now();
    db.scan_workspace(&project_path);
    let duration = start.elapsed();

    assert!(duration.as_millis() < 500, "Re-scanning took too long");
}

// MARK: Error Handling E2E Tests

#[test]
fn test_e2e_malformed_python_file() {
    let db = FixtureDatabase::new();

    // Create a temp file with invalid Python
    let temp_dir = std::env::temp_dir().join("test_malformed");
    std::fs::create_dir_all(&temp_dir).ok();

    let bad_file = temp_dir.join("test_bad.py");
    std::fs::write(
        &bad_file,
        "def test_something(\n    this is not valid python",
    )
    .ok();

    // Should not crash
    db.scan_workspace(&temp_dir);

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_e2e_mixed_valid_and_invalid_files() {
    let db = FixtureDatabase::new();

    let temp_dir = std::env::temp_dir().join("test_mixed");
    std::fs::create_dir_all(&temp_dir).ok();

    // Valid file
    std::fs::write(
        temp_dir.join("test_valid.py"),
        r#"
import pytest

@pytest.fixture
def valid_fixture():
    return "valid"

def test_something(valid_fixture):
    pass
"#,
    )
    .ok();

    // Invalid file
    std::fs::write(
        temp_dir.join("test_invalid.py"),
        "def test_broken(\n    invalid syntax here",
    )
    .ok();

    db.scan_workspace(&temp_dir);

    // Should still find the valid fixture
    assert!(db.definitions.get("valid_fixture").is_some());

    std::fs::remove_dir_all(&temp_dir).ok();
}

// MARK: - Renamed Fixtures E2E Tests

#[test]
fn test_e2e_renamed_fixtures_in_test_project() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // The test_renamed_fixtures.py file has fixtures with name= parameter
    // Fixtures should be registered by their alias, not function name
    assert!(
        db.definitions.contains_key("renamed_db"),
        "Should find fixture by alias 'renamed_db'"
    );
    assert!(
        db.definitions.contains_key("user"),
        "Should find fixture by alias 'user'"
    );
    assert!(
        db.definitions.contains_key("normal_fixture"),
        "Should find normal fixture by function name"
    );

    // Internal function names should NOT be registered
    assert!(
        !db.definitions.contains_key("internal_database_fixture"),
        "Internal function name should not be registered"
    );
    assert!(
        !db.definitions.contains_key("create_user_fixture"),
        "Internal function name should not be registered"
    );
}

#[test]
fn test_e2e_renamed_fixture_references() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Get the renamed_db fixture definition
    let renamed_db_defs = db.definitions.get("renamed_db");
    assert!(renamed_db_defs.is_some());

    let def = &renamed_db_defs.unwrap()[0];
    let refs = db.find_references_for_definition(def);

    // Should have references from:
    // 1. create_user_fixture (depends on renamed_db)
    // 2. test_with_renamed_fixture
    // 3. test_mixed_fixtures
    assert!(
        refs.len() >= 3,
        "Should have at least 3 references to renamed_db, got {}",
        refs.len()
    );

    // All references should use the alias name
    assert!(
        refs.iter().all(|r| r.name == "renamed_db"),
        "All references should use alias name"
    );
}

#[test]
fn test_e2e_renamed_fixture_goto_definition() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    let test_file = project_path
        .join("test_renamed_fixtures.py")
        .canonicalize()
        .unwrap();

    // Find "renamed_db" in test_with_renamed_fixture (line 24, 0-indexed: 23)
    // def test_with_renamed_fixture(renamed_db):
    let fixture_name = db.find_fixture_at_position(&test_file, 23, 30);
    assert_eq!(
        fixture_name,
        Some("renamed_db".to_string()),
        "Should find fixture name at position"
    );

    let definition = db.find_fixture_definition(&test_file, 23, 30);
    assert!(definition.is_some(), "Should find fixture definition");

    let def = definition.unwrap();
    assert_eq!(def.name, "renamed_db", "Definition should have alias name");
}

#[test]
fn test_e2e_cli_fixtures_list_with_renamed() {
    // Run CLI and verify renamed fixtures appear correctly
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg("tests/test_project")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show renamed fixtures by their alias
    assert!(
        stdout.contains("renamed_db"),
        "Output should contain 'renamed_db'"
    );
    assert!(stdout.contains("user"), "Output should contain 'user'");
    assert!(
        stdout.contains("normal_fixture"),
        "Output should contain 'normal_fixture'"
    );

    // Should NOT show internal function names
    assert!(
        !stdout.contains("internal_database_fixture"),
        "Should not show internal function name"
    );
    assert!(
        !stdout.contains("create_user_fixture"),
        "Should not show internal function name"
    );
}
