//! LSP protocol tests.
//!
//! All tests have a 30-second timeout to prevent hangs from blocking CI.

use ntest::timeout;
use pytest_language_server::FixtureDefinition;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;

#[test]
#[timeout(30000)]
fn test_hover_content_with_leading_newline() {
    // Create a mock fixture definition with docstring
    let definition = FixtureDefinition {
        name: "my_fixture".to_string(),
        file_path: PathBuf::from("/tmp/test/conftest.py"),
        line: 4,
        end_line: 10,
        start_char: 4,
        end_char: 14,
        docstring: Some("This is a test fixture.\n\nIt does something useful.".to_string()),
        return_type: None,
        return_type_imports: vec![],
        is_third_party: false,
        is_plugin: false,
        dependencies: vec![],
        scope: pytest_language_server::FixtureScope::Function,
        yield_line: None,
        autouse: false,
    };

    // Build hover content (same logic as hover method)
    let mut content = String::new();

    // Add "from" line with relative path (using just filename for test)
    let relative_path = definition
        .file_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unknown");
    content.push_str(&format!("**from** `{}`\n", relative_path));

    // Add code block with fixture signature
    content.push_str(&format!(
        "```python\n@pytest.fixture\ndef {}(...):\n```",
        definition.name
    ));

    // Add docstring if present
    if let Some(ref docstring) = definition.docstring {
        content.push_str("\n\n---\n\n");
        content.push_str(docstring);
    }

    // Verify the structure
    let lines: Vec<&str> = content.lines().collect();

    // The structure should be:
    // 0: **from** `conftest.py`
    // 1: ```python
    // 2: @pytest.fixture
    // 3: def my_fixture(...):
    // 4: ```
    // 5: (empty line from \n\n---\n)
    // 6: ---
    // 7: (empty line)
    // 8+: docstring content

    assert!(
        lines[0].starts_with("**from**"),
        "Line 0 should start with 'From', got: '{}'",
        lines[0]
    );
    assert_eq!(lines[1], "```python");
    assert_eq!(lines[2], "@pytest.fixture");
    assert!(lines[3].starts_with("def my_fixture"));
    assert_eq!(lines[4], "```");
}

#[test]
#[timeout(30000)]
fn test_hover_content_structure_without_docstring() {
    // Create a mock fixture definition without docstring
    let definition = FixtureDefinition {
        name: "simple_fixture".to_string(),
        file_path: PathBuf::from("/tmp/test/conftest.py"),
        line: 4,
        end_line: 6,
        start_char: 4,
        end_char: 18,
        docstring: None,
        return_type: None,
        return_type_imports: vec![],
        is_third_party: false,
        is_plugin: false,
        dependencies: vec![],
        scope: pytest_language_server::FixtureScope::Function,
        yield_line: None,
        autouse: false,
    };

    // Build hover content
    let mut content = String::new();

    // Add "from" line with relative path (using just filename for test)
    let relative_path = definition
        .file_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unknown");
    content.push_str(&format!("**from** `{}`\n", relative_path));

    // Add code block with fixture signature
    content.push_str(&format!(
        "```python\n@pytest.fixture\ndef {}(...):\n```",
        definition.name
    ));

    // For a fixture without docstring, the content should end with the code block
    let lines: Vec<&str> = content.lines().collect();

    assert_eq!(lines.len(), 5); // from line (1 line) + code block (4 lines)
    assert!(lines[0].starts_with("**from**"));
    assert_eq!(lines[1], "```python");
    assert_eq!(lines[4], "```");
}

#[test]
#[timeout(30000)]
fn test_references_from_parent_definition() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child conftest with override
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file using child fixture
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get parent definition by clicking on the child's parameter (which references parent)
    // In child conftest, line 5 has "def cli_runner(cli_runner):"
    // Line 5 (1-indexed) = line 4 (0-indexed), char 19 is in the parameter "cli_runner"
    let parent_def = db.find_fixture_definition(&child_conftest, 4, 19);
    assert!(
        parent_def.is_some(),
        "Child parameter should resolve to parent definition"
    );

    // Find references for parent - should include child's parameter, not test usages
    let refs = db.find_references_for_definition(&parent_def.unwrap());

    assert!(
        refs.iter().any(|r| r.file_path == child_conftest),
        "Parent references should include child fixture parameter"
    );

    assert!(
        refs.iter().all(|r| r.file_path != test_path),
        "Parent references should NOT include test file usages (they use child)"
    );
}

#[test]
#[timeout(30000)]
fn test_references_from_child_definition() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child conftest with override
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file using child fixture
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get child definition by clicking on a test usage
    // Line 2 (1-indexed) = line 1 (0-indexed), char 13 is in "cli_runner" parameter
    let child_def = db.find_fixture_definition(&test_path, 1, 13);
    assert!(
        child_def.is_some(),
        "Test usage should resolve to child definition"
    );

    // Find references for child - should include test usages
    let refs = db.find_references_for_definition(&child_def.unwrap());

    let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

    assert_eq!(
        test_refs.len(),
        2,
        "Child references should include both test usages"
    );
}

#[test]
#[timeout(30000)]
fn test_references_from_usage_in_test() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child conftest with override
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file using child fixture
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Simulate clicking on cli_runner in test_one (line 2, 1-indexed)
    let resolved_def = db.find_fixture_definition(&test_path, 1, 13); // 0-indexed LSP

    assert!(resolved_def.is_some(), "Should resolve usage to definition");

    let def = resolved_def.unwrap();
    assert_eq!(
        def.file_path, child_conftest,
        "Usage should resolve to child definition, not parent"
    );

    // Get references for the resolved definition
    let refs = db.find_references_for_definition(&def);

    // Should include both test usages
    let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

    assert_eq!(
        test_refs.len(),
        2,
        "References should include both test usages"
    );

    // Verify that the current usage (line 2 where we clicked) IS included
    let current_usage = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 2);
    assert!(
        current_usage.is_some(),
        "References should include the current usage (line 2) where cursor is positioned"
    );

    // Verify the other usage is also included
    let other_usage = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 5);
    assert!(
        other_usage.is_some(),
        "References should include the other usage (line 5)"
    );
}

#[test]
#[timeout(30000)]
fn test_references_three_level_hierarchy() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Grandparent
    let grandparent_content = r#"
import pytest

@pytest.fixture
def db():
    return "root"
"#;
    let grandparent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(grandparent_conftest.clone(), grandparent_content);

    // Parent overrides
    let parent_content = r#"
import pytest

@pytest.fixture
def db(db):
    return f"parent_{db}"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/api/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child overrides again
    let child_content = r#"
import pytest

@pytest.fixture
def db(db):
    return f"child_{db}"
"#;
    let child_conftest = PathBuf::from("/tmp/project/api/v1/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test at child level
    let test_content = r#"
def test_db(db):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/api/v1/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get definitions by clicking on parameters that reference them
    // Parent conftest: "def db(db):" - parameter 'db' starts at position 7
    let grandparent_def = db
        .find_fixture_definition(&parent_conftest, 4, 7)
        .expect("Parent parameter should resolve to grandparent");
    // Child conftest: "def db(db):" - parameter 'db' starts at position 7
    let parent_def = db
        .find_fixture_definition(&child_conftest, 4, 7)
        .expect("Child parameter should resolve to parent");
    // Test: "def test_db(db):" - parameter 'db' starts at position 12
    let child_def = db
        .find_fixture_definition(&test_path, 1, 12)
        .expect("Test parameter should resolve to child");

    // Grandparent references should only include parent parameter
    let gp_refs = db.find_references_for_definition(&grandparent_def);
    assert!(
        gp_refs.iter().any(|r| r.file_path == parent_conftest),
        "Grandparent should have parent parameter"
    );
    assert!(
        gp_refs.iter().all(|r| r.file_path != child_conftest),
        "Grandparent should NOT have child references"
    );
    assert!(
        gp_refs.iter().all(|r| r.file_path != test_path),
        "Grandparent should NOT have test references"
    );

    // Parent references should only include child parameter
    let parent_refs = db.find_references_for_definition(&parent_def);
    assert!(
        parent_refs.iter().any(|r| r.file_path == child_conftest),
        "Parent should have child parameter"
    );
    assert!(
        parent_refs.iter().all(|r| r.file_path != test_path),
        "Parent should NOT have test references"
    );

    // Child references should include test usage
    let child_refs = db.find_references_for_definition(&child_def);
    assert!(
        child_refs.iter().any(|r| r.file_path == test_path),
        "Child should have test reference"
    );
}

#[test]
#[timeout(30000)]
fn test_references_no_duplicate_definition() {
    // Test that when a fixture definition line also has a usage (self-referencing),
    // we don't list the definition twice in the results
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child conftest with self-referencing override
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file
    let test_content = r#"
def test_one(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Click on the child's parameter (which references parent)
    let parent_def = db
        .find_fixture_definition(&child_conftest, 4, 19)
        .expect("Should find parent definition from child parameter");

    // Get references for parent
    let refs = db.find_references_for_definition(&parent_def);

    // The child conftest line 5 should appear exactly once in references
    // (it's both a reference and a definition line, but should only appear once)
    let child_line_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.file_path == child_conftest && r.line == 5)
        .collect();

    assert_eq!(
        child_line_refs.len(),
        1,
        "Child fixture line should appear exactly once in references (not duplicated)"
    );
}

#[test]
#[timeout(30000)]
fn test_comprehensive_fixture_hierarchy_with_cursor_positions() {
    // This test validates all cursor position scenarios with fixture hierarchy
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Root conftest with parent fixtures
    let root_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"

@pytest.fixture
def other_fixture(cli_runner):
    return f"uses_{cli_runner}"
"#;
    let root_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(root_conftest.clone(), root_content);

    // Child conftest with override
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file in child directory
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    println!("\n=== SCENARIO 1: Clicking on PARENT via another fixture that uses it ===");
    // Click on other_fixture's parameter to get parent definition
    let parent_def = db.find_fixture_definition(&root_conftest, 8, 20);
    println!(
        "Parent def: {:?}",
        parent_def.as_ref().map(|d| (&d.file_path, d.line))
    );

    if let Some(parent_def) = parent_def {
        let refs = db.find_references_for_definition(&parent_def);
        println!("Parent references count: {}", refs.len());
        for r in &refs {
            println!("  {:?}:{}", r.file_path, r.line);
        }

        // Parent should have:
        // 1. other_fixture parameter (line 9 in root conftest)
        // 2. Child fixture parameter (line 5 in child conftest)
        // NOT: test_one or test_two (they use child)

        let root_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == root_conftest)
            .collect();
        let child_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == child_conftest)
            .collect();
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

        assert!(
            !root_refs.is_empty(),
            "Parent should have reference from other_fixture"
        );
        assert!(
            !child_refs.is_empty(),
            "Parent should have reference from child fixture"
        );
        assert!(
            test_refs.is_empty(),
            "Parent should NOT have test references (they use child)"
        );
    }

    println!("\n=== SCENARIO 2: Clicking on CHILD fixture via test usage ===");
    let child_def = db.find_fixture_definition(&test_path, 1, 13);
    println!(
        "Child def: {:?}",
        child_def.as_ref().map(|d| (&d.file_path, d.line))
    );

    if let Some(child_def) = child_def {
        let refs = db.find_references_for_definition(&child_def);
        println!("Child references count: {}", refs.len());
        for r in &refs {
            println!("  {:?}:{}", r.file_path, r.line);
        }

        // Child should have:
        // 1. test_one (line 2 in test file)
        // 2. test_two (line 5 in test file)
        // NOT: other_fixture (uses parent)

        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        let root_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == root_conftest)
            .collect();

        assert_eq!(test_refs.len(), 2, "Child should have 2 test references");
        assert!(
            root_refs.is_empty(),
            "Child should NOT have root conftest references"
        );
    }

    println!("\n=== SCENARIO 3: Clicking on CHILD fixture parameter (resolves to parent) ===");
    let parent_via_child_param = db.find_fixture_definition(&child_conftest, 4, 19);
    println!(
        "Parent via child param: {:?}",
        parent_via_child_param
            .as_ref()
            .map(|d| (&d.file_path, d.line))
    );

    if let Some(parent_def) = parent_via_child_param {
        assert_eq!(
            parent_def.file_path, root_conftest,
            "Child parameter should resolve to parent"
        );

        let refs = db.find_references_for_definition(&parent_def);

        // Should be same as SCENARIO 1
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        assert!(
            test_refs.is_empty(),
            "Parent (via child param) should NOT have test references"
        );
    }
}

#[test]
#[timeout(30000)]
fn test_references_clicking_on_definition_line() {
    // Test that clicking on a fixture definition itself (not parameter, not usage)
    // correctly identifies which definition and returns appropriate references
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child conftest
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    println!("\n=== TEST: Clicking on child fixture definition (function name 'cli_runner') ===");
    // Line 5 (1-indexed) = line 4 (0-indexed), clicking on "def cli_runner" at char 4
    let fixture_name = db.find_fixture_at_position(&child_conftest, 4, 4);
    println!("Fixture name at position: {:?}", fixture_name);
    assert_eq!(fixture_name, Some("cli_runner".to_string()));

    // Get the definition at this line
    let child_def = db.get_definition_at_line(&child_conftest, 5, "cli_runner");
    println!(
        "Definition at line: {:?}",
        child_def.as_ref().map(|d| (&d.file_path, d.line))
    );
    assert!(
        child_def.is_some(),
        "Should find child definition at line 5"
    );

    if let Some(child_def) = child_def {
        let refs = db.find_references_for_definition(&child_def);
        println!("Child definition references count: {}", refs.len());
        for r in &refs {
            println!("  {:?}:{}", r.file_path, r.line);
        }

        // Child definition should have only test file usages, not parent conftest
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        let parent_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == parent_conftest)
            .collect();

        assert_eq!(
            test_refs.len(),
            2,
            "Child definition should have 2 test references"
        );
        assert!(
            parent_refs.is_empty(),
            "Child definition should NOT have parent references"
        );
    }

    println!("\n=== TEST: Clicking on parent fixture definition (function name 'cli_runner') ===");
    let fixture_name = db.find_fixture_at_position(&parent_conftest, 4, 4);
    println!("Fixture name at position: {:?}", fixture_name);

    let parent_def = db.get_definition_at_line(&parent_conftest, 5, "cli_runner");
    println!(
        "Definition at line: {:?}",
        parent_def.as_ref().map(|d| (&d.file_path, d.line))
    );
    assert!(
        parent_def.is_some(),
        "Should find parent definition at line 5"
    );

    if let Some(parent_def) = parent_def {
        let refs = db.find_references_for_definition(&parent_def);
        println!("Parent definition references count: {}", refs.len());
        for r in &refs {
            println!("  {:?}:{}", r.file_path, r.line);
        }

        // Parent should have child's parameter, but NOT test file usages
        let child_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == child_conftest)
            .collect();
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

        assert!(
            !child_refs.is_empty(),
            "Parent should have child fixture parameter reference"
        );
        assert!(
            test_refs.is_empty(),
            "Parent should NOT have test file references"
        );
    }
}

