//! Integration tests for configuration file support.

use pytest_language_server::{Config, FixtureDatabase};
use std::fs;
use tempfile::TempDir;

/// Helper to create a temporary project with pyproject.toml
fn create_temp_project(pyproject_content: &str) -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let pyproject_path = temp_dir.path().join("pyproject.toml");
    fs::write(&pyproject_path, pyproject_content).unwrap();
    temp_dir
}

/// Helper to create a Python test file
fn create_test_file(dir: &std::path::Path, filename: &str, content: &str) {
    fs::write(dir.join(filename), content).unwrap();
}

// ============ Config Loading Tests ============

#[test]
fn test_config_load_from_pyproject() {
    let temp_dir = create_temp_project(
        r#"
[project]
name = "myproject"

[tool.pytest-language-server]
exclude = ["build/**", "dist"]
disabled_diagnostics = ["undeclared-fixture"]
"#,
    );

    let config = Config::load(temp_dir.path());
    assert_eq!(config.exclude.len(), 2);
    assert_eq!(config.disabled_diagnostics, vec!["undeclared-fixture"]);
}

#[test]
fn test_config_load_missing_pyproject() {
    let temp_dir = TempDir::new().unwrap();
    // No pyproject.toml created

    let config = Config::load(temp_dir.path());
    // Should return defaults
    assert!(config.exclude.is_empty());
    assert!(config.disabled_diagnostics.is_empty());
}

#[test]
fn test_config_load_empty_tool_section() {
    let temp_dir = create_temp_project(
        r#"
[project]
name = "myproject"

[tool.pytest-language-server]
"#,
    );

    let config = Config::load(temp_dir.path());
    assert!(config.exclude.is_empty());
    assert!(config.disabled_diagnostics.is_empty());
}

#[test]
fn test_config_load_no_tool_section() {
    let temp_dir = create_temp_project(
        r#"
[project]
name = "myproject"
version = "1.0.0"
"#,
    );

    let config = Config::load(temp_dir.path());
    assert!(config.exclude.is_empty());
}

// ============ Exclude Pattern Tests ============

#[test]
fn test_exclude_patterns_in_workspace_scan() {
    let temp_dir = TempDir::new().unwrap();

    // Create pyproject.toml with exclude pattern
    fs::write(
        temp_dir.path().join("pyproject.toml"),
        r#"
[tool.pytest-language-server]
exclude = ["excluded/**"]
"#,
    )
    .unwrap();

    // Create test directories
    fs::create_dir_all(temp_dir.path().join("tests")).unwrap();
    fs::create_dir_all(temp_dir.path().join("excluded")).unwrap();

    // Create test files
    create_test_file(
        &temp_dir.path().join("tests"),
        "test_included.py",
        r#"
import pytest

@pytest.fixture
def included_fixture():
    return "included"

def test_included(included_fixture):
    pass
"#,
    );

    create_test_file(
        &temp_dir.path().join("excluded"),
        "test_excluded.py",
        r#"
import pytest

@pytest.fixture
def excluded_fixture():
    return "excluded"

def test_excluded(excluded_fixture):
    pass
"#,
    );

    // Load config and scan with excludes
    let config = Config::load(temp_dir.path());
    let db = FixtureDatabase::new();
    db.scan_workspace_with_excludes(temp_dir.path(), &config.exclude);

    // Verify included fixture is found
    assert!(
        db.definitions.contains_key("included_fixture"),
        "included_fixture should be found"
    );

    // Verify excluded fixture is NOT found
    assert!(
        !db.definitions.contains_key("excluded_fixture"),
        "excluded_fixture should be excluded"
    );
}

