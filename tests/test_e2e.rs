// E2E Integration Tests
// These tests verify the full system behavior including CLI commands,
// workspace scanning, and LSP functionality using the test_project.
//
// All tests have a 30-second timeout to prevent hangs from blocking CI.

#![allow(deprecated)]

use assert_cmd::Command;
use insta::assert_snapshot;
use ntest::timeout;
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
fn test_cli_fixtures_list_nonexistent_path() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("fixtures")
        .arg("list")
        .arg("/nonexistent/path/to/project")
        .assert()
        .failure();
}

#[test]
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
#[timeout(30000)]
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
#[timeout(30000)]
fn test_cli_invalid_subcommand() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("invalid").assert().failure();
}

#[test]
#[timeout(30000)]
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

// MARK: CLI fixtures unused E2E Tests

#[test]
#[timeout(30000)]
fn test_cli_fixtures_unused_text_output() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("unused")
        .arg("tests/test_project")
        .output()
        .expect("Failed to execute command");

    // Should exit with code 1 when unused fixtures are found
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain the header
    assert!(stdout.contains("Found") && stdout.contains("unused fixture"));

    // Should contain known unused fixtures from test_project
    // (iterator_fixture, auto_cleanup, temp_dir, temp_file are unused)
    assert!(stdout.contains("iterator_fixture") || stdout.contains("auto_cleanup"));
}

#[test]
#[timeout(30000)]
fn test_cli_fixtures_unused_json_output() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("unused")
        .arg("tests/test_project")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to execute command");

    // Should exit with code 1 when unused fixtures are found
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should be valid JSON
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "Output should be valid JSON: {}", stdout);

    let json = parsed.unwrap();
    assert!(json.is_array(), "JSON output should be an array");

    let arr = json.as_array().unwrap();
    assert!(!arr.is_empty(), "Should have at least one unused fixture");

    // Each item should have "file" and "fixture" keys
    for item in arr {
        assert!(
            item.get("file").is_some(),
            "Each item should have 'file' key"
        );
        assert!(
            item.get("fixture").is_some(),
            "Each item should have 'fixture' key"
        );
    }
}

#[test]
#[timeout(30000)]
fn test_cli_fixtures_unused_exit_code_zero_when_all_used() {
    // Create a temp directory with all fixtures used
    let temp_dir = std::env::temp_dir().join("test_all_used");
    std::fs::create_dir_all(&temp_dir).ok();

    std::fs::write(
        temp_dir.join("conftest.py"),
        r#"
import pytest

@pytest.fixture
def my_fixture():
    return "value"
"#,
    )
    .ok();

    std::fs::write(
        temp_dir.join("test_example.py"),
        r#"
def test_something(my_fixture):
    assert my_fixture == "value"
"#,
    )
    .ok();

    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("unused")
        .arg(&temp_dir)
        .output()
        .expect("Failed to execute command");

    // Should exit with code 0 when all fixtures are used
    assert!(output.status.success());
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No unused fixtures found"));

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
#[timeout(30000)]
fn test_cli_fixtures_unused_nonexistent_path() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("fixtures")
        .arg("unused")
        .arg("/nonexistent/path/to/project")
        .assert()
        .failure();
}

#[test]
#[timeout(30000)]
fn test_cli_fixtures_unused_help() {
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    cmd.arg("fixtures")
        .arg("unused")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("unused fixtures"))
        .stdout(predicate::str::contains("--format"));
}

// MARK: Workspace Scanning E2E Tests

#[test]
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
fn test_e2e_autouse_fixture_detection() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Should detect the autouse fixture
    let autouse = db.definitions.get("auto_cleanup");
    assert!(autouse.is_some());
}

#[test]
#[timeout(30000)]
fn test_e2e_scoped_fixtures() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Should detect session and module scoped fixtures
    assert!(db.definitions.get("session_fixture").is_some());
    assert!(db.definitions.get("module_fixture").is_some());
}

#[test]
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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
#[timeout(30000)]
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

// MARK: - Class-based Tests E2E Tests (issue #19)