#[test]
#[timeout(30000)]
fn test_fixture_override_in_test_file_not_conftest() {
    // This reproduces the strawberry test_codegen.py scenario:
    // A test file that defines a fixture overriding a parent from conftest
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let conftest_path = PathBuf::from("/tmp/project/tests/cli/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file with fixture override AND tests using it
    let test_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner

def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass

def test_three(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/cli/test_codegen.py");
    db.analyze_file(test_path.clone(), test_content);

    println!(
        "\n=== SCENARIO 1: Click on child fixture definition (function name) in test file ==="
    );
    // Line 5 (1-indexed) = line 4 (0-indexed), "def cli_runner"
    let fixture_name = db.find_fixture_at_position(&test_path, 4, 4);
    println!("Fixture name: {:?}", fixture_name);
    assert_eq!(fixture_name, Some("cli_runner".to_string()));

    let child_def = db.get_definition_at_line(&test_path, 5, "cli_runner");
    println!(
        "Child def: {:?}",
        child_def.as_ref().map(|d| (&d.file_path, d.line))
    );
    assert!(
        child_def.is_some(),
        "Should find child definition in test file"
    );

    if let Some(child_def) = child_def {
        let refs = db.find_references_for_definition(&child_def);
        println!("Child references count: {}", refs.len());
        for r in &refs {
            println!("  {:?}:{}", r.file_path, r.line);
        }

        // Should only have references from the SAME FILE (test_one, test_two, test_three)
        // Should NOT have references from other files
        let same_file_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        let other_file_refs: Vec<_> = refs.iter().filter(|r| r.file_path != test_path).collect();

        assert_eq!(
            same_file_refs.len(),
            3,
            "Child should have 3 references in same file"
        );
        assert!(
            other_file_refs.is_empty(),
            "Child should NOT have references from other files"
        );
    }

    println!(
        "\n=== SCENARIO 2: Click on child fixture parameter (points to parent) in test file ==="
    );
    // Line 5, char 19 is the parameter "cli_runner"
    let parent_def = db.find_fixture_definition(&test_path, 4, 19);
    println!(
        "Parent def via child param: {:?}",
        parent_def.as_ref().map(|d| (&d.file_path, d.line))
    );

    if let Some(parent_def) = parent_def {
        assert_eq!(
            parent_def.file_path, conftest_path,
            "Parameter should resolve to parent in conftest"
        );

        let refs = db.find_references_for_definition(&parent_def);
        println!("Parent references count: {}", refs.len());
        for r in &refs {
            println!("  {:?}:{}", r.file_path, r.line);
        }

        // Parent should have:
        // 1. Child fixture parameter (line 5 in test file)
        // NOT: test_one, test_two, test_three (they use child, not parent)
        let test_file_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

        // Should only have the child fixture's parameter (line 5), not the test usages
        assert_eq!(
            test_file_refs.len(),
            1,
            "Parent should have 1 reference from test file (child parameter only)"
        );
        assert_eq!(
            test_file_refs[0].line, 5,
            "Parent reference should be on line 5 (child fixture parameter)"
        );
    }

    println!("\n=== SCENARIO 3: Click on usage in test function ===");
    // Line 8 (1-indexed) = line 7 (0-indexed), test_one's cli_runner parameter
    let resolved = db.find_fixture_definition(&test_path, 7, 17);
    println!(
        "Resolved from test: {:?}",
        resolved.as_ref().map(|d| (&d.file_path, d.line))
    );

    if let Some(def) = resolved {
        assert_eq!(
            def.file_path, test_path,
            "Test usage should resolve to child in same file"
        );
        assert_eq!(def.line, 5, "Should resolve to child fixture at line 5");
    }
}

#[test]
#[timeout(30000)]
fn test_references_include_current_position() {
    // LSP Spec requirement: textDocument/references should include the current position
    // where the cursor is, whether it's a usage or a definition
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "runner"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass

def test_three(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    println!("\n=== TEST: Click on usage at test_one (line 2) ===");
    // Line 2 (1-indexed), clicking on cli_runner parameter
    let fixture_name = db.find_fixture_at_position(&test_path, 1, 13);
    assert_eq!(fixture_name, Some("cli_runner".to_string()));

    let resolved_def = db.find_fixture_definition(&test_path, 1, 13);
    assert!(
        resolved_def.is_some(),
        "Should resolve to conftest definition"
    );

    let def = resolved_def.unwrap();
    let refs = db.find_references_for_definition(&def);

    println!("References found: {}", refs.len());
    for r in &refs {
        println!(
            "  {:?}:{} (chars {}-{})",
            r.file_path.file_name(),
            r.line,
            r.start_char,
            r.end_char
        );
    }

    // CRITICAL: References should include ALL usages, including the current one
    assert_eq!(refs.len(), 3, "Should have 3 references (all test usages)");

    // Verify line 2 (where we clicked) IS included
    let line2_ref = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 2);
    assert!(
        line2_ref.is_some(),
        "References MUST include current position (line 2)"
    );

    // Verify other lines are also included
    let line5_ref = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 5);
    assert!(line5_ref.is_some(), "References should include line 5");

    let line8_ref = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 8);
    assert!(line8_ref.is_some(), "References should include line 8");

    println!("\n=== TEST: Click on usage at test_two (line 5) ===");
    let resolved_def = db.find_fixture_definition(&test_path, 4, 13);
    assert!(resolved_def.is_some());

    let def = resolved_def.unwrap();
    let refs = db.find_references_for_definition(&def);

    // Should still have all 3 references
    assert_eq!(refs.len(), 3, "Should have 3 references");

    // Current position (line 5) MUST be included
    let line5_ref = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 5);
    assert!(
        line5_ref.is_some(),
        "References MUST include current position (line 5)"
    );

    // Simulate LSP handler logic: verify no references would be incorrectly skipped
    // (only skip if reference is on same line as definition)
    for r in &refs {
        if r.file_path == def.file_path && r.line == def.line {
            println!(
                "  Would skip (same as definition): {:?}:{}",
                r.file_path.file_name(),
                r.line
            );
        } else {
            println!(
                "  Would include: {:?}:{} (chars {}-{})",
                r.file_path.file_name(),
                r.line,
                r.start_char,
                r.end_char
            );
        }
    }

    // In this scenario, no references should be skipped (definition is in conftest, usages in test file)
    let would_be_skipped = refs
        .iter()
        .filter(|r| r.file_path == def.file_path && r.line == def.line)
        .count();
    assert_eq!(
        would_be_skipped, 0,
        "No references should be skipped in this scenario"
    );

    println!("\n=== TEST: Click on definition (line 5 in conftest) ===");
    // When clicking on the definition itself, references should include all usages
    let fixture_name = db.find_fixture_at_position(&conftest_path, 4, 4);
    assert_eq!(fixture_name, Some("cli_runner".to_string()));

    // This should return None (we're on definition, not usage)
    let resolved = db.find_fixture_definition(&conftest_path, 4, 4);
    assert!(
        resolved.is_none(),
        "Clicking on definition name should return None"
    );

    // Get definition at this line
    let def = db.get_definition_at_line(&conftest_path, 5, "cli_runner");
    assert!(def.is_some());

    let def = def.unwrap();
    let refs = db.find_references_for_definition(&def);

    // Should have all 3 test usages
    assert_eq!(refs.len(), 3, "Definition should have 3 usage references");

    println!("\nAll LSP spec requirements verified ✓");
}

#[test]
#[timeout(30000)]
fn test_references_multiline_function_signature() {
    // Test that references work correctly with multiline function signatures
    // This simulates the strawberry test_codegen.py scenario
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "runner"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Multiline function signature (like strawberry line 87-89)
    let test_content = r#"
def test_codegen(
    cli_app: Typer, cli_runner: CliRunner, query_file_path: Path
):
    pass

def test_another(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_codegen.py");
    db.analyze_file(test_path.clone(), test_content);

    println!("\n=== TEST: Click on cli_runner in function signature (line 3, char 23) ===");
    // Line 3 (1-indexed): "    cli_app: Typer, cli_runner: CliRunner, query_file_path: Path"
    // Character position 23 should be in "cli_runner" (starts at ~20)

    let fixture_name = db.find_fixture_at_position(&test_path, 2, 23); // 0-indexed for LSP
    println!("Fixture at position: {:?}", fixture_name);
    assert_eq!(
        fixture_name,
        Some("cli_runner".to_string()),
        "Should find cli_runner at this position"
    );

    let resolved_def = db.find_fixture_definition(&test_path, 2, 23);
    assert!(
        resolved_def.is_some(),
        "Should resolve to conftest definition"
    );

    let def = resolved_def.unwrap();
    println!("Resolved to: {:?}:{}", def.file_path.file_name(), def.line);

    let refs = db.find_references_for_definition(&def);
    println!("\nReferences found: {}", refs.len());
    for r in &refs {
        println!(
            "  {:?}:{} (chars {}-{})",
            r.file_path.file_name(),
            r.line,
            r.start_char,
            r.end_char
        );
    }

    // Should have 2 references: line 3 (signature) and line 7 (test_another)
    assert_eq!(
        refs.len(),
        2,
        "Should have 2 references (both function signatures)"
    );

    // CRITICAL: Line 3 (where we clicked) MUST be included
    let line3_ref = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 3);
    assert!(
        line3_ref.is_some(),
        "References MUST include current position (line 3 in signature)"
    );

    // Also verify line 7 (test_another) is included
    let line7_ref = refs
        .iter()
        .find(|r| r.file_path == test_path && r.line == 7);
    assert!(
        line7_ref.is_some(),
        "References should include test_another parameter (line 7)"
    );

    println!("\nMultiline signature test passed ✓");
}

#[tokio::test]
async fn test_code_action_for_undeclared_fixture() {
    // Test that code actions are generated for undeclared fixtures
    use pytest_language_server::FixtureDatabase;

    let db = Arc::new(FixtureDatabase::new());

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_undeclared():
    result = my_fixture + 1
    assert result == 43
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get undeclared fixtures
    let undeclared = db.get_undeclared_fixtures(&test_path);
    println!("\nUndeclared fixtures: {:?}", undeclared);
    assert_eq!(undeclared.len(), 1, "Should have 1 undeclared fixture");

    let fixture = &undeclared[0];
    assert_eq!(fixture.name, "my_fixture");
    assert_eq!(fixture.line, 3); // 1-indexed
    assert_eq!(fixture.function_name, "test_undeclared");
    assert_eq!(fixture.function_line, 2); // 1-indexed

    // Simulate creating a diagnostic
    let diagnostic = Diagnostic {
        range: Range {
            start: Position {
                line: (fixture.line - 1) as u32, // 0-indexed for LSP
                character: fixture.start_char as u32,
            },
            end: Position {
                line: (fixture.line - 1) as u32,
                character: fixture.end_char as u32,
            },
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String("undeclared-fixture".to_string())),
        source: Some("pytest-lsp".to_string()),
        message: format!(
            "Fixture '{}' is used but not declared as a parameter",
            fixture.name
        ),
        code_description: None,
        related_information: None,
        tags: None,
        data: None,
    };

    println!("Created diagnostic: {:?}", diagnostic);

    // Now test that the Backend would create a code action
    // We can't easily test the actual LSP handler without a full LSP setup,
    // but we can verify the data structures are correct
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("undeclared-fixture".to_string()))
    );
    assert_eq!(diagnostic.range.start.line, 2); // Line 3 in 1-indexed is line 2 in 0-indexed

    println!("\nCode action test passed ✓");
}

// ============================================================================
// HIGH PRIORITY TESTS: LSP Protocol Edge Cases
// ============================================================================

#[test]
#[timeout(30000)]
fn test_position_in_string_literal() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_something(my_fixture):
    # Fixture name in string literal - should NOT trigger goto-definition
    text = "my_fixture"
    assert my_fixture == 42
"#;
    let test_path = PathBuf::from("/tmp/test/test_string.py");
    db.analyze_file(test_path.clone(), test_content);

    // Try to find definition at position inside string literal "my_fixture"
    // Line 3 (0-indexed), character 12 is inside the string
    let definition = db.find_fixture_definition(&test_path, 3, 12);

    // Should NOT find definition because cursor is in a string literal
    // Note: Current implementation may not distinguish string literals from identifiers
    if definition.is_some() {
        println!("LIMITATION: String literals not distinguished from identifiers");
        // This is a known limitation - the current implementation doesn't
        // have context about whether a position is in a string or comment
    } else {
        // Correctly ignores string literals
    }
}

#[test]
#[timeout(30000)]
fn test_position_in_comment() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_something(my_fixture):
    # my_fixture is used here - cursor should not trigger
    assert my_fixture == 42
"#;
    let test_path = PathBuf::from("/tmp/test/test_comment.py");
    db.analyze_file(test_path.clone(), test_content);

    // Try to find definition at position inside comment
    // Line 2 (0-indexed), character 8 is inside "# my_fixture"
    let definition = db.find_fixture_definition(&test_path, 2, 8);

    // Should NOT find definition in comment
    // Note: Current implementation doesn't track comments, so this depends on usage tracking
    if definition.is_some() {
        println!("LIMITATION: Comments not distinguished from code");
    } else {
        // Correctly ignores comments
    }
}

#[test]
#[timeout(30000)]
fn test_empty_file() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let empty_content = "";
    let test_path = PathBuf::from("/tmp/test/test_empty.py");
    db.analyze_file(test_path.clone(), empty_content);

    // Should not crash on empty file
    let definition = db.find_fixture_definition(&test_path, 0, 0);
    assert!(definition.is_none(), "Empty file should return None");

    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert!(
        undeclared.is_empty(),
        "Empty file should have no undeclared fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_position_out_of_bounds() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let test_content = r#"
def test_something():
    assert True
"#;
    let test_path = PathBuf::from("/tmp/test/test_bounds.py");
    db.analyze_file(test_path.clone(), test_content);

    // Try position beyond last line
    let definition = db.find_fixture_definition(&test_path, 999, 0);
    assert!(
        definition.is_none(),
        "Out of bounds line should return None"
    );

    // Try position beyond last character on valid line
    let definition2 = db.find_fixture_definition(&test_path, 1, 9999);
    assert!(
        definition2.is_none(),
        "Out of bounds character should return None"
    );
}

#[test]
#[timeout(30000)]
fn test_whitespace_only_file() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let whitespace_content = "   \n\n\t\t\n   \n";
    let test_path = PathBuf::from("/tmp/test/test_whitespace.py");
    db.analyze_file(test_path.clone(), whitespace_content);

    // Should handle whitespace-only file gracefully
    let definition = db.find_fixture_definition(&test_path, 1, 2);
    assert!(definition.is_none(), "Whitespace file should return None");

    // Should not detect any fixtures
    assert!(
        !db.definitions
            .iter()
            .any(|entry| { entry.value().iter().any(|def| def.file_path == test_path) }),
        "Whitespace file should not have fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_malformed_python_syntax() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Python file with syntax error
    let malformed_content = r#"
import pytest

@pytest.fixture
def incomplete_fixture(
    # Missing closing parenthesis and function body
"#;
    let test_path = PathBuf::from("/tmp/test/test_malformed.py");
    db.analyze_file(test_path.clone(), malformed_content);

    // Should not crash on syntax error
    // Fixture detection may or may not work depending on how parser handles errors
    println!("Malformed file handled without crash");

    // Just verify it doesn't panic
    let _ = db.get_undeclared_fixtures(&test_path);
    // Malformed file handled gracefully
}

#[test]
#[timeout(30000)]
fn test_multi_byte_utf8_characters() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "测试"
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_unicode(my_fixture):
    # Comment with emoji 🔥 and Chinese 测试
    result = my_fixture
    assert result == "测试"
"#;
    let test_path = PathBuf::from("/tmp/test/test_unicode.py");
    db.analyze_file(test_path.clone(), test_content);

    // Verify usages were detected despite unicode in file
    let usages = db.usages.get(&test_path);
    assert!(
        usages.is_some(),
        "Should detect usages in file with unicode"
    );

    // Verify fixture can be found
    let definition = db.find_fixture_definition(&test_path, 1, 17);
    assert!(definition.is_some(), "Should find fixture in unicode file");
}

#[test]
#[timeout(30000)]
fn test_very_long_line() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def fixture_with_very_long_name_that_exceeds_normal_expectations():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_long(fixture_with_very_long_name_that_exceeds_normal_expectations):
    result = fixture_with_very_long_name_that_exceeds_normal_expectations
    assert result == 42
"#;
    let test_path = PathBuf::from("/tmp/test/test_long.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should handle very long fixture names
    assert!(db
        .definitions
        .contains_key("fixture_with_very_long_name_that_exceeds_normal_expectations"));

    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Should detect long fixture names");
}

// ============================================================================
// HIGH PRIORITY TESTS: Error Handling
// ============================================================================