#[test]
fn test_exclude_patterns_glob_matching() {
    let temp_dir = TempDir::new().unwrap();

    fs::write(
        temp_dir.path().join("pyproject.toml"),
        r#"
[tool.pytest-language-server]
exclude = ["**/generated/**", "legacy_*"]
"#,
    )
    .unwrap();

    // Create directories
    fs::create_dir_all(temp_dir.path().join("src/generated")).unwrap();
    fs::create_dir_all(temp_dir.path().join("tests")).unwrap();

    // Create test files
    create_test_file(
        &temp_dir.path().join("src/generated"),
        "test_generated.py",
        r#"
import pytest

@pytest.fixture
def generated_fixture():
    pass
"#,
    );

    create_test_file(
        temp_dir.path(),
        "legacy_test_old.py",
        r#"
import pytest

@pytest.fixture
def legacy_fixture():
    pass
"#,
    );

    create_test_file(
        &temp_dir.path().join("tests"),
        "test_normal.py",
        r#"
import pytest

@pytest.fixture
def normal_fixture():
    pass
"#,
    );

    let config = Config::load(temp_dir.path());
    let db = FixtureDatabase::new();
    db.scan_workspace_with_excludes(temp_dir.path(), &config.exclude);

    // normal_fixture should be found
    assert!(
        db.definitions.contains_key("normal_fixture"),
        "normal_fixture should be found"
    );

    // generated and legacy should be excluded
    assert!(
        !db.definitions.contains_key("generated_fixture"),
        "generated_fixture should be excluded"
    );
    assert!(
        !db.definitions.contains_key("legacy_fixture"),
        "legacy_fixture should be excluded"
    );
}

// ============ Disabled Diagnostics Tests ============

#[test]
fn test_disabled_diagnostics_check() {
    let temp_dir = create_temp_project(
        r#"
[tool.pytest-language-server]
disabled_diagnostics = ["undeclared-fixture", "scope-mismatch"]
"#,
    );

    let config = Config::load(temp_dir.path());

    assert!(config.is_diagnostic_disabled("undeclared-fixture"));
    assert!(config.is_diagnostic_disabled("scope-mismatch"));
    assert!(!config.is_diagnostic_disabled("circular-dependency"));
}

#[test]
fn test_all_diagnostics_disabled() {
    let temp_dir = create_temp_project(
        r#"
[tool.pytest-language-server]
disabled_diagnostics = ["undeclared-fixture", "scope-mismatch", "circular-dependency"]
"#,
    );

    let config = Config::load(temp_dir.path());

    assert!(config.is_diagnostic_disabled("undeclared-fixture"));
    assert!(config.is_diagnostic_disabled("scope-mismatch"));
    assert!(config.is_diagnostic_disabled("circular-dependency"));
}

// ============ Skip Plugins Tests ============

#[test]
fn test_skip_plugins_check() {
    let temp_dir = create_temp_project(
        r#"
[tool.pytest-language-server]
skip_plugins = ["pytest-xdist", "pytest-cov"]
"#,
    );

    let config = Config::load(temp_dir.path());

    assert!(config.should_skip_plugin("pytest-xdist"));
    assert!(config.should_skip_plugin("pytest-cov"));
    assert!(!config.should_skip_plugin("pytest-mock"));
}

// ============ Edge Cases ============

#[test]
fn test_config_with_invalid_toml_syntax() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(
        temp_dir.path().join("pyproject.toml"),
        "this is not valid toml [[[",
    )
    .unwrap();

    let config = Config::load(temp_dir.path());
    // Should return defaults without panicking
    assert!(config.exclude.is_empty());
    assert!(config.disabled_diagnostics.is_empty());
}

#[test]
fn test_config_with_wrong_types() {
    let temp_dir = create_temp_project(
        r#"
[tool.pytest-language-server]
exclude = "should-be-array"
"#,
    );

    let config = Config::load(temp_dir.path());
    // Should return defaults when types are wrong
    assert!(config.exclude.is_empty());
}

#[test]
fn test_config_with_extra_unknown_fields() {
    let temp_dir = create_temp_project(
        r#"
[tool.pytest-language-server]
exclude = ["build"]
unknown_field = "should be ignored"
another_unknown = 42
"#,
    );

    let config = Config::load(temp_dir.path());
    // Should parse valid fields and ignore unknown ones
    assert_eq!(config.exclude.len(), 1);
}

#[test]
fn test_config_fixture_paths() {
    let temp_dir = create_temp_project(
        r#"
[tool.pytest-language-server]
fixture_paths = ["fixtures/", "shared/fixtures/"]
"#,
    );

    let config = Config::load(temp_dir.path());
    assert_eq!(config.fixture_paths, vec!["fixtures/", "shared/fixtures/"]);
}