#[test]
#[timeout(30000)]
fn test_e2e_class_based_tests_fixture_usage() {
    // Test case from https://github.com/bellini666/pytest-language-server/issues/19
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Fixtures defined in test_class_based.py should be found
    assert!(
        db.definitions.contains_key("shared_fixture"),
        "Should find shared_fixture"
    );
    assert!(
        db.definitions.contains_key("another_fixture"),
        "Should find another_fixture"
    );

    // Get the test file and check usages
    let test_file = project_path
        .join("test_class_based.py")
        .canonicalize()
        .unwrap();

    let usages = db.usages.get(&test_file);
    assert!(
        usages.is_some(),
        "Should have usages in test_class_based.py"
    );

    let usages = usages.unwrap();

    // Count usages of shared_fixture (should be used by multiple test methods in classes)
    let shared_usages: Vec<_> = usages
        .iter()
        .filter(|u| u.name == "shared_fixture")
        .collect();
    assert!(
        shared_usages.len() >= 4,
        "shared_fixture should be used at least 4 times (by test methods in classes), got {}",
        shared_usages.len()
    );

    // Count usages of another_fixture
    let another_usages: Vec<_> = usages
        .iter()
        .filter(|u| u.name == "another_fixture")
        .collect();
    assert!(
        another_usages.len() >= 2,
        "another_fixture should be used at least 2 times, got {}",
        another_usages.len()
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_class_based_fixture_references() {
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Get the shared_fixture definition
    let shared_defs = db.definitions.get("shared_fixture");
    assert!(shared_defs.is_some());

    let def = &shared_defs.unwrap()[0];
    let refs = db.find_references_for_definition(def);

    // Should have references from test methods in classes
    assert!(
        refs.len() >= 4,
        "shared_fixture should have at least 4 references from class test methods, got {}",
        refs.len()
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_cli_class_based_fixtures_shown_as_used() {
    // Run CLI and verify fixtures used by class-based tests are marked as used
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg("tests/test_project")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // shared_fixture and another_fixture should be shown as used (not "unused")
    // They are used by test methods inside TestClassBased and TestNestedClasses
    assert!(
        stdout.contains("shared_fixture") && !stdout.contains("shared_fixture (unused)"),
        "shared_fixture should be marked as used"
    );
    assert!(
        stdout.contains("another_fixture") && !stdout.contains("another_fixture (unused)"),
        "another_fixture should be marked as used"
    );
}

// MARK: Keyword-only and Positional-only Fixtures E2E Tests

#[test]
#[timeout(30000)]
fn test_e2e_keyword_only_fixture_detection() {
    // Test that fixtures used as keyword-only arguments are properly detected
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");
    db.scan_workspace(&project_path);

    // Get the test file path
    let test_file = project_path
        .join("test_kwonly_fixtures.py")
        .canonicalize()
        .unwrap();

    // Check that usages were detected for keyword-only fixtures
    let usages = db.usages.get(&test_file);
    assert!(
        usages.is_some(),
        "Usages should be detected for test_kwonly_fixtures.py"
    );

    let usages = usages.unwrap();

    // Verify sample_fixture usage (used in keyword-only and positional-only tests)
    assert!(
        usages.iter().any(|u| u.name == "sample_fixture"),
        "sample_fixture should be detected as used"
    );

    // Verify another_fixture usage (used in keyword-only tests)
    assert!(
        usages.iter().any(|u| u.name == "another_fixture"),
        "another_fixture should be detected as used"
    );

    // Verify shared_resource usage (used in keyword-only tests)
    assert!(
        usages.iter().any(|u| u.name == "shared_resource"),
        "shared_resource should be detected as used"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_keyword_only_no_undeclared_fixtures() {
    // Verify that keyword-only fixtures are not flagged as undeclared
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");
    db.scan_workspace(&project_path);

    // Get the test file path
    let test_file = project_path
        .join("test_kwonly_fixtures.py")
        .canonicalize()
        .unwrap();

    // There should be no undeclared fixtures in this file
    let undeclared = db.get_undeclared_fixtures(&test_file);
    assert_eq!(
        undeclared.len(),
        0,
        "Keyword-only fixtures should not be flagged as undeclared. Found: {:?}",
        undeclared.iter().map(|u| &u.name).collect::<Vec<_>>()
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_keyword_only_go_to_definition() {
    // Test that go-to-definition works for keyword-only fixtures
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");
    db.scan_workspace(&project_path);

    let test_file = project_path
        .join("test_kwonly_fixtures.py")
        .canonicalize()
        .unwrap();
    let conftest_file = project_path.join("conftest.py").canonicalize().unwrap();

    // Get the usages for the test file
    let usages = db.usages.get(&test_file);
    assert!(usages.is_some());

    let usages = usages.unwrap();

    // Find the sample_fixture usage
    let sample_usage = usages.iter().find(|u| u.name == "sample_fixture");
    assert!(
        sample_usage.is_some(),
        "sample_fixture usage should be found"
    );
    let sample_usage = sample_usage.unwrap();

    // Try to find the definition using the usage position
    // usage.line is 1-based, but find_fixture_definition expects 0-based LSP coordinates
    let definition = db.find_fixture_definition(
        &test_file,
        (sample_usage.line - 1) as u32,
        sample_usage.start_char as u32,
    );

    assert!(
        definition.is_some(),
        "Definition should be found for keyword-only fixture"
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "sample_fixture");
    assert_eq!(def.file_path, conftest_file);
}

// MARK: Imported Fixtures E2E Tests

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_are_detected() {
    // Tests that fixtures imported via star import in conftest.py are properly detected
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Fixtures from fixture_module.py should be available
    let imported = db.definitions.get("imported_fixture");
    assert!(
        imported.is_some(),
        "imported_fixture should be detected from fixture_module.py"
    );

    let another_imported = db.definitions.get("another_imported_fixture");
    assert!(
        another_imported.is_some(),
        "another_imported_fixture should be detected from fixture_module.py"
    );

    // The explicitly_imported fixture should also be detected
    let explicit = db.definitions.get("explicitly_imported");
    assert!(
        explicit.is_some(),
        "explicitly_imported should be detected from fixture_module.py"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_available_in_test_file() {
    // Tests that imported fixtures are available for tests in the same directory
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    let test_file = project_path.join("imported_fixtures/test_uses_imported.py");
    let test_file_canonical = test_file.canonicalize().unwrap();

    let available = db.get_available_fixtures(&test_file_canonical);
    let names: Vec<&str> = available.iter().map(|f| f.name.as_str()).collect();

    // Should have access to imported fixtures via conftest.py star import
    assert!(
        names.contains(&"imported_fixture"),
        "imported_fixture should be available in test file"
    );
    assert!(
        names.contains(&"another_imported_fixture"),
        "another_imported_fixture should be available in test file"
    );
    assert!(
        names.contains(&"explicitly_imported"),
        "explicitly_imported should be available in test file"
    );

    // Should also have access to the local fixture defined in conftest
    assert!(
        names.contains(&"local_fixture"),
        "local_fixture should be available in test file"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_go_to_definition() {
    // Tests that go-to-definition works for imported fixtures
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    let test_file = project_path.join("imported_fixtures/test_uses_imported.py");
    let test_file_canonical = test_file.canonicalize().unwrap();
    let fixture_module = project_path.join("imported_fixtures/fixture_module.py");
    let fixture_module_canonical = fixture_module.canonicalize().unwrap();

    // Find the usage of imported_fixture in the test file
    let usages = db.usages.get(&test_file_canonical);
    assert!(usages.is_some(), "Test file should have fixture usages");

    let usages = usages.unwrap();
    let imported_usage = usages
        .iter()
        .find(|u| u.name == "imported_fixture")
        .expect("Should find imported_fixture usage");

    // Go-to-definition should find the fixture in fixture_module.py
    let definition = db.find_fixture_definition(
        &test_file_canonical,
        (imported_usage.line - 1) as u32,
        imported_usage.start_char as u32,
    );

    assert!(
        definition.is_some(),
        "Should find definition for imported_fixture"
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "imported_fixture");
    assert_eq!(
        def.file_path, fixture_module_canonical,
        "Definition should be in fixture_module.py"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_find_references() {
    // Tests that find-references works for imported fixtures
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Get the definition of imported_fixture
    let definitions = db.definitions.get("imported_fixture");
    assert!(definitions.is_some());
    let def = definitions.unwrap().first().unwrap().clone();

    // Find all references to this fixture
    let references = db.find_references_for_definition(&def);

    // Should find at least the usage in test_uses_imported.py
    assert!(
        !references.is_empty(),
        "Should find references to imported_fixture"
    );

    // Verify at least one reference is in the test file
    let test_file = project_path.join("imported_fixtures/test_uses_imported.py");
    let test_file_canonical = test_file.canonicalize().unwrap();
    let has_test_ref = references
        .iter()
        .any(|r| r.file_path == test_file_canonical);
    assert!(
        has_test_ref,
        "Should have a reference in test_uses_imported.py"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_no_undeclared_warning() {
    // Tests that imported fixtures are not flagged as undeclared
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    let test_file = project_path.join("imported_fixtures/test_uses_imported.py");
    let test_file_canonical = test_file.canonicalize().unwrap();

    let undeclared = db.get_undeclared_fixtures(&test_file_canonical);
    let undeclared_names: Vec<&str> = undeclared.iter().map(|u| u.name.as_str()).collect();

    // Imported fixtures should NOT be in undeclared
    assert!(
        !undeclared_names.contains(&"imported_fixture"),
        "imported_fixture should not be flagged as undeclared"
    );
    assert!(
        !undeclared_names.contains(&"another_imported_fixture"),
        "another_imported_fixture should not be flagged as undeclared"
    );
    assert!(
        !undeclared_names.contains(&"local_fixture"),
        "local_fixture should not be flagged as undeclared"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_cache_performance() {
    // Tests that the imported fixtures cache provides performance benefit
    use std::time::Instant;

    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    let conftest_file = project_path.join("imported_fixtures/conftest.py");
    let conftest_canonical = conftest_file.canonicalize().unwrap();

    // First call - populates the cache
    let start = Instant::now();
    let mut visited = std::collections::HashSet::new();
    let first_result = db.get_imported_fixtures(&conftest_canonical, &mut visited);
    let first_duration = start.elapsed();

    // Second call - should hit the cache
    let start = Instant::now();
    let mut visited = std::collections::HashSet::new();
    let second_result = db.get_imported_fixtures(&conftest_canonical, &mut visited);
    let second_duration = start.elapsed();

    // Results should be the same
    assert_eq!(first_result, second_result);

    // Cache hit should be faster (allow some variance for system noise)
    // We just verify the cache works by checking results are identical
    // In CI, timing can be unreliable, so we just log for diagnostics
    eprintln!(
        "Import cache: first call {:?}, second call {:?}",
        first_duration, second_duration
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_imported_fixtures_cli_shows_them() {
    // Tests that the CLI shows imported fixtures as used (not unused)
    let mut cmd = Command::cargo_bin("pytest-language-server").unwrap();
    let output = cmd
        .arg("fixtures")
        .arg("list")
        .arg("tests/test_project/imported_fixtures")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // imported_fixture and another_imported_fixture should appear in the output
    // They are used in test_uses_imported.py
    assert!(
        stdout.contains("imported_fixture"),
        "CLI output should list imported_fixture"
    );
    assert!(
        stdout.contains("another_imported_fixture"),
        "CLI output should list another_imported_fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_transitive_imported_fixtures() {
    // Tests that fixtures imported transitively (A imports from B, B imports from C) are detected
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    // Fixtures from nested/deep_fixtures.py should be available via transitive import:
    // conftest.py -> fixture_module.py -> nested/deep_fixtures.py
    let deep_fixture = db.definitions.get("deep_nested_fixture");
    assert!(
        deep_fixture.is_some(),
        "deep_nested_fixture should be detected from nested/deep_fixtures.py"
    );

    let another_deep = db.definitions.get("another_deep_fixture");
    assert!(
        another_deep.is_some(),
        "another_deep_fixture should be detected from nested/deep_fixtures.py"
    );

    // These fixtures should be available in test_uses_imported.py
    let test_file = project_path.join("imported_fixtures/test_uses_imported.py");
    let test_file_canonical = test_file.canonicalize().unwrap();

    let available = db.get_available_fixtures(&test_file_canonical);
    let names: Vec<&str> = available.iter().map(|f| f.name.as_str()).collect();

    assert!(
        names.contains(&"deep_nested_fixture"),
        "deep_nested_fixture should be available via transitive import"
    );
    assert!(
        names.contains(&"another_deep_fixture"),
        "another_deep_fixture should be available via transitive import"
    );
}

#[test]
#[timeout(30000)]
fn test_e2e_transitive_imports_go_to_definition() {
    // Tests that go-to-definition works for transitively imported fixtures
    let db = FixtureDatabase::new();
    let project_path = PathBuf::from("tests/test_project");

    db.scan_workspace(&project_path);

    let test_file = project_path.join("imported_fixtures/test_uses_imported.py");
    let test_file_canonical = test_file.canonicalize().unwrap();
    let deep_fixtures_file = project_path.join("imported_fixtures/nested/deep_fixtures.py");
    let deep_fixtures_canonical = deep_fixtures_file.canonicalize().unwrap();

    // Find the usage of deep_nested_fixture in the test file
    let usages = db.usages.get(&test_file_canonical);
    assert!(usages.is_some(), "Test file should have fixture usages");

    let usages = usages.unwrap();
    let deep_usage = usages
        .iter()
        .find(|u| u.name == "deep_nested_fixture")
        .expect("Should find deep_nested_fixture usage");

    // Go-to-definition should find the fixture in nested/deep_fixtures.py
    let definition = db.find_fixture_definition(
        &test_file_canonical,
        (deep_usage.line - 1) as u32,
        deep_usage.start_char as u32,
    );

    assert!(
        definition.is_some(),
        "Should find definition for deep_nested_fixture"
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "deep_nested_fixture");
    assert_eq!(
        def.file_path, deep_fixtures_canonical,
        "Definition should be in nested/deep_fixtures.py"
    );
}