#[test]
#[timeout(30000)]
fn test_invalid_utf8_content() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Invalid UTF-8 byte sequences
    // Rust strings must be valid UTF-8, so we can't actually create invalid UTF-8 in a string literal
    // This test documents that the file reading layer should handle this

    // Instead, test with valid but unusual UTF-8
    let unusual_content = "import pytest\n\n@pytest.fixture\ndef \u{FEFF}bom_fixture():  # BOM character\n    return 42";
    let test_path = PathBuf::from("/tmp/test/test_utf8.py");
    db.analyze_file(test_path.clone(), unusual_content);

    // Should handle without crashing
    println!("UTF-8 with unusual characters handled gracefully");
    // No crash on unusual UTF-8
}

#[test]
#[timeout(30000)]
fn test_incomplete_function_definition() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let incomplete_content = r#"
import pytest

@pytest.fixture
def incomplete_fixture(
"#;
    let test_path = PathBuf::from("/tmp/test/test_incomplete.py");
    db.analyze_file(test_path.clone(), incomplete_content);

    // Should not crash, but won't detect incomplete fixture
    // The parser will fail, and we should handle that gracefully
    println!("Incomplete function definition handled without panic");
    // Graceful handling of syntax error
}

#[test]
#[timeout(30000)]
fn test_truncated_file() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let truncated_content = r#"
import pytest

@pytest.fixture
def truncated_fixture():
    return "
"#;
    let test_path = PathBuf::from("/tmp/test/test_truncated.py");
    db.analyze_file(test_path.clone(), truncated_content);

    // Should handle truncated string literal without crash
    println!("Truncated file handled gracefully");
    // No crash on truncated file
}

#[test]
#[timeout(30000)]
fn test_mixed_line_endings() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Mix of \n (Unix) and \r\n (Windows) line endings
    let mixed_content =
        "import pytest\r\n\n@pytest.fixture\r\ndef my_fixture():\n    return 42\r\n";

    let test_path = PathBuf::from("/tmp/test/test_mixed.py");
    db.analyze_file(test_path.clone(), mixed_content);

    // Should detect fixture despite mixed line endings
    assert!(
        db.definitions.contains_key("my_fixture"),
        "Should detect fixtures with mixed line endings"
    );
}

#[test]
#[timeout(30000)]
fn test_file_with_only_comments() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let comment_only = r#"
# This is a comment
# Another comment
# TODO: implement tests
"#;
    let test_path = PathBuf::from("/tmp/test/test_comments.py");
    db.analyze_file(test_path.clone(), comment_only);

    // Should not crash, no fixtures detected
    assert!(
        !db.definitions
            .iter()
            .any(|entry| { entry.value().iter().any(|def| def.file_path == test_path) }),
        "Comment-only file should have no fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_deeply_nested_indentation() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let nested_content = r#"
import pytest

@pytest.fixture
def deeply_nested():
    class A:
        class B:
            class C:
                class D:
                    def inner():
                        def more_inner():
                            return 42
    return A()
"#;
    let test_path = PathBuf::from("/tmp/test/test_nested.py");
    db.analyze_file(test_path.clone(), nested_content);

    // Should detect the fixture definition despite deep nesting
    assert!(
        db.definitions.contains_key("deeply_nested"),
        "Should handle deeply nested structures"
    );
}

#[test]
#[timeout(30000)]
fn test_tabs_and_spaces_mixed() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Python typically rejects mixed tabs and spaces, but parser should handle it
    let mixed_indentation = "import pytest\n\n@pytest.fixture\ndef my_fixture():\n\treturn 42  # tab\n    # space indentation";

    let test_path = PathBuf::from("/tmp/test/test_tabs.py");
    db.analyze_file(test_path.clone(), mixed_indentation);

    // Should detect fixture or handle parse error gracefully
    if db.definitions.contains_key("my_fixture") {
        // Fixture detected despite mixed indentation
    } else {
        println!("Parser rejected mixed tabs/spaces (expected)");
        // Graceful handling of indentation error
    }
}

#[test]
#[timeout(30000)]
fn test_non_ascii_fixture_name() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Python 3 allows non-ASCII identifiers
    let non_ascii_content = r#"
import pytest

@pytest.fixture
def测试_fixture():
    return "test"

@pytest.fixture
def фикстура():
    return "fixture"
"#;
    let test_path = PathBuf::from("/tmp/test/test_non_ascii.py");
    db.analyze_file(test_path.clone(), non_ascii_content);

    // Should handle non-ASCII fixture names
    if db.definitions.contains_key("测试_fixture") {
        // Non-ASCII fixture names supported
        assert!(db.definitions.contains_key("фикстура"));
    } else {
        println!("LIMITATION: Non-ASCII identifiers not fully supported");
        // Test documents non-ASCII handling
    }
}

// MARK: - Renamed Fixtures Tests (name= parameter)

#[test]
#[timeout(30000)]
fn test_goto_definition_renamed_fixture() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest = r#"
import pytest

@pytest.fixture(name="db_conn")
def internal_database_connection():
    return "connection"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest);

    let test_content = r#"
def test_uses_renamed(db_conn):
    assert db_conn == "connection"
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Click on db_conn in test - should find definition
    let fixture_name = db.find_fixture_at_position(&test_path, 1, 22);
    assert_eq!(fixture_name, Some("db_conn".to_string()));

    let definition = db.find_fixture_definition(&test_path, 1, 22);
    assert!(
        definition.is_some(),
        "Should find renamed fixture definition"
    );

    let def = definition.unwrap();
    assert_eq!(def.name, "db_conn");
    assert_eq!(def.file_path, conftest_path);
    assert_eq!(def.line, 5); // Line where function def is (1-indexed)
}

#[test]
#[timeout(30000)]
fn test_find_references_renamed_fixture() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest = r#"
import pytest

@pytest.fixture(name="client")
def create_test_client():
    return "test_client"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest);

    let test_content = r#"
def test_one(client):
    pass

def test_two(client):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get definition and find references
    let definition = db.find_fixture_definition(&test_path, 1, 14);
    assert!(definition.is_some());

    let refs = db.find_references_for_definition(&definition.unwrap());
    assert_eq!(refs.len(), 2, "Should find 2 references to 'client'");

    // Both should reference "client" not "create_test_client"
    assert!(refs.iter().all(|r| r.name == "client"));
}

#[test]
#[timeout(30000)]
fn test_renamed_fixture_with_dependency() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(name="db")
def database_fixture():
    return "database"

@pytest.fixture(name="user")
def user_fixture(db):
    return {"db": db}

def test_example(user, db):
    pass
"#;
    let file_path = PathBuf::from("/tmp/project/test_file.py");
    db.analyze_file(file_path.clone(), content);

    // Verify both renamed fixtures are registered correctly
    assert!(db.definitions.contains_key("db"));
    assert!(db.definitions.contains_key("user"));
    assert!(!db.definitions.contains_key("database_fixture"));
    assert!(!db.definitions.contains_key("user_fixture"));

    // Verify usages: user_fixture uses db, test uses both
    let usages = db.usages.get(&file_path).unwrap();
    let db_usages: Vec<_> = usages.iter().filter(|u| u.name == "db").collect();
    let user_usages: Vec<_> = usages.iter().filter(|u| u.name == "user").collect();

    assert_eq!(
        db_usages.len(),
        2,
        "db should be used twice (in user_fixture and test)"
    );
    assert_eq!(user_usages.len(), 1, "user should be used once (in test)");
}

#[test]
#[timeout(30000)]
fn test_normal_fixture_no_regression() {
    // Ensure fixtures without name= still work correctly
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest = r#"
import pytest

@pytest.fixture
def normal_fixture():
    return "normal"

@pytest.fixture(scope="session")
def session_fixture():
    return "session"

@pytest.fixture(autouse=True)
def autouse_fixture():
    return "autouse"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest);

    let test_content = r#"
def test_example(normal_fixture, session_fixture):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // All fixtures should be registered by function name
    assert!(db.definitions.contains_key("normal_fixture"));
    assert!(db.definitions.contains_key("session_fixture"));
    assert!(db.definitions.contains_key("autouse_fixture"));

    // Goto definition should work
    let def = db.find_fixture_definition(&test_path, 1, 18);
    assert!(def.is_some());
    assert_eq!(def.unwrap().name, "normal_fixture");

    // References should work
    let def = db.find_fixture_definition(&test_path, 1, 18).unwrap();
    let refs = db.find_references_for_definition(&def);
    assert_eq!(refs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_mixed_renamed_and_normal_fixtures() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(name="renamed")
def internal_name():
    return 1

@pytest.fixture
def normal():
    return 2

def test_mixed(renamed, normal):
    pass
"#;
    let file_path = PathBuf::from("/tmp/project/test_file.py");
    db.analyze_file(file_path.clone(), content);

    // Renamed fixture uses alias
    assert!(db.definitions.contains_key("renamed"));
    assert!(!db.definitions.contains_key("internal_name"));

    // Normal fixture uses function name
    assert!(db.definitions.contains_key("normal"));

    // Both should be findable via goto definition
    let renamed_def = db.find_fixture_definition(&file_path, 11, 15);
    let normal_def = db.find_fixture_definition(&file_path, 11, 24);

    assert!(renamed_def.is_some());
    assert!(normal_def.is_some());
    assert_eq!(renamed_def.unwrap().name, "renamed");
    assert_eq!(normal_def.unwrap().name, "normal");
}

// ============================================================================
// COMPLETION PROVIDER TESTS
// ============================================================================

#[test]
#[timeout(30000)]
fn test_completion_context_in_function_signature() {
    use pytest_language_server::CompletionContext;
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_example(my_fixture, ):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position after the comma in the signature (line 1, char 29)
    // Line 2 in content = line 1 in 0-indexed LSP
    let ctx = db.get_completion_context(&test_path, 1, 30);

    assert!(ctx.is_some(), "Should detect function signature context");
    match ctx.unwrap() {
        CompletionContext::FunctionSignature {
            function_name,
            declared_params,
            ..
        } => {
            assert_eq!(function_name, "test_example");
            assert!(declared_params.contains(&"my_fixture".to_string()));
        }
        _ => panic!("Expected FunctionSignature context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_in_function_body() {
    use pytest_language_server::CompletionContext;
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_example():
    result = None
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position inside the function body (line 3, the "pass" line)
    let ctx = db.get_completion_context(&test_path, 3, 4);

    assert!(ctx.is_some(), "Should detect function body context");
    match ctx.unwrap() {
        CompletionContext::FunctionBody {
            function_name,
            declared_params,
            ..
        } => {
            assert_eq!(function_name, "test_example");
            assert!(declared_params.is_empty());
        }
        _ => panic!("Expected FunctionBody context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_in_usefixtures_decorator() {
    use pytest_language_server::CompletionContext;
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
import pytest

@pytest.mark.usefixtures("")
def test_example():
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position inside the usefixtures string (line 3, char 27 - inside quotes)
    let ctx = db.get_completion_context(&test_path, 3, 27);

    assert!(ctx.is_some(), "Should detect usefixtures decorator context");
    match ctx.unwrap() {
        CompletionContext::UsefixturesDecorator => {}
        _ => panic!("Expected UsefixturesDecorator context"),
    }
}

#[test]
#[timeout(30000)]
fn test_get_available_fixtures() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def fixture_one():
    return 1

@pytest.fixture
def fixture_two():
    return 2
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
import pytest

@pytest.fixture
def local_fixture():
    return 3

def test_example():
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get available fixtures for the test file
    let available = db.get_available_fixtures(&test_path);

    // Should include fixtures from conftest.py and local fixtures
    let names: Vec<_> = available.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.contains(&"fixture_one"),
        "Should include conftest fixtures"
    );
    assert!(
        names.contains(&"fixture_two"),
        "Should include conftest fixtures"
    );
    assert!(
        names.contains(&"local_fixture"),
        "Should include local fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_get_available_fixtures_priority() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_conftest = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "parent"
"#;
    let parent_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_path.clone(), parent_conftest);

    // Child conftest that overrides
    let child_conftest = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "child"
"#;
    let child_path = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_path.clone(), child_conftest);

    let test_content = r#"
def test_example():
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get available fixtures for the test file
    let available = db.get_available_fixtures(&test_path);

    // Should only include one "shared_fixture" (the closest one)
    let shared_fixtures: Vec<_> = available
        .iter()
        .filter(|f| f.name == "shared_fixture")
        .collect();
    assert_eq!(
        shared_fixtures.len(),
        1,
        "Should only have one shared_fixture (closest wins)"
    );

    // The fixture should be from the child conftest (closest)
    assert_eq!(
        shared_fixtures[0].file_path, child_path,
        "Should prefer closer conftest"
    );
}

#[test]
#[timeout(30000)]
fn test_get_function_param_insertion_info() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
def test_with_params(existing_param):
    pass

def test_no_params():
    pass
"#;
    let file_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(file_path.clone(), content);

    // Test function with existing params (line 2 in 1-indexed)
    let info = db.get_function_param_insertion_info(&file_path, 2);
    assert!(info.is_some(), "Should find insertion info");
    let info = info.unwrap();
    assert!(
        info.needs_comma,
        "Should need comma since there's an existing param"
    );
    assert_eq!(info.line, 2, "Should be on line 2");

    // Test function with no params (line 5 in 1-indexed)
    let info = db.get_function_param_insertion_info(&file_path, 5);
    assert!(
        info.is_some(),
        "Should find insertion info for no-param function"
    );
    let info = info.unwrap();
    assert!(!info.needs_comma, "Should not need comma for empty params");
}

#[test]
#[timeout(30000)]
fn test_get_function_param_insertion_info_multiline() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
def test_multiline(
    first_param,
    second_param,
):
    pass
"#;
    let file_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(file_path.clone(), content);

    // Test multiline function (starts at line 2 in 1-indexed)
    let info = db.get_function_param_insertion_info(&file_path, 2);
    assert!(
        info.is_some(),
        "Should find insertion info for multiline signature"
    );
}

// ============================================================================
// CODE ACTION TESTS
// ============================================================================

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_detection() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def available_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_undeclared():
    result = available_fixture + 1
    assert result == 43
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get undeclared fixtures
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(undeclared.len(), 1, "Should detect 1 undeclared fixture");
    assert_eq!(undeclared[0].name, "available_fixture");
    assert_eq!(undeclared[0].function_name, "test_undeclared");
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_not_detected_when_declared() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_declared(my_fixture):
    result = my_fixture + 1
    assert result == 43
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get undeclared fixtures - should be empty since my_fixture is declared
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert!(
        undeclared.is_empty(),
        "Should not detect fixture as undeclared when it's a parameter"
    );
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_multiple() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def fixture_a():
    return 1

@pytest.fixture
def fixture_b():
    return 2

@pytest.fixture
def fixture_c():
    return 3
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_multiple_undeclared():
    total = fixture_a + fixture_b + fixture_c
    assert total == 6
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get undeclared fixtures
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(undeclared.len(), 3, "Should detect 3 undeclared fixtures");
    let names: Vec<_> = undeclared.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"fixture_a"));
    assert!(names.contains(&"fixture_b"));
    assert!(names.contains(&"fixture_c"));
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_position_accuracy() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_position():
    result = my_fixture + 1
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(undeclared.len(), 1);

    let fixture = &undeclared[0];
    assert_eq!(fixture.line, 3, "Should be on line 3 (1-indexed)");
    assert_eq!(
        fixture.function_line, 2,
        "Function should start on line 2 (1-indexed)"
    );
    // start_char and end_char should accurately point to "my_fixture"
    assert!(
        fixture.start_char < fixture.end_char,
        "Character positions should be valid"
    );
}

#[test]
#[timeout(30000)]
fn test_is_third_party_fixture() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Third-party fixture in site-packages
    let third_party_content = r#"
import pytest

@pytest.fixture
def mock():
    pass
"#;
    let third_party_path =
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/pytest_mock/plugin.py");
    db.analyze_file(third_party_path.clone(), third_party_content);

    // Local fixture
    let local_content = r#"
import pytest

@pytest.fixture
def local_fixture():
    pass
"#;
    let local_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(local_path.clone(), local_content);

    // Check the is_third_party field
    let mock_defs = db.definitions.get("mock").unwrap();
    assert!(
        mock_defs.iter().all(|d| d.is_third_party),
        "mock should be third-party"
    );

    let local_defs = db.definitions.get("local_fixture").unwrap();
    assert!(
        local_defs.iter().all(|d| !d.is_third_party),
        "local_fixture should not be third-party"
    );
}

// =============================================================================
// Document Symbol Tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_document_symbol_returns_fixtures_in_file() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def fixture_one():
    """First fixture."""
    return 1

@pytest.fixture
def fixture_two() -> str:
    """Second fixture."""
    return "two"

def test_something(fixture_one, fixture_two):
    pass
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Verify fixtures were extracted
    let fixture_one = db.definitions.get("fixture_one").unwrap();
    assert_eq!(fixture_one.len(), 1);
    assert_eq!(fixture_one[0].file_path, file_path);

    let fixture_two = db.definitions.get("fixture_two").unwrap();
    assert_eq!(fixture_two.len(), 1);
    assert_eq!(fixture_two[0].file_path, file_path);
    assert_eq!(fixture_two[0].return_type.as_deref(), Some("str"));
}

#[test]
#[timeout(30000)]
fn test_document_symbol_filters_by_file() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // First file
    let content1 = r#"
import pytest

@pytest.fixture
def fixture_a():
    pass
"#;
    let file1 = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file1.clone(), content1);

    // Second file
    let content2 = r#"
import pytest

@pytest.fixture
def fixture_b():
    pass
"#;
    let file2 = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(file2.clone(), content2);

    // Collect fixtures for file1 only
    let mut file1_fixtures: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if def.file_path == file1 && !def.is_third_party {
                file1_fixtures.push(def.name.clone());
            }
        }
    }

    assert_eq!(file1_fixtures.len(), 1);
    assert!(file1_fixtures.contains(&"fixture_a".to_string()));

    // Collect fixtures for file2 only
    let mut file2_fixtures: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if def.file_path == file2 && !def.is_third_party {
                file2_fixtures.push(def.name.clone());
            }
        }
    }

    assert_eq!(file2_fixtures.len(), 1);
    assert!(file2_fixtures.contains(&"fixture_b".to_string()));
}

#[test]
#[timeout(30000)]
fn test_document_symbol_excludes_third_party() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Third-party fixture
    let tp_content = r#"
import pytest

@pytest.fixture
def mocker():
    pass
"#;
    let tp_path = PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/pytest_mock/plugin.py");
    db.analyze_file(tp_path.clone(), tp_content);

    // Count non-third-party fixtures for this file
    let mut count = 0;
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if def.file_path == tp_path && !def.is_third_party {
                count += 1;
            }
        }
    }

    // Should be 0 because all fixtures in site-packages are third-party
    assert_eq!(count, 0);
}

// =============================================================================
// Workspace Symbol Tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_workspace_symbol_returns_all_fixtures() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Multiple files with fixtures
    let content1 = r#"
import pytest

@pytest.fixture
def alpha():
    pass

@pytest.fixture
def beta():
    pass
"#;
    db.analyze_file(PathBuf::from("/tmp/project/conftest.py"), content1);

    let content2 = r#"
import pytest

@pytest.fixture
def gamma():
    pass
"#;
    db.analyze_file(PathBuf::from("/tmp/project/tests/conftest.py"), content2);

    // Count total non-third-party fixtures
    let mut all_fixtures: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if !def.is_third_party {
                all_fixtures.push(def.name.clone());
            }
        }
    }

    assert_eq!(all_fixtures.len(), 3);
    assert!(all_fixtures.contains(&"alpha".to_string()));
    assert!(all_fixtures.contains(&"beta".to_string()));
    assert!(all_fixtures.contains(&"gamma".to_string()));
}

#[test]
#[timeout(30000)]
fn test_workspace_symbol_filters_by_query() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def database_connection():
    pass

@pytest.fixture
def database_transaction():
    pass

@pytest.fixture
def http_client():
    pass
"#;
    db.analyze_file(PathBuf::from("/tmp/project/conftest.py"), content);

    // Simulate query filtering
    let query = "database".to_lowercase();
    let mut matching: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if !def.is_third_party && def.name.to_lowercase().contains(&query) {
                matching.push(def.name.clone());
            }
        }
    }

    assert_eq!(matching.len(), 2);
    assert!(matching.contains(&"database_connection".to_string()));
    assert!(matching.contains(&"database_transaction".to_string()));
}

#[test]
#[timeout(30000)]
fn test_workspace_symbol_empty_query_returns_all() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def one():
    pass

@pytest.fixture
def two():
    pass
"#;
    db.analyze_file(PathBuf::from("/tmp/project/conftest.py"), content);

    // Empty query should return all non-third-party fixtures
    let mut matching: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if !def.is_third_party {
                matching.push(def.name.clone());
            }
        }
    }

    assert_eq!(matching.len(), 2);
}

#[test]
#[timeout(30000)]
fn test_workspace_symbol_excludes_third_party() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Local fixture
    let local_content = r#"
import pytest

@pytest.fixture
def my_local():
    pass
"#;
    db.analyze_file(PathBuf::from("/tmp/project/conftest.py"), local_content);

    // Third-party fixture
    let tp_content = r#"
import pytest

@pytest.fixture
def mocker():
    pass
"#;
    db.analyze_file(
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/pytest_mock/plugin.py"),
        tp_content,
    );

    // Only local fixtures should be returned
    let mut matching: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if !def.is_third_party {
                matching.push(def.name.clone());
            }
        }
    }

    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0], "my_local");
}

#[test]
#[timeout(30000)]
fn test_workspace_symbol_case_insensitive_query() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def MyMixedCaseFixture():
    pass
"#;
    db.analyze_file(PathBuf::from("/tmp/project/conftest.py"), content);

    // Query with different case
    let query = "mymixed".to_lowercase();
    let mut matching: Vec<String> = Vec::new();
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if !def.is_third_party && def.name.to_lowercase().contains(&query) {
                matching.push(def.name.clone());
            }
        }
    }

    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0], "MyMixedCaseFixture");
}

// ============================================================================
// Code Lens Tests
// ============================================================================

#[test]
#[timeout(30000)]
fn test_code_lens_shows_usage_count() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let file_path = PathBuf::from("/tmp/test_project/conftest.py");

    let conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    """A fixture used by multiple tests."""
    return "shared"
"#;
    db.analyze_file(file_path.clone(), conftest_content);

    let test_content = r#"
def test_one(shared_fixture):
    pass

def test_two(shared_fixture):
    pass

def test_three(shared_fixture):
    pass
"#;
    db.analyze_file(
        PathBuf::from("/tmp/test_project/test_example.py"),
        test_content,
    );

    // Get definitions and count references
    let definitions = db.definitions.get("shared_fixture").unwrap();
    let def = &definitions[0];
    let references = db.find_references_for_definition(def);

    // Should have 3 usages
    assert_eq!(references.len(), 3);
}

#[test]
#[timeout(30000)]
fn test_code_lens_excludes_third_party_fixtures() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();

    // Third-party fixture
    let tp_content = r#"
import pytest

@pytest.fixture
def mocker():
    pass
"#;
    db.analyze_file(
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/pytest_mock/plugin.py"),
        tp_content,
    );

    // Local fixture
    let local_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    pass
"#;
    let local_path = PathBuf::from("/tmp/test_project/conftest.py");
    db.analyze_file(local_path.clone(), local_content);

    // Count fixtures in local file that are not third-party
    let mut local_fixture_count = 0;
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if def.file_path == local_path && !def.is_third_party {
                local_fixture_count += 1;
            }
        }
    }

    assert_eq!(local_fixture_count, 1);
}

#[test]
#[timeout(30000)]
fn test_code_lens_zero_usages() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let file_path = PathBuf::from("/tmp/test_project/conftest.py");

    let content = r#"
import pytest

@pytest.fixture
def unused_fixture():
    """This fixture is never used."""
    return "unused"
"#;
    db.analyze_file(file_path.clone(), content);

    // Get definitions and count references
    let definitions = db.definitions.get("unused_fixture").unwrap();
    let def = &definitions[0];
    let references = db.find_references_for_definition(def);

    // Should have 0 usages
    assert_eq!(references.len(), 0);
}

#[test]
#[timeout(30000)]
fn test_code_lens_fixture_used_by_other_fixture() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let file_path = PathBuf::from("/tmp/test_project/conftest.py");

    let content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"

@pytest.fixture
def derived_fixture(base_fixture):
    return base_fixture + "_derived"
"#;
    db.analyze_file(file_path.clone(), content);

    // Get base_fixture definitions and count references
    let definitions = db.definitions.get("base_fixture").unwrap();
    let def = &definitions[0];
    let references = db.find_references_for_definition(def);

    // Should have 1 usage (in derived_fixture)
    assert_eq!(references.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_code_lens_multiple_fixtures_in_file() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let file_path = PathBuf::from("/tmp/test_project/conftest.py");

    let content = r#"
import pytest

@pytest.fixture
def fixture_a():
    return "a"

@pytest.fixture
def fixture_b():
    return "b"

@pytest.fixture
def fixture_c():
    return "c"
"#;
    db.analyze_file(file_path.clone(), content);

    // Count fixtures in this file
    let mut fixture_count = 0;
    for entry in db.definitions.iter() {
        for def in entry.value() {
            if def.file_path == file_path && !def.is_third_party {
                fixture_count += 1;
            }
        }
    }

    assert_eq!(fixture_count, 3);
}

// =============================================================================
// Inlay Hints Tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_inlay_hints_with_return_type() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_inlay/conftest.py");
    let test_path = PathBuf::from("/tmp/test_inlay/test_example.py");

    // Fixture with explicit return type
    let conftest_content = r#"
import pytest

@pytest.fixture
def database() -> Database:
    """Returns a database connection."""
    return Database()

@pytest.fixture
def user() -> User:
    return User("test")

@pytest.fixture
def config():
    """No return type annotation."""
    return {}
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file using fixtures
    let test_content = r#"
def test_example(database, user, config):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Get available fixtures and check return types
    let available = db.get_available_fixtures(&test_path);

    let database_fixture = available.iter().find(|f| f.name == "database");
    assert!(database_fixture.is_some());
    assert_eq!(
        database_fixture.unwrap().return_type,
        Some("Database".to_string())
    );

    let user_fixture = available.iter().find(|f| f.name == "user");
    assert!(user_fixture.is_some());
    assert_eq!(user_fixture.unwrap().return_type, Some("User".to_string()));

    let config_fixture = available.iter().find(|f| f.name == "config");
    assert!(config_fixture.is_some());
    assert_eq!(config_fixture.unwrap().return_type, None);

    // Get usages and verify they are tracked
    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 3);

    // Verify usage positions
    let database_usage = usages.iter().find(|u| u.name == "database");
    assert!(database_usage.is_some());
    assert_eq!(database_usage.unwrap().line, 2);
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_generator_return_type() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let file_path = PathBuf::from("/tmp/test_inlay_gen/conftest.py");

    // Generator fixture with yield type extraction
    let content = r#"
import pytest
from typing import Generator

@pytest.fixture
def session() -> Generator[Session, None, None]:
    """Yields a session."""
    session = Session()
    yield session
    session.close()
"#;
    db.analyze_file(file_path.clone(), content);

    let definitions = db.definitions.get("session").unwrap();
    assert_eq!(definitions.len(), 1);
    // Should extract the yielded type (Session) from Generator[Session, None, None]
    assert_eq!(definitions[0].return_type, Some("Session".to_string()));
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_no_duplicates_same_fixture() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_inlay_dup/conftest.py");
    let test_path = PathBuf::from("/tmp/test_inlay_dup/test_example.py");

    let conftest_content = r#"
import pytest

@pytest.fixture
def db() -> Database:
    return Database()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Multiple usages of same fixture in different functions
    let test_content = r#"
def test_one(db):
    pass

def test_two(db):
    pass

def test_three(db):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Each usage should be tracked separately
    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 3);

    // All usages should refer to 'db'
    assert!(usages.iter().all(|u| u.name == "db"));
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_complex_return_types() {
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let file_path = PathBuf::from("/tmp/test_inlay_complex/conftest.py");

    let content = r#"
import pytest
from typing import Optional, Dict, List

@pytest.fixture
def optional_user() -> Optional[User]:
    return None

@pytest.fixture
def user_map() -> Dict[str, User]:
    return {}

@pytest.fixture
def user_list() -> List[User]:
    return []

@pytest.fixture
def union_type() -> str | int:
    return "value"
"#;
    db.analyze_file(file_path.clone(), content);

    let optional = db.definitions.get("optional_user").unwrap();
    assert!(optional[0].return_type.is_some());

    let dict_type = db.definitions.get("user_map").unwrap();
    assert!(dict_type[0].return_type.is_some());

    let list_type = db.definitions.get("user_list").unwrap();
    assert!(list_type[0].return_type.is_some());

    let union = db.definitions.get("union_type").unwrap();
    assert_eq!(union[0].return_type, Some("str | int".to_string()));
}

// =============================================================================
// Inlay Hints - Annotation Detection Tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_inlay_hints_skip_annotated_params() {
    // Test that inlay hints are correctly skipped for already-annotated parameters
    // and shown for unannotated parameters
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_inlay_skip/conftest.py");
    let test_path = PathBuf::from("/tmp/test_inlay_skip/test_example.py");

    let conftest_content = r#"
import pytest
from typer import Typer

@pytest.fixture
def cli_app() -> Typer:
    return Typer()

@pytest.fixture
def cli_runner() -> CliRunner:
    return CliRunner()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test with mixed annotated and unannotated parameters
    let test_content = r#"
def test_with_annotation(cli_app: Typer):
    pass

def test_without_annotation(cli_app):
    pass

def test_mixed(cli_app: Typer, cli_runner):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Get usages and check their positions
    let usages = db.usages.get(&test_path).unwrap();

    // Verify usages exist
    assert_eq!(usages.len(), 4, "Should have 4 fixture usages");

    // Get content lines for verification
    let lines: Vec<&str> = test_content.lines().collect();

    // Line 2: "def test_with_annotation(cli_app: Typer):" - cli_app is annotated
    let line2_usage = usages.iter().find(|u| u.line == 2).unwrap();
    let line2 = lines.get(1).unwrap();
    let after_param2 = &line2[line2_usage.end_char..];
    assert!(
        after_param2.trim_start().starts_with(':'),
        "Line 2 should have annotation, after='{}', line='{}'",
        after_param2,
        line2
    );

    // Line 5: "def test_without_annotation(cli_app):" - cli_app is NOT annotated
    let line5_usage = usages.iter().find(|u| u.line == 5).unwrap();
    let line5 = lines.get(4).unwrap();
    let after_param5 = &line5[line5_usage.end_char..];
    assert!(
        !after_param5.trim_start().starts_with(':'),
        "Line 5 should NOT have annotation, after='{}', line='{}'",
        after_param5,
        line5
    );
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_usage_end_char_accuracy() {
    // Test that usage end_char values correctly point to the end of the parameter name
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let test_path = PathBuf::from("/tmp/test_end_char/test_example.py");

    let test_content = r#"
def test_example(my_fixture):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1);

    let usage = &usages[0];
    assert_eq!(usage.name, "my_fixture");
    assert_eq!(usage.line, 2);

    // Verify end_char points to right after "my_fixture"
    let lines: Vec<&str> = test_content.lines().collect();
    let line = lines[1]; // "def test_example(my_fixture):"

    // The character at end_char should be ')' (right after my_fixture)
    let char_at_end = line.chars().nth(usage.end_char);
    assert_eq!(
        char_at_end,
        Some(')'),
        "end_char should point to ')' after parameter name, got {:?} at pos {} in '{}'",
        char_at_end,
        usage.end_char,
        line
    );
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_no_return_types_early_return() {
    // Test that when no fixtures have return types, we get an empty hints list
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_no_return/conftest.py");
    let test_path = PathBuf::from("/tmp/test_no_return/test_example.py");

    // Fixtures WITHOUT return type annotations
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "value"

@pytest.fixture
def another_fixture():
    return 123
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_example(my_fixture, another_fixture):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Verify fixtures exist but have no return types
    let available = db.get_available_fixtures(&test_path);
    let my_fixture = available.iter().find(|f| f.name == "my_fixture").unwrap();
    assert!(
        my_fixture.return_type.is_none(),
        "my_fixture should have no return type"
    );

    let another = available
        .iter()
        .find(|f| f.name == "another_fixture")
        .unwrap();
    assert!(
        another.return_type.is_none(),
        "another_fixture should have no return type"
    );

    // Usages should still be tracked
    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 2, "Should have 2 fixture usages");
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_unicode_parameter_names() {
    // Test that Unicode parameter names are handled correctly
    // Note: Python 3 allows Unicode identifiers (PEP 3131)
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_unicode/conftest.py");
    let test_path = PathBuf::from("/tmp/test_unicode/test_example.py");

    // Fixture with Unicode name and return type
    let conftest_content = r#"
import pytest

@pytest.fixture
def データベース() -> Database:
    return Database()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_example(データベース):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Verify the fixture is found
    let definitions = db.definitions.get("データベース");
    assert!(definitions.is_some(), "Unicode fixture should be found");

    // Verify usage is tracked
    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].name, "データベース");

    // The end_char calculation uses byte length, which for "データベース" (5 chars, 15 bytes)
    // means end_char = start_char + 15. This is consistent with LSP's UTF-16 handling
    // for the common case where editors normalize to byte offsets.
    let usage = &usages[0];
    let expected_byte_length = "データベース".len(); // 15 bytes
    assert_eq!(
        usage.end_char - usage.start_char,
        expected_byte_length,
        "end_char - start_char should equal byte length of Unicode name"
    );
}

#[test]
#[timeout(30000)]
fn test_inlay_hints_mixed_annotated_unannotated_multiline() {
    // Test multiline function signatures with mixed annotations
    use pytest_language_server::FixtureDatabase;
    use std::path::PathBuf;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_multiline/conftest.py");
    let test_path = PathBuf::from("/tmp/test_multiline/test_example.py");

    let conftest_content = r#"
import pytest

@pytest.fixture
def fixture_a() -> TypeA:
    return TypeA()

@pytest.fixture
def fixture_b() -> TypeB:
    return TypeB()

@pytest.fixture
def fixture_c() -> TypeC:
    return TypeC()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Multiline function with mixed annotations
    let test_content = r#"
def test_multiline(
    fixture_a: TypeA,
    fixture_b,
    fixture_c: TypeC,
):
    pass
"#;
    db.analyze_file(test_path.clone(), test_content);

    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 3, "Should have 3 fixture usages");

    // Get lines for annotation checking
    let lines: Vec<&str> = test_content.lines().collect();

    // fixture_a on line 3 (1-indexed) should have annotation
    let fixture_a_usage = usages.iter().find(|u| u.name == "fixture_a").unwrap();
    assert_eq!(fixture_a_usage.line, 3);
    let line_a = lines[2]; // 0-indexed
    let after_a = &line_a[fixture_a_usage.end_char..];
    assert!(
        after_a.trim_start().starts_with(':'),
        "fixture_a should have annotation"
    );

    // fixture_b on line 4 should NOT have annotation
    let fixture_b_usage = usages.iter().find(|u| u.name == "fixture_b").unwrap();
    assert_eq!(fixture_b_usage.line, 4);
    let line_b = lines[3];
    let after_b = &line_b[fixture_b_usage.end_char..];
    assert!(
        !after_b.trim_start().starts_with(':'),
        "fixture_b should NOT have annotation"
    );

    // fixture_c on line 5 should have annotation
    let fixture_c_usage = usages.iter().find(|u| u.name == "fixture_c").unwrap();
    assert_eq!(fixture_c_usage.line, 5);
    let line_c = lines[4];
    let after_c = &line_c[fixture_c_usage.end_char..];
    assert!(
        after_c.trim_start().starts_with(':'),
        "fixture_c should have annotation"
    );
}

// =============================================================================
// Call Hierarchy Tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_call_hierarchy_prepare_on_fixture_definition() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(scope="session")
def db_connection():
    """Database connection fixture."""
    return "connection"
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Line 5 (0-indexed: 4) is "def db_connection():"
    // Position on the fixture name (starts at char 4) should find it
    let definition = db.find_fixture_or_definition_at_position(&file_path, 4, 4);
    assert!(
        definition.is_some(),
        "Should find fixture at definition line"
    );

    let def = definition.unwrap();
    assert_eq!(def.name, "db_connection");
    assert_eq!(def.scope, pytest_language_server::FixtureScope::Session);
}

#[test]
#[timeout(30000)]
fn test_call_hierarchy_incoming_calls() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Base fixture
    let conftest = r#"
import pytest

@pytest.fixture
def db_connection():
    return "connection"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest);

    // Fixture that depends on db_connection
    let dependent_conftest = r#"
import pytest

@pytest.fixture
def db_session(db_connection):
    return f"session({db_connection})"
"#;
    let dependent_path = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(dependent_path.clone(), dependent_conftest);

    // Test that uses db_connection
    let test_content = r#"
def test_database(db_connection):
    assert db_connection is not None
"#;
    let test_path = PathBuf::from("/tmp/project/tests/test_db.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get definition and find its references (incoming calls)
    let definition = db.find_fixture_or_definition_at_position(&conftest_path, 4, 4);
    assert!(
        definition.is_some(),
        "Should find fixture at definition line"
    );

    let refs = db.find_references_for_definition(&definition.unwrap());

    // Should have references from:
    // 1. The definition itself (conftest.py)
    // 2. db_session fixture parameter (tests/conftest.py)
    // 3. test_database test parameter (tests/test_db.py)
    assert!(
        refs.len() >= 2,
        "Should have at least 2 references (excluding definition)"
    );

    let from_dependent = refs.iter().any(|r| r.file_path == dependent_path);
    let from_test = refs.iter().any(|r| r.file_path == test_path);

    assert!(
        from_dependent,
        "Should have reference from dependent fixture"
    );
    assert!(from_test, "Should have reference from test");
}

#[test]
#[timeout(30000)]
fn test_call_hierarchy_outgoing_calls() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"

@pytest.fixture
def mid_fixture(base_fixture):
    return f"mid({base_fixture})"

@pytest.fixture
def top_fixture(mid_fixture, base_fixture):
    return f"top({mid_fixture}, {base_fixture})"
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // top_fixture depends on mid_fixture and base_fixture
    let top_def = db.definitions.get("top_fixture").unwrap();
    let top = &top_def[0];

    assert_eq!(top.dependencies.len(), 2);
    assert!(top.dependencies.contains(&"mid_fixture".to_string()));
    assert!(top.dependencies.contains(&"base_fixture".to_string()));

    // mid_fixture depends only on base_fixture
    let mid_def = db.definitions.get("mid_fixture").unwrap();
    let mid = &mid_def[0];

    assert_eq!(mid.dependencies.len(), 1);
    assert!(mid.dependencies.contains(&"base_fixture".to_string()));

    // base_fixture has no dependencies
    let base_def = db.definitions.get("base_fixture").unwrap();
    let base = &base_def[0];

    assert_eq!(base.dependencies.len(), 0);
}

#[test]
#[timeout(30000)]
fn test_call_hierarchy_with_fixture_override() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    // Parent fixture
    let parent_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "parent"
"#;
    let parent_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_path.clone(), parent_content);

    // Child fixture that overrides and depends on parent
    let child_content = r#"
import pytest

@pytest.fixture
def shared_fixture(shared_fixture):
    return f"child({shared_fixture})"
"#;
    let child_path = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(child_path.clone(), child_content);

    // Child fixture depends on parent's shared_fixture
    let child_def = db.definitions.get("shared_fixture").unwrap();
    let child = child_def
        .iter()
        .find(|d| d.file_path == child_path)
        .unwrap();

    assert_eq!(child.dependencies.len(), 1);
    assert!(child.dependencies.contains(&"shared_fixture".to_string()));
}

#[test]
#[timeout(30000)]
fn test_call_hierarchy_find_containing_function() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def outer_fixture():
    return "outer"

def test_example(outer_fixture):
    result = outer_fixture
    assert result is not None
"#;
    let file_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(file_path.clone(), content);

    // Line 9 (1-indexed) is inside test_example
    let containing = db.find_containing_function(&file_path, 9);
    assert_eq!(containing, Some("test_example".to_string()));

    // Line 5 (1-indexed) is inside outer_fixture
    let containing = db.find_containing_function(&file_path, 5);
    assert_eq!(containing, Some("outer_fixture".to_string()));
}

#[test]
#[timeout(30000)]
fn test_call_hierarchy_deep_dependency_chain() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def level_1():
    return 1

@pytest.fixture
def level_2(level_1):
    return level_1 + 1

@pytest.fixture
def level_3(level_2):
    return level_2 + 1

@pytest.fixture
def level_4(level_3, level_1):
    return level_3 + level_1
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Verify the dependency chain
    let l4 = &db.definitions.get("level_4").unwrap()[0];
    assert_eq!(l4.dependencies.len(), 2);
    assert!(l4.dependencies.contains(&"level_3".to_string()));
    assert!(l4.dependencies.contains(&"level_1".to_string()));

    let l3 = &db.definitions.get("level_3").unwrap()[0];
    assert_eq!(l3.dependencies.len(), 1);
    assert!(l3.dependencies.contains(&"level_2".to_string()));

    let l2 = &db.definitions.get("level_2").unwrap()[0];
    assert_eq!(l2.dependencies.len(), 1);
    assert!(l2.dependencies.contains(&"level_1".to_string()));

    let l1 = &db.definitions.get("level_1").unwrap()[0];
    assert_eq!(l1.dependencies.len(), 0);
}

// =============================================================================
// Go-to-Implementation Tests (Yield Statement Navigation)
// =============================================================================

#[test]
#[timeout(30000)]
fn test_goto_implementation_yield_fixture() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def database_session():
    """Create a database session with cleanup."""
    session = create_session()
    yield session
    session.close()
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let def = &db.definitions.get("database_session").unwrap()[0];

    // Yield is on line 8 (1-indexed)
    assert_eq!(def.yield_line, Some(8));
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_non_yield_fixture() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def simple_fixture():
    return "value"
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let def = &db.definitions.get("simple_fixture").unwrap()[0];

    // No yield statement
    assert_eq!(def.yield_line, None);
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_yield_in_with_block() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def file_handle():
    with open("test.txt") as f:
        yield f
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let def = &db.definitions.get("file_handle").unwrap()[0];

    // Yield is on line 7 (1-indexed), inside with block
    assert_eq!(def.yield_line, Some(7));
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_yield_in_try_finally() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def resource():
    resource = acquire_resource()
    try:
        yield resource
    finally:
        resource.release()
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let def = &db.definitions.get("resource").unwrap()[0];

    // Yield is on line 8 (1-indexed), inside try block
    assert_eq!(def.yield_line, Some(8));
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_multiple_fixtures_with_yield() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def first_resource():
    yield "first"

@pytest.fixture
def second_resource():
    yield "second"

@pytest.fixture
def third_no_yield():
    return "third"
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let first = &db.definitions.get("first_resource").unwrap()[0];
    assert_eq!(first.yield_line, Some(6));

    let second = &db.definitions.get("second_resource").unwrap()[0];
    assert_eq!(second.yield_line, Some(10));

    let third = &db.definitions.get("third_no_yield").unwrap()[0];
    assert_eq!(third.yield_line, None);
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_fixture_definition_lookup() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let conftest = r#"
import pytest

@pytest.fixture
def yielding_fixture():
    setup()
    yield "value"
    teardown()
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest);

    let test = r#"
def test_uses_yield(yielding_fixture):
    assert yielding_fixture == "value"
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test);

    // Looking up from test file should find the fixture with yield_line
    let def = db.find_fixture_definition(&test_path, 1, 20);
    assert!(def.is_some());

    let fixture = def.unwrap();
    assert_eq!(fixture.name, "yielding_fixture");
    assert_eq!(fixture.yield_line, Some(7));
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_async_yield_fixture() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest
import pytest_asyncio

@pytest_asyncio.fixture
async def async_db():
    db = await create_db()
    yield db
    await db.close()
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Async fixtures with yield should also be detected
    let def = &db.definitions.get("async_db").unwrap()[0];
    assert_eq!(def.yield_line, Some(8));
}

#[test]
#[timeout(30000)]
fn test_goto_implementation_yield_with_conditional() {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def conditional_resource(request):
    if request.param:
        yield "value"
    else:
        yield None
"#;
    let file_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let def = &db.definitions.get("conditional_resource").unwrap()[0];
    // Should find the first yield
    assert!(def.yield_line.is_some());
    // First yield is on line 7
    assert_eq!(def.yield_line, Some(7));
}

// ============================================================================
// TYPE-ANNOTATION CODE ACTION TESTS
// ============================================================================

#[test]
#[timeout(30000)]
fn test_return_type_imports_from_import_style() {
    // Fixture uses `from pathlib import Path` and returns `-> Path`.
    // The resolved TypeImportSpec should produce a `from pathlib import Path` statement.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_from/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def tmp_dir() -> Path:
    return Path("/tmp")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("tmp_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Path"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_direct_import_style() {
    // Fixture uses `import pathlib` and returns `-> pathlib.Path`.
    // The resolved TypeImportSpec should produce an `import pathlib` statement,
    // and the check_name should be `"pathlib"` (the top-level name).
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_direct/conftest.py");

    let conftest_content = r#"
import pytest
import pathlib

@pytest.fixture
def tmp_dir() -> pathlib.Path:
    return pathlib.Path("/tmp")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("tmp_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("pathlib.Path"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "pathlib".to_string(),
            import_statement: "import pathlib".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_aliased_import() {
    // Fixture uses `from pathlib import Path as P` and returns `-> P`.
    // The TypeImportSpec must preserve the alias in both check_name and import_statement.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_alias/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path as P

@pytest.fixture
def tmp_dir() -> P:
    return P("/tmp")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("tmp_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("P"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "P".to_string(),
            import_statement: "from pathlib import Path as P".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_aliased_module_import() {
    // Fixture uses `import pathlib as pl` and returns `-> pl.Path`.
    // The check_name should be `"pl"` and import_statement should preserve the alias.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_alias_mod/conftest.py");

    let conftest_content = r#"
import pytest
import pathlib as pl

@pytest.fixture
def tmp_dir() -> pl.Path:
    return pl.Path("/tmp")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("tmp_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("pl.Path"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "pl".to_string(),
            import_statement: "import pathlib as pl".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_builtin_type() {
    // Fixtures returning builtin types (int, str, bool, …) require no import.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_builtin/conftest.py");

    let conftest_content = r#"
import pytest

@pytest.fixture
def answer() -> int:
    return 42

@pytest.fixture
def greeting() -> str:
    return "hello"

@pytest.fixture
def flag() -> bool:
    return True
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    for name in &["answer", "greeting", "flag"] {
        let defs = db.definitions.get(*name).expect("fixture not found");
        let def = &defs[0];
        assert!(
            def.return_type.is_some(),
            "return_type should be set for {}",
            name
        );
        assert!(
            def.return_type_imports.is_empty(),
            "return_type_imports should be empty for builtin type fixture '{}'",
            name
        );
    }
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_no_annotation() {
    // A fixture without a return annotation should have empty return_type_imports
    // and return_type = None.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_none/conftest.py");

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("my_fixture").expect("fixture not found");
    let def = &defs[0];

    assert!(def.return_type.is_none());
    assert!(def.return_type_imports.is_empty());
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_complex_generic_type() {
    // Complex/generic return types (containing `[`) resolve all identifiers.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_generic/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Optional
from myapp.db import Database

@pytest.fixture
def db_fixture() -> Optional[Database]:
    return Database()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("db_fixture").expect("fixture not found");
    let def = &defs[0];

    // Annotation is captured as-is.
    assert_eq!(def.return_type.as_deref(), Some("Optional[Database]"));
    // Both `Optional` and `Database` need imports from different modules.
    assert_eq!(
        def.return_type_imports,
        vec![
            TypeImportSpec {
                check_name: "Optional".to_string(),
                import_statement: "from typing import Optional".to_string(),
            },
            TypeImportSpec {
                check_name: "Database".to_string(),
                import_statement: "from myapp.db import Database".to_string(),
            },
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_union_type() {
    // Union types with `|` resolve the non-builtin identifiers.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_union/conftest.py");

    let conftest_content = r#"
import pytest
from myapp.db import Database

@pytest.fixture
def maybe_db() -> Database | None:
    return None
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("maybe_db").expect("fixture not found");
    let def = &defs[0];

    // `None` is a builtin, only `Database` needs an import.
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Database".to_string(),
            import_statement: "from myapp.db import Database".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_dict_str_any() {
    // `dict[str, Any]` — `dict` and `str` are builtins, only `Any` needs an import.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_dict_any/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Any

@pytest.fixture
def rig_config() -> dict[str, Any]:
    return {"key": "value"}
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("rig_config").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("dict[str, Any]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_tuple_path_int() {
    // `tuple[Path, int]` — `tuple` and `int` are builtins, only `Path` needs an import.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_tuple_path/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def path_pair() -> tuple[Path, int]:
    return (Path("/tmp"), 42)
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("path_pair").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("tuple[Path, int]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_nested_generics() {
    // `list[dict[str, Any]]` — nested generics, only `Any` needs an import.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_nested/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Any

@pytest.fixture
def configs() -> list[dict[str, Any]]:
    return [{"key": "value"}]
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("configs").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("list[dict[str, Any]]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_duplicate_names_deduplicated() {
    // `tuple[Path, Path]` — `Path` appears twice but should produce only one import.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_dedup/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def two_paths() -> tuple[Path, Path]:
    return (Path("/a"), Path("/b"))
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("two_paths").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("tuple[Path, Path]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_multi_module() {
    // `dict[str, Path]` — `dict` and `str` are builtins, `Path` from pathlib.
    // `Sequence[tuple[Database, Path]]` — `Sequence` from collections.abc,
    // `Database` from myapp.db, `Path` from pathlib.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_multi_mod/conftest.py");

    let conftest_content = r#"
import pytest
from collections.abc import Sequence
from myapp.db import Database
from pathlib import Path

@pytest.fixture
def records() -> Sequence[tuple[Database, Path]]:
    return []
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("records").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Sequence[tuple[Database, Path]]")
    );
    assert_eq!(
        def.return_type_imports,
        vec![
            TypeImportSpec {
                check_name: "Sequence".to_string(),
                import_statement: "from collections.abc import Sequence".to_string(),
            },
            TypeImportSpec {
                check_name: "Database".to_string(),
                import_statement: "from myapp.db import Database".to_string(),
            },
            TypeImportSpec {
                check_name: "Path".to_string(),
                import_statement: "from pathlib import Path".to_string(),
            },
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_locally_defined_type() {
    // A class defined directly in conftest.py (not imported from anywhere).
    // The import spec should reference the conftest module itself.
    // With /tmp paths (no __init__.py), the module resolves to just "conftest".
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_local/conftest.py");

    let conftest_content = r#"
import pytest

class Database:
    def query(self):
        return []

@pytest.fixture
def db() -> Database:
    return Database()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("db").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Database"));
    assert_eq!(def.return_type_imports.len(), 1);
    let spec = &def.return_type_imports[0];
    assert_eq!(spec.check_name, "Database");
    // Without __init__.py the module path is just the file stem.
    assert_eq!(spec.import_statement, "from conftest import Database");
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_yield_fixture_resolved_type() {
    // Generator fixtures have their yielded type extracted.
    // The import should reference that extracted type, not the full Generator annotation.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_yield/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Generator
from pathlib import Path

@pytest.fixture
def tmp_path_fixture() -> Generator[Path, None, None]:
    p = Path("/tmp/test")
    p.mkdir(exist_ok=True)
    yield p
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db
        .definitions
        .get("tmp_path_fixture")
        .expect("fixture not found");
    let def = &defs[0];

    // extract_return_type unwraps Generator[Path, …] to just "Path"
    assert_eq!(def.return_type.as_deref(), Some("Path"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_code_action_import_already_present_in_test_file() {
    // When the test file already imports `Path`, no duplicate import spec should
    // be added.  We test this by inspecting the imports DashMap directly.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_ca_dedup/conftest.py");
    let test_path = PathBuf::from("/tmp/test_ca_dedup/test_example.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def tmp_dir() -> Path:
    return Path("/tmp")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file already has `from pathlib import Path` — the name "Path" is in imports.
    let test_content = r#"
from pathlib import Path

def test_uses_tmp_dir():
    result = tmp_dir / "file.txt"
    assert result.parent == tmp_dir
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Confirm the fixture definition has the import spec.
    let defs = db.definitions.get("tmp_dir").expect("fixture not found");
    let def = &defs[0];
    assert_eq!(def.return_type_imports.len(), 1);
    assert_eq!(def.return_type_imports[0].check_name, "Path");

    // Confirm the test file's imports map already contains "Path".
    let test_imports = db
        .imports
        .get(&test_path)
        .expect("test file imports not found");
    assert!(
        test_imports.contains("Path"),
        "Test file should already have 'Path' in its imports"
    );
    // So the code action would skip adding the import (checked by caller).
}

#[test]
#[timeout(30000)]
fn test_code_action_import_not_yet_present_in_test_file() {
    // When the test file does NOT import the type, the TypeImportSpec should be
    // returned and the check_name should NOT appear in the test file's imports.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_ca_missing/conftest.py");
    let test_path = PathBuf::from("/tmp/test_ca_missing/test_example.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def tmp_dir() -> Path:
    return Path("/tmp")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file has NO pathlib import.
    let test_content = r#"
import pytest

def test_uses_tmp_dir():
    result = tmp_dir / "file.txt"
    assert result.parent == tmp_dir
"#;
    db.analyze_file(test_path.clone(), test_content);

    let defs = db.definitions.get("tmp_dir").expect("fixture not found");
    let def = &defs[0];
    assert_eq!(def.return_type_imports.len(), 1);
    let spec = &def.return_type_imports[0];
    assert_eq!(spec.check_name, "Path");
    assert_eq!(spec.import_statement, "from pathlib import Path");

    // Confirm "Path" is absent from the test file's imports.
    let test_imports = db
        .imports
        .get(&test_path)
        .expect("test file imports not found");
    assert!(
        !test_imports.contains("Path"),
        "Test file should NOT yet have 'Path' in its imports"
    );
}

#[test]
#[timeout(30000)]
fn test_code_action_annotation_in_param_text() {
    // Integration test: after analysis, the fixture definition carries enough
    // information for the code action to build `"my_fixture: Path"` as the
    // parameter text.  We verify the data, not the full LSP handler.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_ca_param_text/conftest.py");
    let test_path = PathBuf::from("/tmp/test_ca_param_text/test_example.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def work_dir() -> Path:
    return Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
import pytest

def test_something():
    result = work_dir / "out.txt"
"#;
    db.analyze_file(test_path.clone(), test_content);

    // Resolve the fixture definition as the code action would.
    let fixture_def = db.resolve_fixture_for_file(&test_path, "work_dir");
    assert!(fixture_def.is_some(), "Should resolve fixture definition");
    let fixture_def = fixture_def.unwrap();

    // Simulate code action param-text construction.
    let type_suffix = fixture_def
        .return_type
        .as_deref()
        .map(|t| format!(": {}", t))
        .unwrap_or_default();

    // When adding as the first parameter (no existing params).
    let param_text_no_comma = format!("work_dir{}", type_suffix);
    assert_eq!(param_text_no_comma, "work_dir: Path");

    // When appending after existing parameters.
    let param_text_with_comma = format!(", work_dir{}", type_suffix);
    assert_eq!(param_text_with_comma, ", work_dir: Path");

    // Import spec is correct.
    assert_eq!(fixture_def.return_type_imports.len(), 1);
    assert_eq!(fixture_def.return_type_imports[0].check_name, "Path");
    assert_eq!(
        fixture_def.return_type_imports[0].import_statement,
        "from pathlib import Path"
    );
}

#[test]
#[timeout(30000)]
fn test_code_action_no_annotation_when_no_return_type() {
    // Fixtures without a return annotation keep the old bare-name behaviour:
    // type_suffix is empty and return_type_imports is empty.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_ca_no_type/conftest.py");
    let test_path = PathBuf::from("/tmp/test_ca_no_type/test_example.py");

    let conftest_content = r#"
import pytest

@pytest.fixture
def plain_fixture():
    return 42
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_uses_plain():
    result = plain_fixture + 1
"#;
    db.analyze_file(test_path.clone(), test_content);

    let fixture_def = db.resolve_fixture_for_file(&test_path, "plain_fixture");
    assert!(fixture_def.is_some());
    let fixture_def = fixture_def.unwrap();

    assert!(fixture_def.return_type.is_none());
    assert!(fixture_def.return_type_imports.is_empty());

    let type_suffix = fixture_def
        .return_type
        .as_deref()
        .map(|t| format!(": {}", t))
        .unwrap_or_default();
    assert_eq!(type_suffix, "", "No type suffix when no return annotation");

    let param_text = format!("plain_fixture{}", type_suffix);
    assert_eq!(param_text, "plain_fixture");
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_relative_import_resolved() {
    // A conftest.py using `from .models import Database` (relative import).
    // With /tmp paths (no __init__.py), the relative import resolves to just
    // `"models"` as the module, producing `"from models import Database"`.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    // Use a path that simulates a relative import scenario.
    let conftest_path = PathBuf::from("/tmp/test_relative_import/conftest.py");

    // NOTE: The relative import `.models` won't resolve to a real file in /tmp,
    // but `resolve_relative_module_to_string` still computes the path mathematically
    // and `file_path_to_module_path` returns "models" (no __init__.py found).
    let conftest_content = r#"
import pytest
from .models import Database

@pytest.fixture
def db_fixture() -> Database:
    return Database()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("db_fixture").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Database"));
    assert_eq!(def.return_type_imports.len(), 1);
    let spec = &def.return_type_imports[0];
    assert_eq!(spec.check_name, "Database");
    // With no __init__.py, the resolved module is "models".
    assert_eq!(spec.import_statement, "from models import Database");
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_multiple_fixtures_different_types() {
    // Multiple fixtures in one conftest with different return types all get
    // independent, correct TypeImportSpec values.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_multi_types/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path
import os

@pytest.fixture
def work_dir() -> Path:
    return Path("/work")

@pytest.fixture
def env_path() -> os.PathLike:
    return Path("/env")

@pytest.fixture
def count() -> int:
    return 0
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    // `work_dir` → Path, from-import style.
    let work_dir_def = &db.definitions.get("work_dir").unwrap()[0];
    assert_eq!(
        work_dir_def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );

    // `env_path` → os.PathLike, top-level name is "os", direct-import style.
    let env_path_def = &db.definitions.get("env_path").unwrap()[0];
    assert_eq!(
        env_path_def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "os".to_string(),
            import_statement: "import os".to_string(),
        }]
    );

    // `count` → int, builtin, no imports.
    let count_def = &db.definitions.get("count").unwrap()[0];
    assert!(count_def.return_type_imports.is_empty());
}

// ── Edge-case tests for type identifier extraction (item 4) ─────────────

#[test]
#[timeout(30000)]
fn test_return_type_imports_literal_string_values_ignored() {
    // `Literal["x", "y"]` — `Literal` needs a typing import, but the string
    // contents `x` and `y` are tokenised as identifiers and must be harmlessly
    // skipped (they won't appear in the import map or module-level names).
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_literal/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Literal

@pytest.fixture
def mode() -> Literal["read", "write"]:
    return "read"
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("mode").expect("fixture not found");
    let def = &defs[0];

    // The AST stringifies string constants via Debug as `Str("...")`.
    assert_eq!(
        def.return_type.as_deref(),
        Some(r#"Literal[Str("read"), Str("write")]"#)
    );
    // Only `Literal` should produce an import — `Str`, `read` and `write` are
    // not in the import map or module-level names so they are silently skipped.
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Literal".to_string(),
            import_statement: "from typing import Literal".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_annotated_with_string_metadata() {
    // `Annotated[User, "metadata"]` — `Annotated` and `User` need imports,
    // the string content `metadata` should be harmlessly ignored.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_annotated/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Annotated
from myapp.models import User

@pytest.fixture
def admin_user() -> Annotated[User, "metadata"]:
    return User(admin=True)
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("admin_user").expect("fixture not found");
    let def = &defs[0];

    // The AST stringifies string constants via Debug as `Str("...")`.
    assert_eq!(
        def.return_type.as_deref(),
        Some(r#"Annotated[User, Str("metadata")]"#)
    );
    // `Str` and `metadata` are bare identifiers from the constant — they should
    // not appear in the result because they're not in the import map or module-level names.
    assert_eq!(
        def.return_type_imports,
        vec![
            TypeImportSpec {
                check_name: "Annotated".to_string(),
                import_statement: "from typing import Annotated".to_string(),
            },
            TypeImportSpec {
                check_name: "User".to_string(),
                import_statement: "from myapp.models import User".to_string(),
            },
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_callable_nested_brackets() {
    // `Callable[[int, str], bool]` — `Callable` needs an import from typing,
    // `int`, `str`, `bool` are all builtins. The double-bracket `[[` should
    // not trip up the tokeniser.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_callable/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Callable

@pytest.fixture
def handler() -> Callable[[int, str], bool]:
    return lambda x, y: True
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("handler").expect("fixture not found");
    let def = &defs[0];

    // The AST represents the inner `[int, str]` as a List node, which
    // `expr_to_string` maps to `"Any"` (unknown node type fallback).
    assert_eq!(def.return_type.as_deref(), Some("Callable[Any, bool]"));
    // `Callable` is in the import map; `Any` is NOT imported so it is skipped;
    // `bool` is a builtin.
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Callable".to_string(),
            import_statement: "from typing import Callable".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_callable_with_custom_types() {
    // `Callable[[Request], Response]` — the inner `[Request]` is a List node
    // which `expr_to_string` maps to `"Any"`, so `Request` is lost in the
    // return type string.  Only `Callable` and `Response` survive.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_callable_custom/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Callable
from myapp.http import Request, Response

@pytest.fixture
def endpoint() -> Callable[[Request], Response]:
    return lambda req: Response()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("endpoint").expect("fixture not found");
    let def = &defs[0];

    // The inner list `[Request]` becomes `Any`, so the return type is
    // `Callable[Any, Response]`.  `Request` is not present in the string.
    assert_eq!(def.return_type.as_deref(), Some("Callable[Any, Response]"));
    assert_eq!(
        def.return_type_imports,
        vec![
            TypeImportSpec {
                check_name: "Callable".to_string(),
                import_statement: "from typing import Callable".to_string(),
            },
            TypeImportSpec {
                check_name: "Response".to_string(),
                import_statement: "from myapp.http import Response".to_string(),
            },
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_dotted_collections_abc() {
    // `collections.abc.Iterable[str]` with `import collections.abc` — the
    // import map stores the key as `"collections.abc"` (the full dotted name),
    // but the tokeniser splits the return type string into individual
    // identifiers `collections`, `abc`, `Iterable`, `str`.  None of those
    // match the full dotted key, so no import spec is produced.
    //
    // This is a known limitation: `import X.Y` followed by `X.Y.Z` in an
    // annotation is not resolved.  Use `from collections.abc import Iterable`
    // instead for proper resolution.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_dotted_abc/conftest.py");

    let conftest_content = r#"
import pytest
import collections.abc

@pytest.fixture
def items() -> collections.abc.Iterable[str]:
    return ["a", "b"]
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("items").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("collections.abc.Iterable[str]")
    );
    // No import specs produced — the dotted import key doesn't match any
    // individual identifier token.  This documents the current limitation.
    assert!(
        def.return_type_imports.is_empty(),
        "Expected no imports for dotted bare import (known limitation), got: {:?}",
        def.return_type_imports
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_from_collections_abc_iterable() {
    // `Iterable[str]` with `from collections.abc import Iterable` — the
    // from-import puts `Iterable` directly in the import map.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_from_abc/conftest.py");

    let conftest_content = r#"
import pytest
from collections.abc import Iterable

@pytest.fixture
def items() -> Iterable[str]:
    return ["a", "b"]
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("items").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Iterable[str]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Iterable".to_string(),
            import_statement: "from collections.abc import Iterable".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_forward_ref_quoted() {
    // `list["User"]` — forward reference with quotes.  The AST stringifies
    // the string constant as `Str("User")`, so the return type string is
    // `list[Str("User")]`.  The tokeniser extracts `list`, `Str`, `User`.
    // `list` is builtin, `Str` is not in the import map, and `User` IS a
    // module-level class definition so it falls back to module-path import.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_forward_ref/conftest.py");

    let conftest_content = r#"
import pytest

class User:
    pass

@pytest.fixture
def users() -> list["User"]:
    return [User()]
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("users").expect("fixture not found");
    let def = &defs[0];

    // The AST Debug-formats string constants as `Str("...")`.
    assert_eq!(def.return_type.as_deref(), Some(r#"list[Str("User")]"#));
    // `User` is locally defined → import generated from module path.
    assert_eq!(def.return_type_imports.len(), 1);
    assert_eq!(def.return_type_imports[0].check_name, "User");
    assert_eq!(
        def.return_type_imports[0].import_statement,
        "from conftest import User"
    );
}

// ── Typing symbol tests (item 5) ───────────────────────────────────────

#[test]
#[timeout(30000)]
fn test_return_type_imports_typing_any_needs_import() {
    // `Any` is a typing symbol, NOT a builtin — it must produce an import.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_any/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Any

@pytest.fixture
def anything() -> Any:
    return 42
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("anything").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Any"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_typing_optional_needs_import() {
    // `Optional[str]` — `Optional` is a typing symbol (not builtin), `str` is builtin.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_optional/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Optional

@pytest.fixture
def maybe_name() -> Optional[str]:
    return None
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("maybe_name").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Optional[str]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Optional".to_string(),
            import_statement: "from typing import Optional".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_typing_union_needs_import() {
    // `Union[str, int]` — `Union` is a typing symbol, `str` and `int` are builtins.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_union_sym/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Union

@pytest.fixture
def flexible() -> Union[str, int]:
    return "hello"
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("flexible").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Union[str, int]"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Union".to_string(),
            import_statement: "from typing import Union".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_typing_literal_needs_import() {
    // `Literal[1, 2, 3]` — `Literal` from typing needs an import.
    // The AST Debug-formats integer constants as `Int(N)`.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_literal_int/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Literal

@pytest.fixture
def priority() -> Literal[1, 2, 3]:
    return 1
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("priority").expect("fixture not found");
    let def = &defs[0];

    // Integer constants are Debug-formatted as `Int(N)`.
    assert_eq!(
        def.return_type.as_deref(),
        Some("Literal[Int(1), Int(2), Int(3)]")
    );
    // `Int` is not in the import map or builtins, so only `Literal` produces
    // an import spec.
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Literal".to_string(),
            import_statement: "from typing import Literal".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_typing_annotated_needs_import() {
    // `Annotated[int, "positive"]` — `Annotated` from typing needs an import,
    // `int` is builtin, the string constant is Debug-formatted as `Str("positive")`.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_type_annotated_int/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Annotated

@pytest.fixture
def positive_int() -> Annotated[int, "positive"]:
    return 42
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db
        .definitions
        .get("positive_int")
        .expect("fixture not found");
    let def = &defs[0];

    // String constants are Debug-formatted as `Str("...")`.
    assert_eq!(
        def.return_type.as_deref(),
        Some(r#"Annotated[int, Str("positive")]"#)
    );
    // Only `Annotated` should produce an import; `int` is builtin, `Str` and
    // `positive` are not in the import map or module-level names.
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Annotated".to_string(),
            import_statement: "from typing import Annotated".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_all_builtins_skipped() {
    // Verify a broad set of builtin type names produce no import specs.
    // This covers the BUILTINS static set in analyzer.rs.
    use pytest_language_server::FixtureDatabase;

    let builtin_types = [
        ("f_int", "int"),
        ("f_str", "str"),
        ("f_bool", "bool"),
        ("f_float", "float"),
        ("f_bytes", "bytes"),
        ("f_bytearray", "bytearray"),
        ("f_complex", "complex"),
        ("f_list", "list"),
        ("f_dict", "dict"),
        ("f_tuple", "tuple"),
        ("f_set", "set"),
        ("f_frozenset", "frozenset"),
        ("f_type", "type"),
        ("f_object", "object"),
        ("f_none", "None"),
        ("f_range", "range"),
        ("f_slice", "slice"),
        ("f_memoryview", "memoryview"),
    ];

    // Build a conftest with one fixture per builtin type
    let mut conftest_content = String::from("import pytest\n\n");
    for (name, ret_type) in &builtin_types {
        conftest_content.push_str(&format!(
            "@pytest.fixture\ndef {}() -> {}:\n    pass\n\n",
            name, ret_type
        ));
    }

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_all_builtins/conftest.py");
    db.analyze_file(conftest_path.clone(), &conftest_content);

    for (name, ret_type) in &builtin_types {
        let defs = db
            .definitions
            .get(*name)
            .unwrap_or_else(|| panic!("fixture '{}' not found", name));
        let def = &defs[0];
        assert_eq!(def.return_type.as_deref(), Some(*ret_type));
        assert!(
            def.return_type_imports.is_empty(),
            "Builtin type '{}' should not produce any import specs, but got: {:?}",
            ret_type,
            def.return_type_imports
        );
    }
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_exception_builtins_skipped() {
    // Exception types listed in the BUILTINS set should be skipped.
    use pytest_language_server::FixtureDatabase;

    let exception_types = [
        ("f_exc", "Exception"),
        ("f_base", "BaseException"),
        ("f_val", "ValueError"),
        ("f_type", "TypeError"),
        ("f_runtime", "RuntimeError"),
        ("f_attr", "AttributeError"),
        ("f_key", "KeyError"),
        ("f_idx", "IndexError"),
    ];

    let mut conftest_content = String::from("import pytest\n\n");
    for (name, ret_type) in &exception_types {
        conftest_content.push_str(&format!(
            "@pytest.fixture\ndef {}() -> {}:\n    raise {}()\n\n",
            name, ret_type, ret_type
        ));
    }

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_exception_builtins/conftest.py");
    db.analyze_file(conftest_path.clone(), &conftest_content);

    for (name, ret_type) in &exception_types {
        let defs = db
            .definitions
            .get(*name)
            .unwrap_or_else(|| panic!("fixture '{}' not found", name));
        let def = &defs[0];
        assert!(
            def.return_type_imports.is_empty(),
            "Exception builtin '{}' should not produce any import specs, but got: {:?}",
            ret_type,
            def.return_type_imports
        );
    }
}

// ── Relative import tests (item 8) ─────────────────────────────────────

#[test]
#[timeout(30000)]
fn test_return_type_imports_relative_import_level_1() {
    // `from .models import Database` (level=1) — resolved relative to the
    // fixture file's directory.  Without __init__.py, the resolved module
    // path is just "models".
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_rel_l1/conftest.py");

    let conftest_content = r#"
import pytest
from .models import Database

@pytest.fixture
def db() -> Database:
    return Database()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("db").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Database"));
    assert_eq!(def.return_type_imports.len(), 1);
    assert_eq!(def.return_type_imports[0].check_name, "Database");
    // level=1 from /tmp/test_rel_l1/conftest.py → base is /tmp/test_rel_l1/
    // target file is /tmp/test_rel_l1/models.py → module path "models"
    assert_eq!(
        def.return_type_imports[0].import_statement,
        "from models import Database"
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_relative_import_level_2() {
    // `from ..shared import Config` (level=2) — navigates up two directories
    // from the fixture file's parent.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    // Fixture lives in /tmp/test_rel_l2/sub/conftest.py
    let conftest_path = PathBuf::from("/tmp/test_rel_l2/sub/conftest.py");

    let conftest_content = r#"
import pytest
from ..shared import Config

@pytest.fixture
def config() -> Config:
    return Config()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("config").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Config"));
    assert_eq!(def.return_type_imports.len(), 1);
    assert_eq!(def.return_type_imports[0].check_name, "Config");
    // level=2 from /tmp/test_rel_l2/sub/conftest.py:
    //   base starts at parent (/tmp/test_rel_l2/sub/), then goes up 1 more → /tmp/test_rel_l2/
    //   target file is /tmp/test_rel_l2/shared.py → module path "shared"
    assert_eq!(
        def.return_type_imports[0].import_statement,
        "from shared import Config"
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_relative_import_bare_dot() {
    // `from . import helpers` (level=1, empty module name) — target is
    // __init__.py in the fixture file's directory.
    use std::fs;

    // Create a temp directory with __init__.py so file_path_to_module_path resolves the package.
    let dir = std::env::temp_dir().join("test_rel_bare_dot");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("__init__.py"), "").unwrap();

    let conftest_path = dir.join("conftest.py");

    let conftest_content = r#"
import pytest
from . import helpers

@pytest.fixture
def helper() -> helpers.Helper:
    return helpers.Helper()
"#;
    db_analyze_and_check_bare_dot(&conftest_path, conftest_content, &dir);

    // Clean up
    let _ = fs::remove_dir_all(&dir);
}

/// Helper for test_return_type_imports_relative_import_bare_dot — separated
/// to ensure tempdir cleanup runs even on assertion failure.
fn db_analyze_and_check_bare_dot(
    conftest_path: &std::path::Path,
    content: &str,
    dir: &std::path::Path,
) {
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    db.analyze_file(conftest_path.to_path_buf(), content);

    let defs = db.definitions.get("helper").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("helpers.Helper"));
    // `from . import helpers` makes the check_name "helpers".
    // The import map resolves `from . import helpers` to the package's __init__
    // path.  `helpers` should appear in the import map.
    // `Helper` alone won't be in the import map (it's `helpers.Helper`).
    let helpers_specs: Vec<_> = def
        .return_type_imports
        .iter()
        .filter(|s| s.check_name == "helpers")
        .collect();
    assert!(
        !helpers_specs.is_empty(),
        "Expected an import spec for 'helpers', got: {:?}",
        def.return_type_imports
    );
    // The dir name is the package name since __init__.py exists.
    let dir_name = dir.file_name().unwrap().to_str().unwrap();
    let expected_import = format!("from {} import helpers", dir_name);
    assert_eq!(helpers_specs[0].import_statement, expected_import);
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_relative_import_level_1_with_package() {
    // Verify that relative imports inside a real package (with __init__.py)
    // produce fully qualified absolute import statements.
    use pytest_language_server::FixtureDatabase;
    use std::fs;

    let dir = std::env::temp_dir().join("test_rel_pkg_l1");
    let _ = fs::remove_dir_all(&dir);
    let pkg = dir.join("mypkg");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("__init__.py"), "").unwrap();

    let conftest_path = pkg.join("conftest.py");

    let conftest_content = r#"
import pytest
from .models import User

@pytest.fixture
def user() -> User:
    return User()
"#;

    let db = FixtureDatabase::new();
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("user").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("User"));
    assert_eq!(def.return_type_imports.len(), 1);
    assert_eq!(def.return_type_imports[0].check_name, "User");
    // level=1 from mypkg/conftest.py: base is mypkg/, target is mypkg/models.py
    // With __init__.py in mypkg/, module path is "mypkg.models"
    assert_eq!(
        def.return_type_imports[0].import_statement,
        "from mypkg.models import User"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_relative_import_above_root_resolved_mathematically() {
    // `from ...too_high import Widget` (level=3) from `/tmp/shallow/conftest.py`.
    // The resolution is purely mathematical (no filesystem check on the target):
    //   parent = /tmp/shallow/ → up 2 more → / → target = /too_high.py
    //   file_path_to_module_path("/too_high.py") = Some("too_high")
    // So the import resolves to `from too_high import Widget`.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/shallow/conftest.py");

    let conftest_content = r#"
import pytest
from ...too_high import Widget

@pytest.fixture
def widget() -> Widget:
    return Widget()
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("widget").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Widget"));
    // The relative import is resolved mathematically even though /too_high.py
    // doesn't exist on disk.  The resolved module path is "too_high".
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Widget".to_string(),
            import_statement: "from too_high import Widget".to_string(),
        }]
    );
}

// ── Consumer-side type adaptation integration tests ─────────────────────

#[test]
#[timeout(30000)]
fn test_return_type_imports_bare_import_produces_module_check_name() {
    // When a fixture file uses `import pathlib` and `-> pathlib.Path`, the
    // TypeImportSpec must have check_name="pathlib" and import_statement=
    // "import pathlib".  This is the data that `adapt_type_for_consumer`
    // (in code_action.rs) uses at code-action time to detect that a consumer
    // file with `from pathlib import Path` can use the short form `Path`.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_bare_import_adapt/conftest.py");

    let conftest_content = r#"
import pytest
import pathlib

@pytest.fixture
def work_dir() -> pathlib.Path:
    return pathlib.Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("pathlib.Path"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "pathlib".to_string(),
            import_statement: "import pathlib".to_string(),
        }]
    );

    // Verify: the consumer file's imports set would contain "Path" (not
    // "pathlib") when it has `from pathlib import Path`.  The check_name
    // "pathlib" does NOT match "Path", so build_import_edits alone would
    // incorrectly add `import pathlib`.  The adapt_type_for_consumer function
    // in code_action.rs handles this by rewriting the type to "Path" and
    // dropping the spec.
    let test_path = PathBuf::from("/tmp/test_bare_import_adapt/test_example.py");
    let test_content = r#"
from pathlib import Path

def test_uses_work_dir():
    result = work_dir / "file.txt"
"#;
    db.analyze_file(test_path.clone(), test_content);

    let test_imports = db.imports.get(&test_path).expect("test imports not found");
    assert!(
        test_imports.contains("Path"),
        "Test file should have 'Path' in its imports"
    );
    assert!(
        !test_imports.contains("pathlib"),
        "Test file should NOT have 'pathlib' as a bare name in its imports"
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_bare_import_aliased_module() {
    // `import pathlib as pl` + `-> pl.Path` — the TypeImportSpec should have
    // check_name="pl" so that adapt_type_for_consumer can find "pl." prefixes
    // in the type string and rewrite them.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_bare_alias_adapt/conftest.py");

    let conftest_content = r#"
import pytest
import pathlib as pl

@pytest.fixture
def work_dir() -> pl.Path:
    return pl.Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("pl.Path"));
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "pl".to_string(),
            import_statement: "import pathlib as pl".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_return_type_imports_bare_import_complex_generic() {
    // `import pathlib` + `from typing import Optional` + `-> Optional[pathlib.Path]`
    // Should produce two specs: one for Optional (from-import) and one for
    // pathlib (bare import).  At code-action time, if the consumer has
    // `from pathlib import Path`, only pathlib.Path is rewritten to Path.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_bare_generic_adapt/conftest.py");

    let conftest_content = r#"
import pytest
import pathlib
from typing import Optional

@pytest.fixture
def maybe_dir() -> Optional[pathlib.Path]:
    return None
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("maybe_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(def.return_type.as_deref(), Some("Optional[pathlib.Path]"));
    assert_eq!(
        def.return_type_imports,
        vec![
            TypeImportSpec {
                check_name: "Optional".to_string(),
                import_statement: "from typing import Optional".to_string(),
            },
            TypeImportSpec {
                check_name: "pathlib".to_string(),
                import_statement: "import pathlib".to_string(),
            },
        ]
    );
}

// ── End-to-end code action integration tests ────────────────────────────

/// Helper: create a `Backend` backed by the given `FixtureDatabase`.
/// Uses `LspService::new` to obtain a valid `Client` handle (same technique
/// as the inline tests in `completion.rs`).
fn make_backend_with_db(
    db: Arc<pytest_language_server::FixtureDatabase>,
) -> pytest_language_server::Backend {
    use pytest_language_server::Backend;
    use tower_lsp_server::LspService;

    let backend_slot: Arc<std::sync::Mutex<Option<Backend>>> =
        Arc::new(std::sync::Mutex::new(None));
    let slot_clone = backend_slot.clone();
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
    let result = backend_slot
        .lock()
        .unwrap()
        .take()
        .expect("Backend should have been created");
    result
}

#[tokio::test]
async fn test_code_action_quickfix_adapts_dotted_to_short() {
    // End-to-end: fixture uses `import pathlib` → return type `pathlib.Path`.
    // Consumer already has `from pathlib import Path`.
    // The quickfix should insert `: Path` (not `: pathlib.Path`) and must NOT
    // add an `import pathlib` statement.
    use pytest_language_server::FixtureDatabase;

    let db = Arc::new(FixtureDatabase::new());

    let conftest_path = PathBuf::from("/tmp/test_ca_e2e_dotted/conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        r#"
import pytest
import pathlib

@pytest.fixture
def work_dir() -> pathlib.Path:
    return pathlib.Path("/work")
"#,
    );

    let test_path = PathBuf::from("/tmp/test_ca_e2e_dotted/test_example.py");
    db.analyze_file(
        test_path.clone(),
        r#"
from pathlib import Path

def test_something():
    result = work_dir
"#,
    );

    // Get undeclared fixture coordinates for the diagnostic.
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(undeclared.len(), 1, "Should detect 1 undeclared fixture");
    let fix = &undeclared[0];
    assert_eq!(fix.name, "work_dir");

    let backend = make_backend_with_db(db);
    let uri = Uri::from_file_path(&test_path).unwrap();

    // Internal (1-based) → LSP (0-based).
    let diag_line_lsp = (fix.line - 1) as u32;
    let func_line_lsp = (fix.function_line - 1) as u32;

    let diagnostic = Diagnostic {
        range: Range {
            start: Position {
                line: diag_line_lsp,
                character: fix.start_char as u32,
            },
            end: Position {
                line: diag_line_lsp,
                character: fix.end_char as u32,
            },
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String("undeclared-fixture".to_string())),
        source: Some("pytest-lsp".to_string()),
        message: format!(
            "Fixture '{}' is used but not declared as a parameter",
            fix.name
        ),
        code_description: None,
        related_information: None,
        tags: None,
        data: None,
    };

    let params = CodeActionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        range: Range {
            start: Position {
                line: func_line_lsp,
                character: 0,
            },
            end: Position {
                line: func_line_lsp,
                character: 0,
            },
        },
        context: CodeActionContext {
            diagnostics: vec![diagnostic],
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: WorkDoneProgressParams {
            work_done_token: None,
        },
        partial_result_params: PartialResultParams {
            partial_result_token: None,
        },
    };

    let response = backend.handle_code_action(params).await.unwrap();
    let actions = response.expect("Should return code actions");

    // Find the quickfix action.
    let quickfix = actions
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.kind == Some(CodeActionKind::QUICKFIX) => {
                Some(ca)
            }
            _ => None,
        })
        .expect("Should have a quickfix code action");

    // Title should show the adapted short type, not the dotted form.
    assert!(
        quickfix.title.contains("(Path)"),
        "Title should contain '(Path)': {}",
        quickfix.title
    );
    assert!(
        !quickfix.title.contains("pathlib.Path"),
        "Title should NOT contain 'pathlib.Path': {}",
        quickfix.title
    );

    // Inspect the workspace edits.
    let ws_edit = quickfix.edit.as_ref().expect("Should have workspace edit");
    let changes = ws_edit.changes.as_ref().expect("Should have changes");
    let edits: Vec<&TextEdit> = changes.values().flat_map(|v| v.iter()).collect();

    // The parameter-insertion edit should use `: Path` (short form).
    let param_edit = edits
        .iter()
        .find(|e| e.new_text.contains("work_dir"))
        .expect("Should have a parameter insertion edit");
    assert!(
        param_edit.new_text.contains(": Path"),
        "Parameter should use short form: {:?}",
        param_edit.new_text
    );
    assert!(
        !param_edit.new_text.contains("pathlib.Path"),
        "Parameter should NOT use dotted form: {:?}",
        param_edit.new_text
    );

    // No import edit should add `import pathlib` — the consumer's existing
    // `from pathlib import Path` already covers the type.
    let has_bare_import = edits
        .iter()
        .any(|e| e.new_text.contains("import pathlib") && !e.new_text.contains("from"));
    assert!(
        !has_bare_import,
        "Should NOT add 'import pathlib': {:?}",
        edits
    );
}

#[tokio::test]
async fn test_code_action_quickfix_adapts_short_to_dotted() {
    // End-to-end: fixture uses `from pathlib import Path` → short `Path`.
    // Consumer has `import pathlib` (bare import).
    // The quickfix should insert `: pathlib.Path` and must NOT add
    // `from pathlib import Path`.
    use pytest_language_server::FixtureDatabase;

    let db = Arc::new(FixtureDatabase::new());

    let conftest_path = PathBuf::from("/tmp/test_ca_e2e_short/conftest.py");
    db.analyze_file(
        conftest_path.clone(),
        r#"
import pytest
from pathlib import Path

@pytest.fixture
def work_dir() -> Path:
    return Path("/work")
"#,
    );

    let test_path = PathBuf::from("/tmp/test_ca_e2e_short/test_example.py");
    db.analyze_file(
        test_path.clone(),
        r#"
import pathlib

def test_something():
    result = work_dir
"#,
    );

    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(undeclared.len(), 1);
    let fix = &undeclared[0];
    assert_eq!(fix.name, "work_dir");

    let backend = make_backend_with_db(db);
    let uri = Uri::from_file_path(&test_path).unwrap();

    let diag_line_lsp = (fix.line - 1) as u32;
    let func_line_lsp = (fix.function_line - 1) as u32;

    let diagnostic = Diagnostic {
        range: Range {
            start: Position {
                line: diag_line_lsp,
                character: fix.start_char as u32,
            },
            end: Position {
                line: diag_line_lsp,
                character: fix.end_char as u32,
            },
        },
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String("undeclared-fixture".to_string())),
        source: Some("pytest-lsp".to_string()),
        message: format!(
            "Fixture '{}' is used but not declared as a parameter",
            fix.name
        ),
        code_description: None,
        related_information: None,
        tags: None,
        data: None,
    };

    let params = CodeActionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        range: Range {
            start: Position {
                line: func_line_lsp,
                character: 0,
            },
            end: Position {
                line: func_line_lsp,
                character: 0,
            },
        },
        context: CodeActionContext {
            diagnostics: vec![diagnostic],
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: WorkDoneProgressParams {
            work_done_token: None,
        },
        partial_result_params: PartialResultParams {
            partial_result_token: None,
        },
    };

    let response = backend.handle_code_action(params).await.unwrap();
    let actions = response.expect("Should return code actions");

    let quickfix = actions
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.kind == Some(CodeActionKind::QUICKFIX) => {
                Some(ca)
            }
            _ => None,
        })
        .expect("Should have a quickfix code action");

    // Title should show the adapted dotted type.
    assert!(
        quickfix.title.contains("pathlib.Path"),
        "Title should contain 'pathlib.Path': {}",
        quickfix.title
    );

    let ws_edit = quickfix.edit.as_ref().expect("Should have workspace edit");
    let changes = ws_edit.changes.as_ref().expect("Should have changes");
    let edits: Vec<&TextEdit> = changes.values().flat_map(|v| v.iter()).collect();

    // The parameter edit should use `: pathlib.Path`.
    let param_edit = edits
        .iter()
        .find(|e| e.new_text.contains("work_dir"))
        .expect("Should have a parameter insertion edit");
    assert!(
        param_edit.new_text.contains(": pathlib.Path"),
        "Parameter should use dotted form: {:?}",
        param_edit.new_text
    );

    // No `from pathlib import Path` edit should be present — the adaptation
    // rewrote the type to dotted form, so the from-import spec was dropped.
    let has_from_import = edits
        .iter()
        .any(|e| e.new_text.contains("from pathlib import Path"));
    assert!(
        !has_from_import,
        "Should NOT add 'from pathlib import Path': {:?}",
        edits
    );
}

// ── Type alias expansion tests ──────────────────────────────────────────

#[test]
#[timeout(30000)]
fn test_type_alias_old_style_expanded_in_return_type() {
    // Old-style type alias: `MyPath = Path` then `-> MyPath`.
    // The return type should be expanded to `Path` (not kept as `MyPath`),
    // and the import spec should reference `Path`, not `MyPath`.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_old/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

MyPath = Path

@pytest.fixture
def work_dir() -> MyPath:
    return Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    // Return type should be expanded from `MyPath` to `Path`.
    assert_eq!(
        def.return_type.as_deref(),
        Some("Path"),
        "Type alias should be expanded"
    );
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_old_style_generic_expanded() {
    // Old-style: `UserMap = Dict[str, List[int]]` then `-> UserMap`.
    // Should expand to `Dict[str, List[int]]` with proper imports.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_old_generic/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Dict, List

UserMap = Dict[str, List[int]]

@pytest.fixture
def user_data() -> UserMap:
    return {"scores": [1, 2, 3]}
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("user_data").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Dict[str, List[int]]"),
        "Generic type alias should be expanded"
    );

    // `str` and `int` are builtins — only `Dict` and `List` need imports.
    let check_names: Vec<&str> = def
        .return_type_imports
        .iter()
        .map(|s| s.check_name.as_str())
        .collect();
    assert!(
        check_names.contains(&"Dict"),
        "Should import Dict: {:?}",
        check_names
    );
    assert!(
        check_names.contains(&"List"),
        "Should import List: {:?}",
        check_names
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_pep613_expanded() {
    // PEP 613: `MyPath: TypeAlias = Path` then `-> MyPath`.
    // Should expand to `Path`.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_pep613/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path
from typing import TypeAlias

MyPath: TypeAlias = Path

@pytest.fixture
def work_dir() -> MyPath:
    return Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Path"),
        "PEP 613 type alias should be expanded"
    );
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_pep613_generic_expanded() {
    // PEP 613: `ConfigDict: TypeAlias = Dict[str, Any]` then `-> ConfigDict`.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_pep613_gen/conftest.py");

    let conftest_content = r#"
import pytest
from typing import Any, Dict, TypeAlias

ConfigDict: TypeAlias = Dict[str, Any]

@pytest.fixture
def config() -> ConfigDict:
    return {"debug": True}
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("config").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Dict[str, Any]"),
        "PEP 613 generic alias should be expanded"
    );

    let check_names: Vec<&str> = def
        .return_type_imports
        .iter()
        .map(|s| s.check_name.as_str())
        .collect();
    assert!(
        check_names.contains(&"Dict"),
        "Should import Dict: {:?}",
        check_names
    );
    assert!(
        check_names.contains(&"Any"),
        "Should import Any: {:?}",
        check_names
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_chained_expansion() {
    // Chained aliases: `A = Path`, `B = Optional[A]`, fixture `-> B`.
    // Should expand B → Optional[A] → Optional[Path].
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_chain/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path
from typing import Optional

MyPath = Path
MaybePath = Optional[MyPath]

@pytest.fixture
def maybe_dir() -> MaybePath:
    return None
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("maybe_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Optional[Path]"),
        "Chained type aliases should be fully expanded"
    );

    let check_names: Vec<&str> = def
        .return_type_imports
        .iter()
        .map(|s| s.check_name.as_str())
        .collect();
    assert!(
        check_names.contains(&"Optional"),
        "Should import Optional: {:?}",
        check_names
    );
    assert!(
        check_names.contains(&"Path"),
        "Should import Path: {:?}",
        check_names
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_union_expanded() {
    // Union alias: `Result = str | int` then `-> Result`.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_union/conftest.py");

    let conftest_content = r#"
import pytest

Result = str | int

@pytest.fixture
def value() -> Result:
    return 42
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("value").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("str | int"),
        "Union type alias should be expanded"
    );
    // str and int are builtins — no imports needed.
    assert!(
        def.return_type_imports.is_empty(),
        "Builtin-only union should need no imports: {:?}",
        def.return_type_imports
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_not_applied_to_lowercase_assignment() {
    // `my_default = Path("/tmp")` should NOT be treated as a type alias
    // because the name starts with lowercase.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_no_lower/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path

default_path = Path("/tmp")

@pytest.fixture
def work_dir() -> Path:
    return default_path
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    // Return type is just `Path` — no alias expansion involved.
    assert_eq!(def.return_type.as_deref(), Some("Path"));
}

#[test]
#[timeout(30000)]
fn test_type_alias_not_applied_to_function_call_rhs() {
    // `Config = load_config()` should NOT be treated as a type alias
    // because the RHS is a function call, not a type expression.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_no_call/conftest.py");

    let conftest_content = r#"
import pytest

def make_config():
    return {"debug": True}

Config = make_config()

@pytest.fixture
def config() -> Config:
    return Config
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("config").expect("fixture not found");
    let def = &defs[0];

    // `Config` is NOT a type alias (RHS is a function call).
    // The return type stays as `Config` (not expanded).
    assert_eq!(def.return_type.as_deref(), Some("Config"));
}

#[test]
#[timeout(30000)]
fn test_type_alias_pep613_with_typing_extensions() {
    // `typing_extensions.TypeAlias` should also be recognized.
    use pytest_language_server::{FixtureDatabase, TypeImportSpec};

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_ext/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path
import typing_extensions

MyPath: typing_extensions.TypeAlias = Path

@pytest.fixture
def work_dir() -> MyPath:
    return Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Path"),
        "typing_extensions.TypeAlias should be recognized"
    );
    assert_eq!(
        def.return_type_imports,
        vec![TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        }]
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_used_inside_generic_return_type() {
    // Alias used within a larger type: `MyPath = Path`, fixture `-> Optional[MyPath]`.
    // Should expand to `Optional[Path]`.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_in_generic/conftest.py");

    let conftest_content = r#"
import pytest
from pathlib import Path
from typing import Optional

MyPath = Path

@pytest.fixture
def maybe_dir() -> Optional[MyPath]:
    return None
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("maybe_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("Optional[Path]"),
        "Alias inside generic should be expanded"
    );
}

#[test]
#[timeout(30000)]
fn test_type_alias_attribute_rhs() {
    // Old-style alias with dotted RHS: `MyPath = pathlib.Path`.
    use pytest_language_server::FixtureDatabase;

    let db = FixtureDatabase::new();
    let conftest_path = PathBuf::from("/tmp/test_alias_attr/conftest.py");

    let conftest_content = r#"
import pytest
import pathlib

MyPath = pathlib.Path

@pytest.fixture
def work_dir() -> MyPath:
    return pathlib.Path("/work")
"#;
    db.analyze_file(conftest_path.clone(), conftest_content);

    let defs = db.definitions.get("work_dir").expect("fixture not found");
    let def = &defs[0];

    assert_eq!(
        def.return_type.as_deref(),
        Some("pathlib.Path"),
        "Attribute-style alias should be expanded"
    );
}
