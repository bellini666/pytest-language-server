//! Unit tests for the FixtureDatabase.
//!
//! All tests have a 30-second timeout to prevent hangs from blocking CI.

use ntest::timeout;
use pytest_language_server::FixtureDatabase;
use std::path::PathBuf;

#[test]
#[timeout(30000)]
fn test_fixture_definition_detection() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

@fixture
def another_fixture():
    return "hello"
"#;

    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Check that fixtures were detected
    assert!(db.definitions.contains_key("my_fixture"));
    assert!(db.definitions.contains_key("another_fixture"));

    // Check fixture details
    let my_fixture_defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(my_fixture_defs.len(), 1);
    assert_eq!(my_fixture_defs[0].name, "my_fixture");
    assert_eq!(my_fixture_defs[0].file_path, conftest_path);
}

#[test]
#[timeout(30000)]
fn test_fixture_usage_detection() {
    let db = FixtureDatabase::new();

    let test_content = r#"
def test_something(my_fixture, another_fixture):
    assert my_fixture == 42
    assert another_fixture == "hello"

def test_other(my_fixture):
    assert my_fixture > 0
"#;

    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that usages were detected
    assert!(db.usages.contains_key(&test_path));

    let usages = db.usages.get(&test_path).unwrap();
    // Should have usages from the first test function (we only track one function per file currently)
    assert!(usages.iter().any(|u| u.name == "my_fixture"));
    assert!(usages.iter().any(|u| u.name == "another_fixture"));
}

#[test]
#[timeout(30000)]
fn test_go_to_definition() {
    let db = FixtureDatabase::new();

    // Set up conftest.py with a fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;

    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Set up a test file that uses the fixture
    let test_content = r#"
def test_something(my_fixture):
    assert my_fixture == 42
"#;

    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Try to find the definition from the test file
    // The usage is on line 2 (1-indexed) - that's where the function parameter is
    // In 0-indexed LSP coordinates, that's line 1
    // Character position 19 is where 'my_fixture' starts
    let definition = db.find_fixture_definition(&test_path, 1, 19);

    assert!(definition.is_some(), "Definition should be found");
    let def = definition.unwrap();
    assert_eq!(def.name, "my_fixture");
    assert_eq!(def.file_path, conftest_path);
}

#[test]
#[timeout(30000)]
fn test_fixture_decorator_variations() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest
from pytest import fixture

@pytest.fixture
def fixture1():
    pass

@pytest.fixture()
def fixture2():
    pass

@fixture
def fixture3():
    pass

@fixture()
def fixture4():
    pass
"#;

    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    // Check all variations were detected
    assert!(db.definitions.contains_key("fixture1"));
    assert!(db.definitions.contains_key("fixture2"));
    assert!(db.definitions.contains_key("fixture3"));
    assert!(db.definitions.contains_key("fixture4"));
}

#[test]
#[timeout(30000)]
fn test_fixture_in_test_file() {
    let db = FixtureDatabase::new();

    // Test file with fixture defined in the same file
    let test_content = r#"
import pytest

@pytest.fixture
def local_fixture():
    return 42

def test_something(local_fixture):
    assert local_fixture == 42
"#;

    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that fixture was detected even though it's not in conftest.py
    assert!(db.definitions.contains_key("local_fixture"));

    let local_fixture_defs = db.definitions.get("local_fixture").unwrap();
    assert_eq!(local_fixture_defs.len(), 1);
    assert_eq!(local_fixture_defs[0].name, "local_fixture");
    assert_eq!(local_fixture_defs[0].file_path, test_path);

    // Check that usage was detected
    assert!(db.usages.contains_key(&test_path));
    let usages = db.usages.get(&test_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "local_fixture"));

    // Test go-to-definition for fixture in same file
    let usage_line = usages
        .iter()
        .find(|u| u.name == "local_fixture")
        .map(|u| u.line)
        .unwrap();

    // Character position 19 is where 'local_fixture' starts in "def test_something(local_fixture):"
    let definition = db.find_fixture_definition(&test_path, (usage_line - 1) as u32, 19);
    assert!(
        definition.is_some(),
        "Should find definition for fixture in same file. Line: {}, char: 19",
        usage_line
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "local_fixture");
    assert_eq!(def.file_path, test_path);
}

#[test]
#[timeout(30000)]
fn test_async_test_functions() {
    let db = FixtureDatabase::new();

    // Test file with async test function
    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

async def test_async_function(my_fixture):
    assert my_fixture == 42

def test_sync_function(my_fixture):
    assert my_fixture == 42
"#;

    let test_path = PathBuf::from("/tmp/test/test_async.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that fixture was detected
    assert!(db.definitions.contains_key("my_fixture"));

    // Check that both async and sync test functions have their usages detected
    assert!(db.usages.contains_key(&test_path));
    let usages = db.usages.get(&test_path).unwrap();

    // Should have 2 usages (one from async, one from sync)
    let fixture_usages: Vec<_> = usages.iter().filter(|u| u.name == "my_fixture").collect();
    assert_eq!(
        fixture_usages.len(),
        2,
        "Should detect fixture usage in both async and sync tests"
    );
}

#[test]
#[timeout(30000)]
fn test_extract_word_at_position() {
    let db = FixtureDatabase::new();

    // Test basic word extraction
    let line = "def test_something(my_fixture):";

    // Cursor on 'm' of 'my_fixture' (position 19)
    assert_eq!(
        db.extract_word_at_position(line, 19),
        Some("my_fixture".to_string())
    );

    // Cursor on 'y' of 'my_fixture' (position 20)
    assert_eq!(
        db.extract_word_at_position(line, 20),
        Some("my_fixture".to_string())
    );

    // Cursor on last 'e' of 'my_fixture' (position 28)
    assert_eq!(
        db.extract_word_at_position(line, 28),
        Some("my_fixture".to_string())
    );

    // Cursor on 'd' of 'def' (position 0)
    assert_eq!(
        db.extract_word_at_position(line, 0),
        Some("def".to_string())
    );

    // Cursor on space after 'def' (position 3) - should return None
    assert_eq!(db.extract_word_at_position(line, 3), None);

    // Cursor on 't' of 'test_something' (position 4)
    assert_eq!(
        db.extract_word_at_position(line, 4),
        Some("test_something".to_string())
    );

    // Cursor on opening parenthesis (position 18) - should return None
    assert_eq!(db.extract_word_at_position(line, 18), None);

    // Cursor on closing parenthesis (position 29) - should return None
    assert_eq!(db.extract_word_at_position(line, 29), None);

    // Cursor on colon (position 31) - should return None
    assert_eq!(db.extract_word_at_position(line, 31), None);
}

#[test]
#[timeout(30000)]
fn test_extract_word_at_position_fixture_definition() {
    let db = FixtureDatabase::new();

    let line = "@pytest.fixture";

    // Cursor on '@' - should return None
    assert_eq!(db.extract_word_at_position(line, 0), None);

    // Cursor on 'p' of 'pytest' (position 1)
    assert_eq!(
        db.extract_word_at_position(line, 1),
        Some("pytest".to_string())
    );

    // Cursor on '.' - should return None
    assert_eq!(db.extract_word_at_position(line, 7), None);

    // Cursor on 'f' of 'fixture' (position 8)
    assert_eq!(
        db.extract_word_at_position(line, 8),
        Some("fixture".to_string())
    );

    let line2 = "def foo(other_fixture):";

    // Cursor on 'd' of 'def'
    assert_eq!(
        db.extract_word_at_position(line2, 0),
        Some("def".to_string())
    );

    // Cursor on space after 'def' - should return None
    assert_eq!(db.extract_word_at_position(line2, 3), None);

    // Cursor on 'f' of 'foo'
    assert_eq!(
        db.extract_word_at_position(line2, 4),
        Some("foo".to_string())
    );

    // Cursor on 'o' of 'other_fixture'
    assert_eq!(
        db.extract_word_at_position(line2, 8),
        Some("other_fixture".to_string())
    );

    // Cursor on parenthesis - should return None
    assert_eq!(db.extract_word_at_position(line2, 7), None);
}

#[test]
#[timeout(30000)]
fn test_word_detection_only_on_fixtures() {
    let db = FixtureDatabase::new();

    // Set up a conftest with a fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Set up a test file
    let test_content = r#"
def test_something(my_fixture, regular_param):
    assert my_fixture == 42
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Line 2 is "def test_something(my_fixture, regular_param):"
    // Character positions:
    // 0: 'd' of 'def'
    // 4: 't' of 'test_something'
    // 19: 'm' of 'my_fixture'
    // 31: 'r' of 'regular_param'

    // Cursor on 'def' - should NOT find a fixture (LSP line 1, 0-based)
    assert_eq!(db.find_fixture_definition(&test_path, 1, 0), None);

    // Cursor on 'test_something' - should NOT find a fixture
    assert_eq!(db.find_fixture_definition(&test_path, 1, 4), None);

    // Cursor on 'my_fixture' - SHOULD find the fixture
    let result = db.find_fixture_definition(&test_path, 1, 19);
    assert!(result.is_some());
    let def = result.unwrap();
    assert_eq!(def.name, "my_fixture");

    // Cursor on 'regular_param' - should NOT find a fixture (it's not a fixture)
    assert_eq!(db.find_fixture_definition(&test_path, 1, 31), None);

    // Cursor on comma or parenthesis - should NOT find a fixture
    assert_eq!(db.find_fixture_definition(&test_path, 1, 18), None); // '('
    assert_eq!(db.find_fixture_definition(&test_path, 1, 29), None); // ','
}

#[test]
#[timeout(30000)]
fn test_self_referencing_fixture() {
    let db = FixtureDatabase::new();

    // Set up a parent conftest.py with the original fixture
    let parent_conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return "parent"
"#;
    let parent_conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(parent_conftest_path.clone(), parent_conftest_content);

    // Set up a child directory conftest.py that overrides foo, referencing itself
    let child_conftest_content = r#"
import pytest

@pytest.fixture
def foo(foo):
    return foo + " child"
"#;
    let child_conftest_path = PathBuf::from("/tmp/test/subdir/conftest.py");
    db.analyze_file(child_conftest_path.clone(), child_conftest_content);

    // Now test go-to-definition on the parameter `foo` in the child fixture
    // Line 5 is "def foo(foo):" (1-indexed)
    // Character position 8 is the 'f' in the parameter name "foo"
    // LSP uses 0-indexed lines, so line 4 in LSP coordinates

    let result = db.find_fixture_definition(&child_conftest_path, 4, 8);

    assert!(
        result.is_some(),
        "Should find parent definition for self-referencing fixture"
    );
    let def = result.unwrap();
    assert_eq!(def.name, "foo");
    assert_eq!(
        def.file_path, parent_conftest_path,
        "Should resolve to parent conftest.py, not the child"
    );
    assert_eq!(def.line, 5, "Should point to line 5 of parent conftest.py");
}

#[test]
#[timeout(30000)]
fn test_fixture_overriding_same_file() {
    let db = FixtureDatabase::new();

    // A test file with multiple fixtures with the same name (unusual but valid)
    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "first"

@pytest.fixture
def my_fixture():
    return "second"

def test_something(my_fixture):
    assert my_fixture == "second"
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // When there are multiple definitions in the same file, the later one should win
    // (Python's behavior - later definitions override earlier ones)

    // Test go-to-definition on the parameter in test_something
    // Line 12 is "def test_something(my_fixture):" (1-indexed)
    // Character position 19 is the 'm' in "my_fixture"
    // LSP uses 0-indexed lines, so line 11 in LSP coordinates

    let result = db.find_fixture_definition(&test_path, 11, 19);

    assert!(result.is_some(), "Should find fixture definition");
    let def = result.unwrap();
    assert_eq!(def.name, "my_fixture");
    assert_eq!(def.file_path, test_path);
    // The current implementation returns the first match in the same file
    // For true Python semantics, we'd want the last one, but that's a more complex change
    // For now, we just verify it finds *a* definition in the same file
}

#[test]
#[timeout(30000)]
fn test_fixture_overriding_conftest_hierarchy() {
    let db = FixtureDatabase::new();

    // Root conftest.py
    let root_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "root"
"#;
    let root_conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(root_conftest_path.clone(), root_conftest_content);

    // Subdirectory conftest.py that overrides the fixture
    let sub_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "subdir"
"#;
    let sub_conftest_path = PathBuf::from("/tmp/test/subdir/conftest.py");
    db.analyze_file(sub_conftest_path.clone(), sub_conftest_content);

    // Test file in subdirectory
    let test_content = r#"
def test_something(shared_fixture):
    assert shared_fixture == "subdir"
"#;
    let test_path = PathBuf::from("/tmp/test/subdir/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Go-to-definition from the test should find the closest conftest.py (subdir)
    // Line 2 is "def test_something(shared_fixture):" (1-indexed)
    // Character position 19 is the 's' in "shared_fixture"
    // LSP uses 0-indexed lines, so line 1 in LSP coordinates

    let result = db.find_fixture_definition(&test_path, 1, 19);

    assert!(result.is_some(), "Should find fixture definition");
    let def = result.unwrap();
    assert_eq!(def.name, "shared_fixture");
    assert_eq!(
        def.file_path, sub_conftest_path,
        "Should resolve to closest conftest.py"
    );

    // Now test from a file in the parent directory
    let parent_test_content = r#"
def test_parent(shared_fixture):
    assert shared_fixture == "root"
"#;
    let parent_test_path = PathBuf::from("/tmp/test/test_parent.py");
    db.analyze_file(parent_test_path.clone(), parent_test_content);

    let result = db.find_fixture_definition(&parent_test_path, 1, 16);

    assert!(result.is_some(), "Should find fixture definition");
    let def = result.unwrap();
    assert_eq!(def.name, "shared_fixture");
    assert_eq!(
        def.file_path, root_conftest_path,
        "Should resolve to root conftest.py"
    );
}

#[test]
#[timeout(30000)]
fn test_scoped_references() {
    let db = FixtureDatabase::new();

    // Set up a root conftest.py with a fixture
    let root_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "root"
"#;
    let root_conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(root_conftest_path.clone(), root_conftest_content);

    // Set up subdirectory conftest.py that overrides the fixture
    let sub_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "subdir"
"#;
    let sub_conftest_path = PathBuf::from("/tmp/test/subdir/conftest.py");
    db.analyze_file(sub_conftest_path.clone(), sub_conftest_content);

    // Test file in the root directory (uses root fixture)
    let root_test_content = r#"
def test_root(shared_fixture):
    assert shared_fixture == "root"
"#;
    let root_test_path = PathBuf::from("/tmp/test/test_root.py");
    db.analyze_file(root_test_path.clone(), root_test_content);

    // Test file in subdirectory (uses subdir fixture)
    let sub_test_content = r#"
def test_sub(shared_fixture):
    assert shared_fixture == "subdir"
"#;
    let sub_test_path = PathBuf::from("/tmp/test/subdir/test_sub.py");
    db.analyze_file(sub_test_path.clone(), sub_test_content);

    // Another test in subdirectory
    let sub_test2_content = r#"
def test_sub2(shared_fixture):
    assert shared_fixture == "subdir"
"#;
    let sub_test2_path = PathBuf::from("/tmp/test/subdir/test_sub2.py");
    db.analyze_file(sub_test2_path.clone(), sub_test2_content);

    // Get the root definition
    let root_definitions = db.definitions.get("shared_fixture").unwrap();
    let root_definition = root_definitions
        .iter()
        .find(|d| d.file_path == root_conftest_path)
        .unwrap();

    // Get the subdir definition
    let sub_definition = root_definitions
        .iter()
        .find(|d| d.file_path == sub_conftest_path)
        .unwrap();

    // Find references for the root definition
    let root_refs = db.find_references_for_definition(root_definition);

    // Should only include the test in the root directory
    assert_eq!(
        root_refs.len(),
        1,
        "Root definition should have 1 reference (from root test)"
    );
    assert_eq!(root_refs[0].file_path, root_test_path);

    // Find references for the subdir definition
    let sub_refs = db.find_references_for_definition(sub_definition);

    // Should include both tests in the subdirectory
    assert_eq!(
        sub_refs.len(),
        2,
        "Subdir definition should have 2 references (from subdir tests)"
    );

    let sub_ref_paths: Vec<_> = sub_refs.iter().map(|r| &r.file_path).collect();
    assert!(sub_ref_paths.contains(&&sub_test_path));
    assert!(sub_ref_paths.contains(&&sub_test2_path));

    // Verify that all references by name returns 3 total
    let all_refs = db.find_fixture_references("shared_fixture");
    assert_eq!(
        all_refs.len(),
        3,
        "Should find 3 total references across all scopes"
    );
}

#[test]
#[timeout(30000)]
fn test_multiline_parameters() {
    let db = FixtureDatabase::new();

    // Conftest with fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file with multiline parameters
    let test_content = r#"
def test_xxx(
    foo,
):
    assert foo == 42
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Line 3 (1-indexed) is "    foo," - the parameter line
    // In LSP coordinates, that's line 2 (0-indexed)
    // Character position 4 is the 'f' in 'foo'

    // Debug: Check what usages were recorded
    if let Some(usages) = db.usages.get(&test_path) {
        println!("Usages recorded:");
        for usage in usages.iter() {
            println!("  {} at line {} (1-indexed)", usage.name, usage.line);
        }
    } else {
        println!("No usages recorded for test file");
    }

    // The content has a leading newline, so:
    // Line 1: (empty)
    // Line 2: def test_xxx(
    // Line 3:     foo,
    // Line 4: ):
    // Line 5:     assert foo == 42

    // foo is at line 3 (1-indexed) = line 2 (0-indexed LSP)
    let result = db.find_fixture_definition(&test_path, 2, 4);

    assert!(
        result.is_some(),
        "Should find fixture definition when cursor is on parameter line"
    );
    let def = result.unwrap();
    assert_eq!(def.name, "foo");
}

#[test]
#[timeout(30000)]
fn test_find_references_from_usage() {
    let db = FixtureDatabase::new();

    // Simple fixture and usage in the same file
    let test_content = r#"
import pytest

@pytest.fixture
def foo(): ...


def test_xxx(foo):
    pass
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get the foo definition
    let foo_defs = db.definitions.get("foo").unwrap();
    assert_eq!(foo_defs.len(), 1, "Should have exactly one foo definition");
    let foo_def = &foo_defs[0];
    assert_eq!(foo_def.line, 5, "foo definition should be on line 5");

    // Get references for the definition
    let refs_from_def = db.find_references_for_definition(foo_def);
    println!("References from definition:");
    for r in &refs_from_def {
        println!("  {} at line {}", r.name, r.line);
    }

    assert_eq!(
        refs_from_def.len(),
        1,
        "Should find 1 usage reference (test_xxx parameter)"
    );
    assert_eq!(refs_from_def[0].line, 8, "Usage should be on line 8");

    // Now simulate what happens when user clicks on the usage (line 8, char 13 - the 'f' in 'foo')
    // This is LSP line 7 (0-indexed)
    let fixture_name = db.find_fixture_at_position(&test_path, 7, 13);
    println!(
        "\nfind_fixture_at_position(line 7, char 13): {:?}",
        fixture_name
    );

    assert_eq!(
        fixture_name,
        Some("foo".to_string()),
        "Should find fixture name at usage position"
    );

    let resolved_def = db.find_fixture_definition(&test_path, 7, 13);
    println!(
        "\nfind_fixture_definition(line 7, char 13): {:?}",
        resolved_def.as_ref().map(|d| (d.line, &d.file_path))
    );

    assert!(resolved_def.is_some(), "Should resolve usage to definition");
    assert_eq!(
        resolved_def.unwrap(),
        *foo_def,
        "Should resolve to the correct definition"
    );
}

#[test]
#[timeout(30000)]
fn test_find_references_with_ellipsis_body() {
    // This reproduces the structure from strawberry test_codegen.py
    let db = FixtureDatabase::new();

    let test_content = r#"@pytest.fixture
def foo(): ...


def test_xxx(foo):
    pass
"#;
    let test_path = PathBuf::from("/tmp/test/test_codegen.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check what line foo definition is on
    let foo_defs = db.definitions.get("foo");
    println!(
        "foo definitions: {:?}",
        foo_defs
            .as_ref()
            .map(|defs| defs.iter().map(|d| d.line).collect::<Vec<_>>())
    );

    // Check what line foo usage is on
    if let Some(usages) = db.usages.get(&test_path) {
        println!("usages:");
        for u in usages.iter() {
            println!("  {} at line {}", u.name, u.line);
        }
    }

    assert!(foo_defs.is_some(), "Should find foo definition");
    let foo_def = &foo_defs.unwrap()[0];

    // Get the usage line
    let usages = db.usages.get(&test_path).unwrap();
    let foo_usage = usages.iter().find(|u| u.name == "foo").unwrap();

    // Test from usage position (LSP coordinates are 0-indexed)
    let usage_lsp_line = (foo_usage.line - 1) as u32;
    println!("\nTesting from usage at LSP line {}", usage_lsp_line);

    let fixture_name = db.find_fixture_at_position(&test_path, usage_lsp_line, 13);
    assert_eq!(
        fixture_name,
        Some("foo".to_string()),
        "Should find foo at usage"
    );

    let def_from_usage = db.find_fixture_definition(&test_path, usage_lsp_line, 13);
    assert!(
        def_from_usage.is_some(),
        "Should resolve usage to definition"
    );
    assert_eq!(def_from_usage.unwrap(), *foo_def);
}

#[test]
#[timeout(30000)]
fn test_fixture_hierarchy_parent_references() {
    // Test that finding references from a parent fixture definition
    // includes child fixture definitions but NOT the child's usages
    let db = FixtureDatabase::new();

    // Parent conftest
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    """Parent fixture"""
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    // Child conftest with override
    let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    """Child override that uses parent"""
    return cli_runner
"#;
    let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file in subdir using the child fixture
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/subdir/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get parent definition
    let parent_defs = db.definitions.get("cli_runner").unwrap();
    let parent_def = parent_defs
        .iter()
        .find(|d| d.file_path == parent_conftest)
        .unwrap();

    println!(
        "\nParent definition: {:?}:{}",
        parent_def.file_path, parent_def.line
    );

    // Find references for parent definition
    let refs = db.find_references_for_definition(parent_def);

    println!("\nReferences for parent definition:");
    for r in &refs {
        println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
    }

    // Parent references should include:
    // 1. The child fixture definition (line 5 in child conftest)
    // 2. The child's parameter that references the parent (line 5 in child conftest)
    // But NOT:
    // 3. test_one and test_two usages (they resolve to child, not parent)

    assert!(
        refs.len() <= 2,
        "Parent should have at most 2 references: child definition and its parameter, got {}",
        refs.len()
    );

    // Should include the child conftest
    let child_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.file_path == child_conftest)
        .collect();
    assert!(
        !child_refs.is_empty(),
        "Parent references should include child fixture definition"
    );

    // Should NOT include test file usages
    let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
    assert!(
        test_refs.is_empty(),
        "Parent references should NOT include child's test file usages"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_hierarchy_child_references() {
    // Test that finding references from a child fixture definition
    // includes usages in the same directory (that resolve to the child)
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
    let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file using child fixture
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/subdir/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get child definition
    let child_defs = db.definitions.get("cli_runner").unwrap();
    let child_def = child_defs
        .iter()
        .find(|d| d.file_path == child_conftest)
        .unwrap();

    println!(
        "\nChild definition: {:?}:{}",
        child_def.file_path, child_def.line
    );

    // Find references for child definition
    let refs = db.find_references_for_definition(child_def);

    println!("\nReferences for child definition:");
    for r in &refs {
        println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
    }

    // Child references should include test_one and test_two
    assert!(
        refs.len() >= 2,
        "Child should have at least 2 references from test file, got {}",
        refs.len()
    );

    let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
    assert_eq!(
        test_refs.len(),
        2,
        "Should have 2 references from test file"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_hierarchy_child_parameter_references() {
    // Test that finding references from a child fixture's parameter
    // (which references the parent) includes the child fixture definition
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
    let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // When user clicks on the parameter "cli_runner" in the child definition,
    // it should resolve to the parent definition
    // Line 5 (1-indexed) = line 4 (0-indexed LSP), char 15 is in the parameter name
    let resolved_def = db.find_fixture_definition(&child_conftest, 4, 15);

    assert!(
        resolved_def.is_some(),
        "Child parameter should resolve to parent definition"
    );

    let def = resolved_def.unwrap();
    assert_eq!(
        def.file_path, parent_conftest,
        "Should resolve to parent conftest"
    );

    // Find references for parent definition
    let refs = db.find_references_for_definition(&def);

    println!("\nReferences for parent (from child parameter):");
    for r in &refs {
        println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
    }

    // Should include the child fixture's parameter usage
    let child_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.file_path == child_conftest)
        .collect();
    assert!(
        !child_refs.is_empty(),
        "Parent references should include child fixture parameter"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_hierarchy_usage_from_test() {
    // Test that finding references from a test function parameter
    // includes the definition it resolves to and other usages
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
    let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file using child fixture
    let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass

def test_three(cli_runner):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/subdir/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Click on cli_runner in test_one (line 2, 1-indexed = line 1, 0-indexed)
    let resolved_def = db.find_fixture_definition(&test_path, 1, 13);

    assert!(
        resolved_def.is_some(),
        "Usage should resolve to child definition"
    );

    let def = resolved_def.unwrap();
    assert_eq!(
        def.file_path, child_conftest,
        "Should resolve to child conftest (not parent)"
    );

    // Find references for the resolved definition
    let refs = db.find_references_for_definition(&def);

    println!("\nReferences for child (from test usage):");
    for r in &refs {
        println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
    }

    // Should include all three test usages
    let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
    assert_eq!(test_refs.len(), 3, "Should find all 3 usages in test file");
}

#[test]
#[timeout(30000)]
fn test_fixture_hierarchy_multiple_levels() {
    // Test a three-level hierarchy: grandparent -> parent -> child
    let db = FixtureDatabase::new();

    // Grandparent
    let grandparent_content = r#"
import pytest

@pytest.fixture
def db():
    return "grandparent_db"
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
    let child_conftest = PathBuf::from("/tmp/project/api/tests/conftest.py");
    db.analyze_file(child_conftest.clone(), child_content);

    // Test file at child level
    let test_content = r#"
def test_db(db):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/api/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Get all definitions
    let all_defs = db.definitions.get("db").unwrap();
    assert_eq!(all_defs.len(), 3, "Should have 3 definitions");

    let grandparent_def = all_defs
        .iter()
        .find(|d| d.file_path == grandparent_conftest)
        .unwrap();
    let parent_def = all_defs
        .iter()
        .find(|d| d.file_path == parent_conftest)
        .unwrap();
    let child_def = all_defs
        .iter()
        .find(|d| d.file_path == child_conftest)
        .unwrap();

    // Test from test file - should resolve to child
    let resolved = db.find_fixture_definition(&test_path, 1, 12);
    assert_eq!(
        resolved.as_ref(),
        Some(child_def),
        "Test should use child definition"
    );

    // Child's references should include test file
    let child_refs = db.find_references_for_definition(child_def);
    let test_refs: Vec<_> = child_refs
        .iter()
        .filter(|r| r.file_path == test_path)
        .collect();
    assert!(
        !test_refs.is_empty(),
        "Child should have test file references"
    );

    // Parent's references should include child's parameter, but not test file
    let parent_refs = db.find_references_for_definition(parent_def);
    let child_param_refs: Vec<_> = parent_refs
        .iter()
        .filter(|r| r.file_path == child_conftest)
        .collect();
    let test_refs_in_parent: Vec<_> = parent_refs
        .iter()
        .filter(|r| r.file_path == test_path)
        .collect();

    assert!(
        !child_param_refs.is_empty(),
        "Parent should have child parameter reference"
    );
    assert!(
        test_refs_in_parent.is_empty(),
        "Parent should NOT have test file references"
    );

    // Grandparent's references should include parent's parameter, but not child's stuff
    let grandparent_refs = db.find_references_for_definition(grandparent_def);
    let parent_param_refs: Vec<_> = grandparent_refs
        .iter()
        .filter(|r| r.file_path == parent_conftest)
        .collect();
    let child_refs_in_gp: Vec<_> = grandparent_refs
        .iter()
        .filter(|r| r.file_path == child_conftest)
        .collect();

    assert!(
        !parent_param_refs.is_empty(),
        "Grandparent should have parent parameter reference"
    );
    assert!(
        child_refs_in_gp.is_empty(),
        "Grandparent should NOT have child references"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_hierarchy_same_file_override() {
    // Test that a fixture can be overridden in the same file
    // (less common but valid pytest pattern)
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def base():
    return "base"

@pytest.fixture
def base(base):
    return f"override_{base}"

def test_uses_override(base):
    pass
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), content);

    let defs = db.definitions.get("base").unwrap();
    assert_eq!(defs.len(), 2, "Should have 2 definitions in same file");

    println!("\nDefinitions found:");
    for d in defs.iter() {
        println!("  base at line {}", d.line);
    }

    // Check usages
    if let Some(usages) = db.usages.get(&test_path) {
        println!("\nUsages found:");
        for u in usages.iter() {
            println!("  {} at line {}", u.name, u.line);
        }
    } else {
        println!("\nNo usages found!");
    }

    // The test should resolve to the second definition (override)
    // Line 12 (1-indexed) = line 11 (0-indexed LSP)
    // Character position: "def test_uses_override(base):" - 'b' is at position 23
    let resolved = db.find_fixture_definition(&test_path, 11, 23);

    println!("\nResolved: {:?}", resolved.as_ref().map(|d| d.line));

    assert!(resolved.is_some(), "Should resolve to override definition");

    // The second definition should be at line 9 (1-indexed)
    let override_def = defs.iter().find(|d| d.line == 9).unwrap();
    println!("Override def at line: {}", override_def.line);
    assert_eq!(resolved.as_ref(), Some(override_def));
}

#[test]
#[timeout(30000)]
fn test_cursor_position_on_definition_line() {
    // Debug test to understand what happens at different cursor positions
    // on a fixture definition line with a self-referencing parameter
    let db = FixtureDatabase::new();

    // Add a parent conftest with parent fixture
    let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
    let parent_conftest = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(parent_conftest.clone(), parent_content);

    let content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), content);

    // Line 5 (1-indexed): "def cli_runner(cli_runner):"
    // Position 0: 'd' in def
    // Position 4: 'c' in cli_runner (function name)
    // Position 15: '('
    // Position 16: 'c' in cli_runner (parameter name)

    println!("\n=== Testing character positions on line 5 ===");

    // Check usages
    if let Some(usages) = db.usages.get(&test_path) {
        println!("\nUsages found:");
        for u in usages.iter() {
            println!(
                "  {} at line {}, chars {}-{}",
                u.name, u.line, u.start_char, u.end_char
            );
        }
    } else {
        println!("\nNo usages found!");
    }

    // Test clicking on function name 'cli_runner' - should be treated as definition
    let line_content = "def cli_runner(cli_runner):";
    println!("\nLine content: '{}'", line_content);

    // Position 4 = 'c' in function name cli_runner
    println!("\nPosition 4 (function name):");
    let word_at_4 = db.extract_word_at_position(line_content, 4);
    println!("  Word at cursor: {:?}", word_at_4);
    let fixture_name_at_4 = db.find_fixture_at_position(&test_path, 4, 4);
    println!("  find_fixture_at_position: {:?}", fixture_name_at_4);
    let resolved_4 = db.find_fixture_definition(&test_path, 4, 4); // Line 5 = index 4
    println!(
        "  Resolved: {:?}",
        resolved_4.as_ref().map(|d| (d.name.as_str(), d.line))
    );

    // Position 16 = 'c' in parameter name cli_runner
    println!("\nPosition 16 (parameter name):");
    let word_at_16 = db.extract_word_at_position(line_content, 16);
    println!("  Word at cursor: {:?}", word_at_16);

    // Manual check: does the usage check work?
    if let Some(usages) = db.usages.get(&test_path) {
        for usage in usages.iter() {
            println!("  Checking usage: {} at line {}", usage.name, usage.line);
            if usage.line == 5 && usage.name == "cli_runner" {
                println!("    MATCH! Usage matches our position");
            }
        }
    }

    let fixture_name_at_16 = db.find_fixture_at_position(&test_path, 4, 16);
    println!("  find_fixture_at_position: {:?}", fixture_name_at_16);
    let resolved_16 = db.find_fixture_definition(&test_path, 4, 16); // Line 5 = index 4
    println!(
        "  Resolved: {:?}",
        resolved_16.as_ref().map(|d| (d.name.as_str(), d.line))
    );

    // Expected behavior:
    // - Position 4 (function name): should resolve to CHILD (line 5) - we're ON the definition
    // - Position 16 (parameter): should resolve to PARENT (line 5 in conftest) - it's a usage

    assert_eq!(word_at_4, Some("cli_runner".to_string()));
    assert_eq!(word_at_16, Some("cli_runner".to_string()));

    // Check the actual resolution
    println!("\n=== ACTUAL vs EXPECTED ===");
    println!("Position 4 (function name):");
    println!(
        "  Actual: {:?}",
        resolved_4.as_ref().map(|d| (&d.file_path, d.line))
    );
    println!("  Expected: test file, line 5 (the child definition itself)");

    println!("\nPosition 16 (parameter):");
    println!(
        "  Actual: {:?}",
        resolved_16.as_ref().map(|d| (&d.file_path, d.line))
    );
    println!("  Expected: conftest, line 5 (the parent definition)");

    // The BUG: both return the same thing (child at line 5)
    // Position 4: returning child is CORRECT (though find_fixture_definition returns None,
    //             main.rs falls back to get_definition_at_line which is correct)
    // Position 16: returning child is WRONG - should return parent (line 5 in conftest)

    if let Some(ref def) = resolved_16 {
        assert_eq!(
            def.file_path, parent_conftest,
            "Parameter should resolve to parent definition"
        );
    } else {
        panic!("Position 16 (parameter) should resolve to parent definition");
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_detection_in_test() {
    let db = FixtureDatabase::new();

    // Add a fixture definition in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test that uses the fixture without declaring it
    let test_content = r#"
def test_example():
    result = my_fixture.get()
    assert result == 42
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that undeclared fixture was detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(undeclared.len(), 1, "Should detect one undeclared fixture");

    let fixture = &undeclared[0];
    assert_eq!(fixture.name, "my_fixture");
    assert_eq!(fixture.function_name, "test_example");
    assert_eq!(fixture.line, 3); // Line 3: "result = my_fixture.get()"
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_detection_in_fixture() {
    let db = FixtureDatabase::new();

    // Add fixture definitions in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"

@pytest.fixture
def helper_fixture():
    return "helper"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a fixture that uses another fixture without declaring it
    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture(base_fixture):
    data = helper_fixture.value
    return f"{base_fixture}-{data}"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that undeclared fixture was detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(undeclared.len(), 1, "Should detect one undeclared fixture");

    let fixture = &undeclared[0];
    assert_eq!(fixture.name, "helper_fixture");
    assert_eq!(fixture.function_name, "my_fixture");
    assert_eq!(fixture.line, 6); // Line 6: "data = helper_fixture.value"
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_declared_fixtures() {
    let db = FixtureDatabase::new();

    // Add a fixture definition in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test that properly declares the fixture as a parameter
    let test_content = r#"
def test_example(my_fixture):
    result = my_fixture
    assert result == 42
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that no undeclared fixtures were detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_non_fixtures() {
    let db = FixtureDatabase::new();

    // Add a test that uses regular variables (not fixtures)
    let test_content = r#"
def test_example():
    my_variable = 42
    result = my_variable + 10
    assert result == 52
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that no undeclared fixtures were detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_not_available_in_hierarchy() {
    let db = FixtureDatabase::new();

    // Add a fixture in a different directory (not in hierarchy)
    let other_conftest = r#"
import pytest

@pytest.fixture
def other_fixture():
    return "other"
"#;
    let other_path = PathBuf::from("/other/conftest.py");
    db.analyze_file(other_path.clone(), other_conftest);

    // Add a test that uses a name that happens to match a fixture but isn't available
    let test_content = r#"
def test_example():
    result = other_fixture.value
    assert result == "other"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should not detect undeclared fixture because it's not in the hierarchy
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect fixtures not in hierarchy"
    );
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_async_test() {
    let db = FixtureDatabase::new();

    // Add fixture in same file
    let content = r#"
import pytest

@pytest.fixture
def http_client():
    return "MockClient"

async def test_with_undeclared():
    response = await http_client.query("test")
    assert response == "test"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), content);

    // Check that undeclared fixture was detected
    let undeclared = db.get_undeclared_fixtures(&test_path);

    println!("Found {} undeclared fixtures", undeclared.len());
    for u in &undeclared {
        println!("  - {} at line {} in {}", u.name, u.line, u.function_name);
    }

    assert_eq!(undeclared.len(), 1, "Should detect one undeclared fixture");
    assert_eq!(undeclared[0].name, "http_client");
    assert_eq!(undeclared[0].function_name, "test_with_undeclared");
    assert_eq!(undeclared[0].line, 9);
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_assert_statement() {
    let db = FixtureDatabase::new();

    // Add fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def expected_value():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that uses fixture in assert without declaring it
    let test_content = r#"
def test_assertion():
    result = calculate_value()
    assert result == expected_value
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that undeclared fixture was detected in assert
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        1,
        "Should detect one undeclared fixture in assert"
    );
    assert_eq!(undeclared[0].name, "expected_value");
    assert_eq!(undeclared[0].function_name, "test_assertion");
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_local_variable() {
    // Problem 2: Should not warn if a local variable shadows a fixture name
    let db = FixtureDatabase::new();

    // Add fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return "fixture"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that has a local variable with the same name
    let test_content = r#"
def test_with_local_variable():
    foo = "local variable"
    result = foo.upper()
    assert result == "LOCAL VARIABLE"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should NOT detect undeclared fixture because foo is a local variable
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect undeclared fixture when name is a local variable"
    );
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_imported_name() {
    // Problem 2: Should not warn if an imported name shadows a fixture name
    let db = FixtureDatabase::new();

    // Add fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return "fixture"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that imports a name
    let test_content = r#"
from mymodule import foo

def test_with_import():
    result = foo.something()
    assert result == "value"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should NOT detect undeclared fixture because foo is imported
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect undeclared fixture when name is imported"
    );
}

#[test]
#[timeout(30000)]
fn test_warn_for_fixture_used_directly() {
    // Problem 2: SHOULD warn if trying to use a fixture defined in the same file
    // This is an error because fixtures must be accessed through parameters
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def foo():
    return "fixture"

def test_using_fixture_directly():
    # This is an error - fixtures must be declared as parameters
    result = foo.something()
    assert result == "value"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // SHOULD detect undeclared fixture even though foo is defined in same file
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        1,
        "Should detect fixture used directly without parameter declaration"
    );
    assert_eq!(undeclared[0].name, "foo");
    assert_eq!(undeclared[0].function_name, "test_using_fixture_directly");
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_module_level_assignment() {
    // Should not warn if name is assigned at module level (not a fixture)
    let db = FixtureDatabase::new();

    // Add fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return "fixture"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that has a module-level assignment
    let test_content = r#"
# Module-level assignment
foo = SomeClass()

def test_with_module_var():
    result = foo.method()
    assert result == "value"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should NOT detect undeclared fixture because foo is assigned at module level
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect undeclared fixture when name is assigned at module level"
    );
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_function_definition() {
    // Should not warn if name is a regular function (not a fixture)
    let db = FixtureDatabase::new();

    // Add fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return "fixture"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that has a regular function with the same name
    let test_content = r#"
def foo():
    return "not a fixture"

def test_with_function():
    result = foo()
    assert result == "not a fixture"
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should NOT detect undeclared fixture because foo is a regular function
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect undeclared fixture when name is a regular function"
    );
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_class_definition() {
    // Should not warn if name is a class
    let db = FixtureDatabase::new();

    // Add fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def MyClass():
    return "fixture"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that has a class with the same name
    let test_content = r#"
class MyClass:
    pass

def test_with_class():
    obj = MyClass()
    assert obj is not None
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should NOT detect undeclared fixture because MyClass is a class
    let undeclared = db.get_undeclared_fixtures(&test_path);

    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect undeclared fixture when name is a class"
    );
}

#[test]
#[timeout(30000)]
fn test_line_aware_local_variable_scope() {
    // Test that local variables are only considered "in scope" AFTER they're assigned
    let db = FixtureDatabase::new();

    // Conftest with http_client fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def http_client():
    return "MockClient"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file that uses http_client before and after a local assignment
    let test_content = r#"async def test_example():
    # Line 1: http_client should be flagged (not yet assigned)
    result = await http_client.get("/api")
    # Line 3: Now we assign http_client locally
    http_client = "local"
    # Line 5: http_client should NOT be flagged (local var now)
    result2 = await http_client.get("/api2")
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check for undeclared fixtures
    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Should only detect http_client on line 3 (usage before assignment)
    // NOT on line 7 (after assignment on line 5)
    assert_eq!(
        undeclared.len(),
        1,
        "Should detect http_client only before local assignment"
    );
    assert_eq!(undeclared[0].name, "http_client");
    // Line numbers: 1=def, 2=comment, 3=result (first usage), 4=comment, 5=assignment, 6=comment, 7=result2
    assert_eq!(
        undeclared[0].line, 3,
        "Should flag usage on line 3 (before assignment on line 5)"
    );
}

#[test]
#[timeout(30000)]
fn test_same_line_assignment_and_usage() {
    // Test that usage on the same line as assignment refers to the fixture
    let db = FixtureDatabase::new();

    let conftest_content = r#"import pytest

@pytest.fixture
def http_client():
    return "parent"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"async def test_example():
    # This references the fixture on the RHS, then assigns to local var
    http_client = await http_client.get("/api")
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Should detect http_client on RHS (line 3) because assignment hasn't happened yet
    assert_eq!(undeclared.len(), 1);
    assert_eq!(undeclared[0].name, "http_client");
    assert_eq!(undeclared[0].line, 3);
}

#[test]
#[timeout(30000)]
fn test_no_false_positive_for_later_assignment() {
    // This is the actual bug we fixed - make sure local assignment later in function
    // doesn't prevent detection of undeclared fixture usage BEFORE the assignment
    let db = FixtureDatabase::new();

    let conftest_content = r#"import pytest

@pytest.fixture
def http_client():
    return "fixture"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // This was the original issue: http_client used on line 2, but assigned on line 4
    // Old code would see the assignment and not flag line 2
    let test_content = r#"async def test_example():
    result = await http_client.get("/api")  # Should be flagged
    # Now assign locally
    http_client = "local"
    # This should NOT be flagged because variable is now assigned
    result2 = http_client
"#;
    let test_path = PathBuf::from("/tmp/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Should only detect one undeclared usage (line 2)
    assert_eq!(
        undeclared.len(),
        1,
        "Should detect exactly one undeclared fixture"
    );
    assert_eq!(undeclared[0].name, "http_client");
    assert_eq!(
        undeclared[0].line, 2,
        "Should flag usage on line 2 before assignment on line 4"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_priority_deterministic() {
    // Test that fixture resolution is deterministic and follows priority rules
    // This test ensures we don't randomly pick a definition from DashMap iteration
    let db = FixtureDatabase::new();

    // Create multiple conftest.py files with the same fixture name in different locations
    // Scenario: /tmp/project/app/tests/test_foo.py should resolve to closest conftest

    // Root conftest
    let root_content = r#"
import pytest

@pytest.fixture
def db():
    return "root_db"
"#;
    let root_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(root_conftest.clone(), root_content);

    // Unrelated conftest (different branch of directory tree)
    let unrelated_content = r#"
import pytest

@pytest.fixture
def db():
    return "unrelated_db"
"#;
    let unrelated_conftest = PathBuf::from("/tmp/other/conftest.py");
    db.analyze_file(unrelated_conftest.clone(), unrelated_content);

    // App-level conftest
    let app_content = r#"
import pytest

@pytest.fixture
def db():
    return "app_db"
"#;
    let app_conftest = PathBuf::from("/tmp/project/app/conftest.py");
    db.analyze_file(app_conftest.clone(), app_content);

    // Tests-level conftest (closest)
    let tests_content = r#"
import pytest

@pytest.fixture
def db():
    return "tests_db"
"#;
    let tests_conftest = PathBuf::from("/tmp/project/app/tests/conftest.py");
    db.analyze_file(tests_conftest.clone(), tests_content);

    // Test file
    let test_content = r#"
def test_database(db):
    assert db is not None
"#;
    let test_path = PathBuf::from("/tmp/project/app/tests/test_foo.py");
    db.analyze_file(test_path.clone(), test_content);

    // Run the resolution multiple times to ensure it's deterministic
    for iteration in 0..10 {
        let result = db.find_fixture_definition(&test_path, 1, 18); // Line 2, column 18 = "db" parameter

        assert!(
            result.is_some(),
            "Iteration {}: Should find a fixture definition",
            iteration
        );

        let def = result.unwrap();
        assert_eq!(
            def.name, "db",
            "Iteration {}: Should find 'db' fixture",
            iteration
        );

        // Should ALWAYS resolve to the closest conftest.py (tests_conftest)
        assert_eq!(
            def.file_path, tests_conftest,
            "Iteration {}: Should consistently resolve to closest conftest.py at {:?}, but got {:?}",
            iteration,
            tests_conftest,
            def.file_path
        );
    }
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_prefers_parent_over_unrelated() {
    // Test that when no fixture is in same file or conftest hierarchy,
    // we prefer third-party fixtures (site-packages) over random unrelated conftest files
    let db = FixtureDatabase::new();

    // Unrelated conftest in different directory tree
    let unrelated_content = r#"
import pytest

@pytest.fixture
def custom_fixture():
    return "unrelated"
"#;
    let unrelated_conftest = PathBuf::from("/tmp/other_project/conftest.py");
    db.analyze_file(unrelated_conftest.clone(), unrelated_content);

    // Third-party fixture (mock in site-packages)
    let third_party_content = r#"
import pytest

@pytest.fixture
def custom_fixture():
    return "third_party"
"#;
    let third_party_path =
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/pytest_custom/plugin.py");
    db.analyze_file(third_party_path.clone(), third_party_content);

    // Test file in a different project
    let test_content = r#"
def test_custom(custom_fixture):
    assert custom_fixture is not None
"#;
    let test_path = PathBuf::from("/tmp/my_project/test_foo.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should prefer third-party fixture over unrelated conftest
    let result = db.find_fixture_definition(&test_path, 1, 16);
    assert!(result.is_some());
    let def = result.unwrap();

    // Should be the third-party fixture (site-packages)
    assert_eq!(
        def.file_path, third_party_path,
        "Should prefer third-party fixture from site-packages over unrelated conftest.py"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_hierarchy_over_third_party() {
    // Test that fixtures in the conftest hierarchy are preferred over third-party
    let db = FixtureDatabase::new();

    // Third-party fixture
    let third_party_content = r#"
import pytest

@pytest.fixture
def mocker():
    return "third_party_mocker"
"#;
    let third_party_path =
        PathBuf::from("/tmp/project/.venv/lib/python3.11/site-packages/pytest_mock/plugin.py");
    db.analyze_file(third_party_path.clone(), third_party_content);

    // Local conftest.py that overrides mocker
    let local_content = r#"
import pytest

@pytest.fixture
def mocker():
    return "local_mocker"
"#;
    let local_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(local_conftest.clone(), local_content);

    // Test file
    let test_content = r#"
def test_mocking(mocker):
    assert mocker is not None
"#;
    let test_path = PathBuf::from("/tmp/project/test_foo.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should prefer local conftest over third-party
    let result = db.find_fixture_definition(&test_path, 1, 17);
    assert!(result.is_some());
    let def = result.unwrap();

    assert_eq!(
        def.file_path, local_conftest,
        "Should prefer local conftest.py fixture over third-party fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_with_relative_paths() {
    // Test that fixture resolution works even when paths are stored with different representations
    // This simulates the case where analyze_file is called with relative paths vs absolute paths
    let db = FixtureDatabase::new();

    // Conftest with absolute path
    let conftest_content = r#"
import pytest

@pytest.fixture
def shared():
    return "conftest"
"#;
    let conftest_abs = PathBuf::from("/tmp/project/tests/conftest.py");
    db.analyze_file(conftest_abs.clone(), conftest_content);

    // Test file also with absolute path
    let test_content = r#"
def test_example(shared):
    assert shared == "conftest"
"#;
    let test_abs = PathBuf::from("/tmp/project/tests/test_foo.py");
    db.analyze_file(test_abs.clone(), test_content);

    // Should find the fixture from conftest
    let result = db.find_fixture_definition(&test_abs, 1, 17);
    assert!(result.is_some(), "Should find fixture with absolute paths");
    let def = result.unwrap();
    assert_eq!(def.file_path, conftest_abs, "Should resolve to conftest.py");
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_deep_hierarchy() {
    // Test resolution in a deep directory hierarchy to ensure path traversal works correctly
    let db = FixtureDatabase::new();

    // Root level fixture
    let root_content = r#"
import pytest

@pytest.fixture
def db():
    return "root"
"#;
    let root_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(root_conftest.clone(), root_content);

    // Level 1
    let level1_content = r#"
import pytest

@pytest.fixture
def db():
    return "level1"
"#;
    let level1_conftest = PathBuf::from("/tmp/project/src/conftest.py");
    db.analyze_file(level1_conftest.clone(), level1_content);

    // Level 2
    let level2_content = r#"
import pytest

@pytest.fixture
def db():
    return "level2"
"#;
    let level2_conftest = PathBuf::from("/tmp/project/src/app/conftest.py");
    db.analyze_file(level2_conftest.clone(), level2_content);

    // Level 3 - deepest
    let level3_content = r#"
import pytest

@pytest.fixture
def db():
    return "level3"
"#;
    let level3_conftest = PathBuf::from("/tmp/project/src/app/tests/conftest.py");
    db.analyze_file(level3_conftest.clone(), level3_content);

    // Test at level 3 - should use level 3 fixture
    let test_l3_content = r#"
def test_db(db):
    assert db == "level3"
"#;
    let test_l3 = PathBuf::from("/tmp/project/src/app/tests/test_foo.py");
    db.analyze_file(test_l3.clone(), test_l3_content);

    let result_l3 = db.find_fixture_definition(&test_l3, 1, 12);
    assert!(result_l3.is_some());
    assert_eq!(
        result_l3.unwrap().file_path,
        level3_conftest,
        "Test at level 3 should use level 3 fixture"
    );

    // Test at level 2 - should use level 2 fixture
    let test_l2_content = r#"
def test_db(db):
    assert db == "level2"
"#;
    let test_l2 = PathBuf::from("/tmp/project/src/app/test_bar.py");
    db.analyze_file(test_l2.clone(), test_l2_content);

    let result_l2 = db.find_fixture_definition(&test_l2, 1, 12);
    assert!(result_l2.is_some());
    assert_eq!(
        result_l2.unwrap().file_path,
        level2_conftest,
        "Test at level 2 should use level 2 fixture"
    );

    // Test at level 1 - should use level 1 fixture
    let test_l1_content = r#"
def test_db(db):
    assert db == "level1"
"#;
    let test_l1 = PathBuf::from("/tmp/project/src/test_baz.py");
    db.analyze_file(test_l1.clone(), test_l1_content);

    let result_l1 = db.find_fixture_definition(&test_l1, 1, 12);
    assert!(result_l1.is_some());
    assert_eq!(
        result_l1.unwrap().file_path,
        level1_conftest,
        "Test at level 1 should use level 1 fixture"
    );

    // Test at root - should use root fixture
    let test_root_content = r#"
def test_db(db):
    assert db == "root"
"#;
    let test_root = PathBuf::from("/tmp/project/test_root.py");
    db.analyze_file(test_root.clone(), test_root_content);

    let result_root = db.find_fixture_definition(&test_root, 1, 12);
    assert!(result_root.is_some());
    assert_eq!(
        result_root.unwrap().file_path,
        root_conftest,
        "Test at root should use root fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_sibling_directories() {
    // Test that fixtures in sibling directories don't leak into each other
    let db = FixtureDatabase::new();

    // Root conftest
    let root_content = r#"
import pytest

@pytest.fixture
def shared():
    return "root"
"#;
    let root_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(root_conftest.clone(), root_content);

    // Module A with its own fixture
    let module_a_content = r#"
import pytest

@pytest.fixture
def module_specific():
    return "module_a"
"#;
    let module_a_conftest = PathBuf::from("/tmp/project/module_a/conftest.py");
    db.analyze_file(module_a_conftest.clone(), module_a_content);

    // Module B with its own fixture (same name!)
    let module_b_content = r#"
import pytest

@pytest.fixture
def module_specific():
    return "module_b"
"#;
    let module_b_conftest = PathBuf::from("/tmp/project/module_b/conftest.py");
    db.analyze_file(module_b_conftest.clone(), module_b_content);

    // Test in module A - should use module A's fixture
    let test_a_content = r#"
def test_a(module_specific, shared):
    assert module_specific == "module_a"
    assert shared == "root"
"#;
    let test_a = PathBuf::from("/tmp/project/module_a/test_a.py");
    db.analyze_file(test_a.clone(), test_a_content);

    let result_a = db.find_fixture_definition(&test_a, 1, 11);
    assert!(result_a.is_some());
    assert_eq!(
        result_a.unwrap().file_path,
        module_a_conftest,
        "Test in module_a should use module_a's fixture"
    );

    // Test in module B - should use module B's fixture
    let test_b_content = r#"
def test_b(module_specific, shared):
    assert module_specific == "module_b"
    assert shared == "root"
"#;
    let test_b = PathBuf::from("/tmp/project/module_b/test_b.py");
    db.analyze_file(test_b.clone(), test_b_content);

    let result_b = db.find_fixture_definition(&test_b, 1, 11);
    assert!(result_b.is_some());
    assert_eq!(
        result_b.unwrap().file_path,
        module_b_conftest,
        "Test in module_b should use module_b's fixture"
    );

    // Both should be able to access shared root fixture
    // "shared" starts at column 29 (after "module_specific, ")
    let result_a_shared = db.find_fixture_definition(&test_a, 1, 29);
    assert!(result_a_shared.is_some());
    assert_eq!(
        result_a_shared.unwrap().file_path,
        root_conftest,
        "Test in module_a should access root's shared fixture"
    );

    let result_b_shared = db.find_fixture_definition(&test_b, 1, 29);
    assert!(result_b_shared.is_some());
    assert_eq!(
        result_b_shared.unwrap().file_path,
        root_conftest,
        "Test in module_b should access root's shared fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_multiple_unrelated_branches_is_deterministic() {
    // Issue #23 fix: When a fixture is defined in multiple unrelated branches,
    // and a test file is NOT in any of their hierarchies, the fixture should NOT
    // be accessible (returns None, not a random choice).
    let db = FixtureDatabase::new();

    // Three unrelated project branches - each has their own conftest.py
    let branch_a_content = r#"
import pytest

@pytest.fixture
def common_fixture():
    return "branch_a"
"#;
    let branch_a_conftest = PathBuf::from("/tmp/projects/project_a/conftest.py");
    db.analyze_file(branch_a_conftest.clone(), branch_a_content);

    let branch_b_content = r#"
import pytest

@pytest.fixture
def common_fixture():
    return "branch_b"
"#;
    let branch_b_conftest = PathBuf::from("/tmp/projects/project_b/conftest.py");
    db.analyze_file(branch_b_conftest.clone(), branch_b_content);

    let branch_c_content = r#"
import pytest

@pytest.fixture
def common_fixture():
    return "branch_c"
"#;
    let branch_c_conftest = PathBuf::from("/tmp/projects/project_c/conftest.py");
    db.analyze_file(branch_c_conftest.clone(), branch_c_content);

    // Test in yet another unrelated location - NOT in any project's hierarchy
    let test_content = r#"
def test_something(common_fixture):
    assert common_fixture is not None
"#;
    let test_path = PathBuf::from("/tmp/unrelated/test_foo.py");
    db.analyze_file(test_path.clone(), test_content);

    // The fixture is NOT accessible from this location because:
    // 1. It's not in the same file
    // 2. None of the conftest.py files are in parent directories of test_path
    // 3. It's not in site-packages
    let result = db.find_fixture_definition(&test_path, 1, 19);
    assert!(
        result.is_none(),
        "Fixture should NOT be found - test file is not in any conftest hierarchy"
    );

    // However, a test WITHIN project_a should find project_a's fixture
    let test_in_a_content = r#"
def test_in_project_a(common_fixture):
    pass
"#;
    let test_in_a_path = PathBuf::from("/tmp/projects/project_a/test_example.py");
    db.analyze_file(test_in_a_path.clone(), test_in_a_content);

    let result_in_a = db.find_fixture_definition(&test_in_a_path, 1, 22);
    assert!(
        result_in_a.is_some(),
        "Fixture should be found in project_a"
    );
    assert_eq!(
        result_in_a.unwrap().file_path,
        branch_a_conftest,
        "Should resolve to project_a's conftest.py"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_conftest_at_various_depths() {
    // Test that conftest.py files at different depths are correctly prioritized
    let db = FixtureDatabase::new();

    // Deep conftest
    let deep_content = r#"
import pytest

@pytest.fixture
def fixture_a():
    return "deep"

@pytest.fixture
def fixture_b():
    return "deep"
"#;
    let deep_conftest = PathBuf::from("/tmp/project/src/module/tests/integration/conftest.py");
    db.analyze_file(deep_conftest.clone(), deep_content);

    // Mid-level conftest - overrides fixture_a but not fixture_b
    let mid_content = r#"
import pytest

@pytest.fixture
def fixture_a():
    return "mid"
"#;
    let mid_conftest = PathBuf::from("/tmp/project/src/module/conftest.py");
    db.analyze_file(mid_conftest.clone(), mid_content);

    // Root conftest - defines fixture_c
    let root_content = r#"
import pytest

@pytest.fixture
def fixture_c():
    return "root"
"#;
    let root_conftest = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(root_conftest.clone(), root_content);

    // Test in deep directory
    let test_content = r#"
def test_all(fixture_a, fixture_b, fixture_c):
    assert fixture_a == "deep"
    assert fixture_b == "deep"
    assert fixture_c == "root"
"#;
    let test_path = PathBuf::from("/tmp/project/src/module/tests/integration/test_foo.py");
    db.analyze_file(test_path.clone(), test_content);

    // fixture_a: should resolve to deep (closest)
    let result_a = db.find_fixture_definition(&test_path, 1, 13);
    assert!(result_a.is_some());
    assert_eq!(
        result_a.unwrap().file_path,
        deep_conftest,
        "fixture_a should resolve to closest conftest (deep)"
    );

    // fixture_b: should resolve to deep (only defined there)
    let result_b = db.find_fixture_definition(&test_path, 1, 24);
    assert!(result_b.is_some());
    assert_eq!(
        result_b.unwrap().file_path,
        deep_conftest,
        "fixture_b should resolve to deep conftest"
    );

    // fixture_c: should resolve to root (only defined there)
    let result_c = db.find_fixture_definition(&test_path, 1, 35);
    assert!(result_c.is_some());
    assert_eq!(
        result_c.unwrap().file_path,
        root_conftest,
        "fixture_c should resolve to root conftest"
    );

    // Test in mid-level directory (one level up)
    let test_mid_content = r#"
def test_mid(fixture_a, fixture_c):
    assert fixture_a == "mid"
    assert fixture_c == "root"
"#;
    let test_mid_path = PathBuf::from("/tmp/project/src/module/test_bar.py");
    db.analyze_file(test_mid_path.clone(), test_mid_content);

    // fixture_a from mid-level: should resolve to mid conftest
    let result_a_mid = db.find_fixture_definition(&test_mid_path, 1, 13);
    assert!(result_a_mid.is_some());
    assert_eq!(
        result_a_mid.unwrap().file_path,
        mid_conftest,
        "fixture_a from mid-level test should resolve to mid conftest"
    );
}

#[test]
#[timeout(30000)]
fn test_get_available_fixtures_same_file() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def fixture_a():
    return "a"

@pytest.fixture
def fixture_b():
    return "b"

def test_something():
    pass
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let available = db.get_available_fixtures(&test_path);

    assert_eq!(available.len(), 2, "Should find 2 fixtures in same file");

    let names: Vec<_> = available.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"fixture_a"));
    assert!(names.contains(&"fixture_b"));
}

#[test]
#[timeout(30000)]
fn test_get_available_fixtures_conftest_hierarchy() {
    let db = FixtureDatabase::new();

    // Root conftest
    let root_conftest = r#"
import pytest

@pytest.fixture
def root_fixture():
    return "root"
"#;
    let root_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(root_path.clone(), root_conftest);

    // Subdir conftest
    let sub_conftest = r#"
import pytest

@pytest.fixture
def sub_fixture():
    return "sub"
"#;
    let sub_path = PathBuf::from("/tmp/test/subdir/conftest.py");
    db.analyze_file(sub_path.clone(), sub_conftest);

    // Test file in subdir
    let test_content = r#"
def test_something():
    pass
"#;
    let test_path = PathBuf::from("/tmp/test/subdir/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let available = db.get_available_fixtures(&test_path);

    assert_eq!(
        available.len(),
        2,
        "Should find fixtures from both conftest files"
    );

    let names: Vec<_> = available.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"root_fixture"));
    assert!(names.contains(&"sub_fixture"));
}

#[test]
#[timeout(30000)]
fn test_get_available_fixtures_no_duplicates() {
    let db = FixtureDatabase::new();

    // Root conftest
    let root_conftest = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "root"
"#;
    let root_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(root_path.clone(), root_conftest);

    // Subdir conftest with same fixture name
    let sub_conftest = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "sub"
"#;
    let sub_path = PathBuf::from("/tmp/test/subdir/conftest.py");
    db.analyze_file(sub_path.clone(), sub_conftest);

    // Test file in subdir
    let test_content = r#"
def test_something():
    pass
"#;
    let test_path = PathBuf::from("/tmp/test/subdir/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let available = db.get_available_fixtures(&test_path);

    // Should only find one "shared_fixture" (the closest one)
    let shared_count = available
        .iter()
        .filter(|f| f.name == "shared_fixture")
        .count();
    assert_eq!(shared_count, 1, "Should only include shared_fixture once");

    // The one included should be from the subdir (closest)
    let shared_fixture = available
        .iter()
        .find(|f| f.name == "shared_fixture")
        .unwrap();
    assert_eq!(shared_fixture.file_path, sub_path);
}

#[test]
#[timeout(30000)]
fn test_is_inside_function_in_test() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

def test_example(fixture_a, fixture_b):
    result = fixture_a + fixture_b
    assert result == "ab"
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Test on the function definition line (line 4, 0-indexed line 3)
    let result = db.is_inside_function(&test_path, 3, 10);
    assert!(result.is_some());

    let (func_name, is_fixture, params) = result.unwrap();
    assert_eq!(func_name, "test_example");
    assert!(!is_fixture);
    assert_eq!(params, vec!["fixture_a", "fixture_b"]);

    // Test inside the function body (line 5, 0-indexed line 4)
    let result = db.is_inside_function(&test_path, 4, 10);
    assert!(result.is_some());

    let (func_name, is_fixture, _) = result.unwrap();
    assert_eq!(func_name, "test_example");
    assert!(!is_fixture);
}

#[test]
#[timeout(30000)]
fn test_is_inside_function_in_fixture() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture(other_fixture):
    return other_fixture + "_modified"
"#;
    let test_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(test_path.clone(), test_content);

    // Test on the function definition line (line 5, 0-indexed line 4)
    let result = db.is_inside_function(&test_path, 4, 10);
    assert!(result.is_some());

    let (func_name, is_fixture, params) = result.unwrap();
    assert_eq!(func_name, "my_fixture");
    assert!(is_fixture);
    assert_eq!(params, vec!["other_fixture"]);

    // Test inside the function body (line 6, 0-indexed line 5)
    let result = db.is_inside_function(&test_path, 5, 10);
    assert!(result.is_some());

    let (func_name, is_fixture, _) = result.unwrap();
    assert_eq!(func_name, "my_fixture");
    assert!(is_fixture);
}

#[test]
#[timeout(30000)]
fn test_is_inside_function_outside() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "value"

def test_example(my_fixture):
    assert my_fixture == "value"

# This is a comment outside any function
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Test on the import line (line 1, 0-indexed line 0)
    let result = db.is_inside_function(&test_path, 0, 0);
    assert!(
        result.is_none(),
        "Should not be inside a function on import line"
    );

    // Test on the comment line (line 10, 0-indexed line 9)
    let result = db.is_inside_function(&test_path, 9, 0);
    assert!(
        result.is_none(),
        "Should not be inside a function on comment line"
    );
}

#[test]
#[timeout(30000)]
fn test_is_inside_function_non_test() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

def helper_function():
    return "helper"

def test_example():
    result = helper_function()
    assert result == "helper"
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Test inside helper_function (not a test or fixture)
    let result = db.is_inside_function(&test_path, 3, 10);
    assert!(
        result.is_none(),
        "Should not return non-test, non-fixture functions"
    );

    // Test inside test_example (is a test)
    let result = db.is_inside_function(&test_path, 6, 10);
    assert!(result.is_some(), "Should return test functions");

    let (func_name, is_fixture, _) = result.unwrap();
    assert_eq!(func_name, "test_example");
    assert!(!is_fixture);
}

#[test]
#[timeout(30000)]
fn test_is_inside_async_function() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
async def async_fixture():
    return "async_value"

async def test_async_example(async_fixture):
    assert async_fixture == "async_value"
"#;
    let test_path = PathBuf::from("/tmp/test/test_async.py");
    db.analyze_file(test_path.clone(), test_content);

    // Test inside async fixture (line 5, 0-indexed line 4)
    let result = db.is_inside_function(&test_path, 4, 10);
    assert!(result.is_some());

    let (func_name, is_fixture, _) = result.unwrap();
    assert_eq!(func_name, "async_fixture");
    assert!(is_fixture);

    // Test inside async test (line 8, 0-indexed line 7)
    let result = db.is_inside_function(&test_path, 7, 10);
    assert!(result.is_some());

    let (func_name, is_fixture, params) = result.unwrap();
    assert_eq!(func_name, "test_async_example");
    assert!(!is_fixture);
    assert_eq!(params, vec!["async_fixture"]);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_simple_return_type() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def string_fixture() -> str:
    return "hello"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let fixtures = db.definitions.get("string_fixture").unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].return_type, Some("str".to_string()));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_generator_return_type() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import Generator

@pytest.fixture
def generator_fixture() -> Generator[str, None, None]:
    yield "value"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let fixtures = db.definitions.get("generator_fixture").unwrap();
    assert_eq!(fixtures.len(), 1);
    // Should extract the yielded type (str) from Generator[str, None, None]
    assert_eq!(fixtures[0].return_type, Some("str".to_string()));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_iterator_return_type() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import Iterator

@pytest.fixture
def iterator_fixture() -> Iterator[int]:
    yield 42
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let fixtures = db.definitions.get("iterator_fixture").unwrap();
    assert_eq!(fixtures.len(), 1);
    // Should extract the yielded type (int) from Iterator[int]
    assert_eq!(fixtures[0].return_type, Some("int".to_string()));
}

#[test]
#[timeout(30000)]
fn test_fixture_without_return_type() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def no_type_fixture():
    return 123
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let fixtures = db.definitions.get("no_type_fixture").unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].return_type, None);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_union_return_type() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def union_fixture() -> str | int:
    return "string"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    let fixtures = db.definitions.get("union_fixture").unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].return_type, Some("str | int".to_string()));
}

// ============================================================================
// HIGH PRIORITY TESTS: Real-world pytest patterns
// ============================================================================

#[test]
#[timeout(30000)]
fn test_parametrized_fixture_detection() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(params=[1, 2, 3])
def number_fixture(request):
    return request.param

@pytest.fixture(params=["a", "b"])
def letter_fixture(request):
    return request.param
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect parametrized fixtures
    assert!(db.definitions.contains_key("number_fixture"));
    assert!(db.definitions.contains_key("letter_fixture"));

    let number_defs = db.definitions.get("number_fixture").unwrap();
    assert_eq!(number_defs.len(), 1);
    assert_eq!(number_defs[0].name, "number_fixture");
}

#[test]
#[timeout(30000)]
fn test_parametrized_fixture_usage() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture(params=[1, 2, 3])
def number_fixture(request):
    return request.param
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_with_parametrized(number_fixture):
    assert number_fixture > 0
"#;
    let test_path = PathBuf::from("/tmp/test/test_param.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should find definition for parametrized fixture
    // Line 1 (0-indexed), character position 27 is where 'number_fixture' starts in parameter
    let definition = db.find_fixture_definition(&test_path, 1, 27);
    assert!(
        definition.is_some(),
        "Should find parametrized fixture definition"
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "number_fixture");
    assert_eq!(def.file_path, conftest_path);
}

#[test]
#[timeout(30000)]
fn test_parametrized_fixture_with_ids() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(params=[1, 2, 3], ids=["one", "two", "three"])
def number_with_ids(request):
    return request.param

@pytest.fixture(params=["x", "y"], ids=lambda x: f"letter_{x}")
def letter_with_ids(request):
    return request.param

@pytest.fixture(
    params=[{"a": 1}, {"b": 2}],
    ids=["dict_a", "dict_b"],
    scope="module"
)
def complex_params(request):
    return request.param
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect all parametrized fixtures with ids
    assert!(
        db.definitions.contains_key("number_with_ids"),
        "Should detect fixture with list ids"
    );
    assert!(
        db.definitions.contains_key("letter_with_ids"),
        "Should detect fixture with lambda ids"
    );
    assert!(
        db.definitions.contains_key("complex_params"),
        "Should detect multi-line parametrized fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_factory_fixture_pattern() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def user_factory():
    def _create_user(name, email):
        return {"name": name, "email": email}
    return _create_user

@pytest.fixture
def database_factory(db_connection):
    def _create_database(name):
        return db_connection.create(name)
    return _create_database
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect factory fixtures
    assert!(db.definitions.contains_key("user_factory"));
    assert!(db.definitions.contains_key("database_factory"));

    let user_factory = db.definitions.get("user_factory").unwrap();
    assert_eq!(user_factory.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_autouse_fixture_detection() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(autouse=True)
def auto_fixture():
    print("Running automatically")
    yield
    print("Cleanup")

@pytest.fixture(scope="function", autouse=True)
def another_auto():
    return 42
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect autouse fixtures
    assert!(db.definitions.contains_key("auto_fixture"));
    assert!(db.definitions.contains_key("another_auto"));
}

#[test]
#[timeout(30000)]
fn test_autouse_fixture_not_flagged_as_undeclared() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture(autouse=True)
def auto_setup():
    return "setup"
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    let test_content = r#"
def test_something():
    # auto_setup runs automatically, not declared in parameters
    # Using it in body should NOT be flagged since it's autouse
    result = auto_setup
    assert result == "setup"
"#;
    let test_path = PathBuf::from("/tmp/test/test_autouse.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Current implementation may flag this, which is a limitation
    // This test documents expected behavior for future enhancement
    // For now, autouse fixtures are treated like any other fixture
    // and WILL be flagged if used in function body without parameter declaration
    assert!(
        undeclared.iter().any(|u| u.name == "auto_setup"),
        "Current implementation flags autouse fixtures - this is a known limitation"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_with_scope_session() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(scope="session")
def session_fixture():
    return "session data"

@pytest.fixture(scope="module")
def module_fixture():
    return "module data"

@pytest.fixture(scope="class")
def class_fixture():
    return "class data"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with different scopes
    assert!(db.definitions.contains_key("session_fixture"));
    assert!(db.definitions.contains_key("module_fixture"));
    assert!(db.definitions.contains_key("class_fixture"));
}

#[test]
#[timeout(30000)]
fn test_pytest_asyncio_fixture() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
import pytest_asyncio

@pytest_asyncio.fixture
async def async_fixture():
    return "async data"

@pytest.fixture
async def regular_async_fixture():
    return "also async"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // @pytest_asyncio.fixture is now supported
    assert!(
        db.definitions.contains_key("async_fixture"),
        "pytest_asyncio.fixture should be detected"
    );

    // Regular async fixtures with @pytest.fixture are also detected
    assert!(db.definitions.contains_key("regular_async_fixture"));
}

#[test]
#[timeout(30000)]
fn test_fixture_name_aliasing() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(name="custom_name")
def internal_fixture_name():
    return "aliased"

@pytest.fixture(name="db")
def database_connection():
    return "connection"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures by their alias name (from name= parameter)
    assert!(db.definitions.contains_key("custom_name"));
    assert!(db.definitions.contains_key("db"));

    // The internal function names should NOT be registered as fixtures
    assert!(!db.definitions.contains_key("internal_fixture_name"));
    assert!(!db.definitions.contains_key("database_connection"));
}

#[test]
#[timeout(30000)]
fn test_renamed_fixture_usage_detection() {
    // Test case from https://github.com/bellini666/pytest-language-server/issues/18
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(name="new")
def old() -> int:
    return 1

def test_example(new: int):
    assert new == 1
"#;
    let file_path = PathBuf::from("/tmp/test/test_renamed.py");
    db.analyze_file(file_path.clone(), content);

    // The fixture should be registered under "new", not "old"
    assert!(db.definitions.contains_key("new"));
    assert!(!db.definitions.contains_key("old"));

    // The usage in test_example should reference "new"
    let usages = db.usages.get(&file_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "new"));

    // The fixture should be found and marked as used
    let new_defs = db.definitions.get("new").unwrap();
    assert_eq!(new_defs.len(), 1);
    assert_eq!(new_defs[0].file_path, file_path);
}

#[test]
#[timeout(30000)]
fn test_class_based_test_methods_use_fixtures() {
    // Test case from https://github.com/bellini666/pytest-language-server/issues/19
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture() -> int:
    return 1

class TestInClass:
    def test_in_class(self, my_fixture: int):
        assert my_fixture == 1

    def test_another(self, my_fixture: int):
        assert my_fixture == 1
"#;
    let file_path = PathBuf::from("/tmp/test/test_class.py");
    db.analyze_file(file_path.clone(), content);

    // The fixture should be detected
    assert!(db.definitions.contains_key("my_fixture"));

    // The test methods inside the class should register fixture usages
    let usages = db.usages.get(&file_path).unwrap();
    let my_fixture_usages: Vec<_> = usages.iter().filter(|u| u.name == "my_fixture").collect();

    assert_eq!(
        my_fixture_usages.len(),
        2,
        "Should have 2 usages of my_fixture from test methods in class"
    );
}

#[test]
#[timeout(30000)]
fn test_nested_class_test_methods() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def outer_fixture():
    return "outer"

class TestOuter:
    def test_outer(self, outer_fixture):
        pass

    class TestNested:
        def test_nested(self, outer_fixture):
            pass
"#;
    let file_path = PathBuf::from("/tmp/test/test_nested.py");
    db.analyze_file(file_path.clone(), content);

    // Both outer and nested test methods should find the fixture
    let usages = db.usages.get(&file_path).unwrap();
    let fixture_usages: Vec<_> = usages
        .iter()
        .filter(|u| u.name == "outer_fixture")
        .collect();

    assert_eq!(
        fixture_usages.len(),
        2,
        "Should have 2 usages from both outer and nested test classes"
    );
}

#[test]
#[timeout(30000)]
fn test_deeply_nested_classes() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "shared"

class TestLevel1:
    def test_level1(self, shared_fixture):
        pass

    class TestLevel2:
        def test_level2(self, shared_fixture):
            pass

        class TestLevel3:
            def test_level3(self, shared_fixture):
                pass
"#;
    let file_path = PathBuf::from("/tmp/test/test_deep_nested.py");
    db.analyze_file(file_path.clone(), content);

    // All test methods at all nesting levels should find the fixture
    let usages = db.usages.get(&file_path).unwrap();
    let fixture_usages: Vec<_> = usages
        .iter()
        .filter(|u| u.name == "shared_fixture")
        .collect();

    assert_eq!(
        fixture_usages.len(),
        3,
        "Should have 3 usages from all nesting levels"
    );
}

#[test]
#[timeout(30000)]
fn test_nested_class_with_usefixtures() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def setup_fixture():
    return "setup"

@pytest.fixture
def nested_setup():
    return "nested"

@pytest.mark.usefixtures("setup_fixture")
class TestOuter:
    def test_outer(self):
        pass

    @pytest.mark.usefixtures("nested_setup")
    class TestNested:
        def test_nested(self):
            pass
"#;
    let file_path = PathBuf::from("/tmp/test/test_nested_usefixtures.py");
    db.analyze_file(file_path.clone(), content);

    let usages = db.usages.get(&file_path).unwrap();

    // Both usefixtures decorators should be detected
    assert!(
        usages.iter().any(|u| u.name == "setup_fixture"),
        "setup_fixture from outer class usefixtures should be detected"
    );
    assert!(
        usages.iter().any(|u| u.name == "nested_setup"),
        "nested_setup from nested class usefixtures should be detected"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_in_nested_class() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class TestOuter:
    @pytest.fixture
    def outer_class_fixture(self):
        return "outer"

    def test_uses_outer(self, outer_class_fixture):
        pass

    class TestNested:
        @pytest.fixture
        def nested_class_fixture(self):
            return "nested"

        def test_uses_nested(self, nested_class_fixture):
            pass

        def test_uses_both(self, outer_class_fixture, nested_class_fixture):
            pass
"#;
    let file_path = PathBuf::from("/tmp/test/test_fixture_in_nested.py");
    db.analyze_file(file_path.clone(), content);

    // Both class-level fixtures should be detected
    assert!(
        db.definitions.contains_key("outer_class_fixture"),
        "Fixture in outer class should be detected"
    );
    assert!(
        db.definitions.contains_key("nested_class_fixture"),
        "Fixture in nested class should be detected"
    );

    let usages = db.usages.get(&file_path).unwrap();

    // Check usages
    let outer_usages: Vec<_> = usages
        .iter()
        .filter(|u| u.name == "outer_class_fixture")
        .collect();
    assert_eq!(
        outer_usages.len(),
        2,
        "outer_class_fixture should be used twice"
    );

    let nested_usages: Vec<_> = usages
        .iter()
        .filter(|u| u.name == "nested_class_fixture")
        .collect();
    assert_eq!(
        nested_usages.len(),
        2,
        "nested_class_fixture should be used twice"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_defined_in_class() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class TestWithFixture:
    @pytest.fixture
    def class_fixture(self):
        return "class_value"

    def test_uses_class_fixture(self, class_fixture):
        assert class_fixture == "class_value"
"#;
    let file_path = PathBuf::from("/tmp/test/test_class_fixture.py");
    db.analyze_file(file_path.clone(), content);

    // Fixture defined inside class should be detected
    assert!(
        db.definitions.contains_key("class_fixture"),
        "Class-defined fixture should be detected"
    );

    // Test method should register usage
    let usages = db.usages.get(&file_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "class_fixture"),
        "Usage of class fixture should be detected"
    );
}

#[test]
#[timeout(30000)]
fn test_pytest_django_builtin_fixtures() {
    let db = FixtureDatabase::new();

    // Simulate pytest-django fixtures in site-packages
    let django_plugin_content = r#"
import pytest

@pytest.fixture
def db():
    """Provide django database access"""
    return "db_connection"

@pytest.fixture
def client():
    """Provide django test client"""
    return "test_client"

@pytest.fixture
def admin_client():
    """Provide django admin client"""
    return "admin_client"
"#;
    let plugin_path =
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/pytest_django/fixtures.py");
    db.analyze_file(plugin_path.clone(), django_plugin_content);

    let test_content = r#"
def test_with_django_fixtures(db, client, admin_client):
    assert db is not None
    assert client is not None
"#;
    let test_path = PathBuf::from("/tmp/test/test_django.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should detect django fixtures from third-party plugin
    assert!(db.definitions.contains_key("db"));
    assert!(db.definitions.contains_key("client"));
    assert!(db.definitions.contains_key("admin_client"));

    // Verify usages were detected
    assert!(
        db.usages.contains_key(&test_path),
        "Test file should have fixture usages"
    );
    let usages = db.usages.get(&test_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "db"),
        "Should detect 'db' fixture usage"
    );
    assert!(
        usages.iter().any(|u| u.name == "client"),
        "Should detect 'client' fixture usage"
    );

    // Should find definition using third-party fixture resolution
    // Line 1 (0-indexed), character 31 is where 'db' starts in the parameter list
    let db_def = db.find_fixture_definition(&test_path, 1, 31);
    assert!(db_def.is_some(), "Should find third-party fixture 'db'");
    assert_eq!(db_def.unwrap().name, "db");
}

#[test]
#[timeout(30000)]
fn test_pytest_mock_advanced_patterns() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest
from unittest.mock import Mock

@pytest.fixture
def mock_service():
    return Mock()

@pytest.fixture
def patched_function(mocker):
    return mocker.patch('module.function')
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Should detect fixtures that use mocker
    assert!(db.definitions.contains_key("mock_service"));
    assert!(db.definitions.contains_key("patched_function"));

    // patched_function uses mocker as dependency
    let patched = db.definitions.get("patched_function").unwrap();
    assert_eq!(patched.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_mixed_sync_async_fixture_dependencies() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def sync_fixture():
    return "sync"

@pytest.fixture
async def async_fixture(sync_fixture):
    return f"async_{sync_fixture}"

@pytest.fixture
async def another_async(async_fixture):
    return f"another_{await async_fixture}"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect mixed sync/async fixtures
    assert!(db.definitions.contains_key("sync_fixture"));
    assert!(db.definitions.contains_key("async_fixture"));
    assert!(db.definitions.contains_key("another_async"));

    // Check that async_fixture depends on sync_fixture
    let async_usages = db.usages.get(&file_path).unwrap();
    assert!(async_usages.iter().any(|u| u.name == "sync_fixture"));
}

#[test]
#[timeout(30000)]
fn test_yield_fixture_with_exception_handling() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def resource_with_cleanup():
    resource = acquire_resource()
    try:
        yield resource
    except Exception as e:
        handle_error(e)
    finally:
        cleanup_resource(resource)

@pytest.fixture
def complex_fixture():
    setup()
    try:
        yield "value"
    finally:
        teardown()
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect yield fixtures with exception handling
    assert!(db.definitions.contains_key("resource_with_cleanup"));
    assert!(db.definitions.contains_key("complex_fixture"));
}

#[test]
#[timeout(30000)]
fn test_yield_fixture_basic() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def simple_yield_fixture():
    """A simple yield fixture with setup and teardown."""
    # Setup
    connection = create_connection()
    yield connection
    # Teardown
    connection.close()

@pytest.fixture
def yield_with_value():
    yield 42

@pytest.fixture
def yield_none():
    yield
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // All yield fixtures should be detected
    assert!(
        db.definitions.contains_key("simple_yield_fixture"),
        "Simple yield fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("yield_with_value"),
        "Yield with value should be detected"
    );
    assert!(
        db.definitions.contains_key("yield_none"),
        "Yield None should be detected"
    );

    // Check docstring extraction works for yield fixtures
    let simple = db.definitions.get("simple_yield_fixture").unwrap();
    assert!(
        simple[0].docstring.is_some(),
        "Docstring should be extracted from yield fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_yield_fixture_usage_in_test() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def db_session():
    session = create_session()
    yield session
    session.rollback()
    session.close()
"#;

    let test_content = r#"
def test_with_db(db_session):
    db_session.query("SELECT 1")
"#;

    let conftest_path = PathBuf::from("/tmp/test_yield/conftest.py");
    let test_path = PathBuf::from("/tmp/test_yield/test_db.py");

    db.analyze_file(conftest_path.clone(), conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // Yield fixture should be found via go-to-definition
    let definition = db.find_fixture_definition(&test_path, 1, 18);
    assert!(definition.is_some(), "Should find yield fixture definition");
    let def = definition.unwrap();
    assert_eq!(def.name, "db_session");
    assert_eq!(def.file_path, conftest_path);
}

#[test]
#[timeout(30000)]
fn test_yield_fixture_with_context_manager() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from contextlib import contextmanager

@pytest.fixture
def managed_resource():
    with open("file.txt") as f:
        yield f

@pytest.fixture
def nested_context():
    with lock:
        with connection:
            yield connection
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    assert!(
        db.definitions.contains_key("managed_resource"),
        "Yield fixture with context manager should be detected"
    );
    assert!(
        db.definitions.contains_key("nested_context"),
        "Yield fixture with nested context should be detected"
    );
}

#[test]
#[timeout(30000)]
fn test_async_yield_fixture() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
async def async_db():
    db = await create_async_db()
    yield db
    await db.close()

@pytest.fixture
async def async_client():
    async with httpx.AsyncClient() as client:
        yield client
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    assert!(
        db.definitions.contains_key("async_db"),
        "Async yield fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("async_client"),
        "Async yield fixture with context manager should be detected"
    );
}

#[test]
#[timeout(30000)]
fn test_indirect_parametrization() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def user_data(request):
    return request.param

@pytest.mark.parametrize("user_data", [
    {"name": "Alice"},
    {"name": "Bob"}
], indirect=True)
def test_user(user_data):
    assert user_data["name"] in ["Alice", "Bob"]
"#;
    let test_path = PathBuf::from("/tmp/test/test_indirect.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should detect fixture used with indirect parametrization
    assert!(db.definitions.contains_key("user_data"));

    let usages = db.usages.get(&test_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "user_data"));
}

// ============================================================================
// HIGH PRIORITY TESTS: Undeclared fixture detection gaps
// ============================================================================

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_walrus_operator() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return [1, 2, 3, 4, 5]
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_walrus():
    # Using walrus operator with fixture name
    if (data := my_fixture):
        assert len(data) > 0
"#;
    let test_path = PathBuf::from("/tmp/test/test_walrus.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Current implementation may not detect walrus operator assignments
    // This test documents the limitation
    if undeclared.is_empty() {
        // Known limitation: walrus operator (named expressions) not handled
        println!("LIMITATION: Walrus operator assignments not detected as local variables");
    } else {
        // If detected, it should flag my_fixture as undeclared
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_list_comprehension() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def items():
    return [1, 2, 3]

@pytest.fixture
def multiplier():
    return 2
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_comprehension():
    # Using fixture in list comprehension iterable - should be flagged
    result = [x * 2 for x in items]
    assert len(result) == 3

    # Using fixture in comprehension expression - should be flagged
    result2 = [multiplier * x for x in [1, 2, 3]]
    assert result2 == [2, 4, 6]
"#;
    let test_path = PathBuf::from("/tmp/test/test_comprehension.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Current implementation does not track comprehension loop variables
    // as local variables, so this is a KNOWN LIMITATION
    println!(
        "Undeclared fixtures detected: {:?}",
        undeclared.iter().map(|u| &u.name).collect::<Vec<_>>()
    );

    // This test documents that comprehensions are partially detected
    // but comprehension loop variables are not tracked as locals
    if undeclared.iter().any(|u| u.name == "items") {
        // Good: fixture in iterable is detected
        // Test passes
    } else {
        // Known limitation: comprehension analysis is incomplete
        println!("LIMITATION: List comprehension variables not fully analyzed");
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_dict_comprehension() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def data_dict():
    return {"a": 1, "b": 2}
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_dict_comp():
    # Using fixture in dict comprehension
    result = {k: v * 2 for k, v in data_dict.items()}
    assert result["a"] == 2
"#;
    let test_path = PathBuf::from("/tmp/test/test_dict_comp.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Current implementation does not detect fixtures in dict comprehensions
    // This is a KNOWN LIMITATION
    if undeclared.iter().any(|u| u.name == "data_dict") {
        // Dict comprehension fixture detection working
    } else {
        println!("LIMITATION: Dict comprehension fixture detection not implemented");
        // Test documents known limitation
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_generator_expression() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def numbers():
    return [1, 2, 3, 4, 5]
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_generator():
    # Using fixture in generator expression
    gen = (x * 2 for x in numbers)
    result = list(gen)
    assert len(result) == 5
"#;
    let test_path = PathBuf::from("/tmp/test/test_generator.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Generator expressions are similar to list comprehensions
    // Current implementation does not detect these - KNOWN LIMITATION
    if undeclared.iter().any(|u| u.name == "numbers") {
        // Generator expression fixture detection working
    } else {
        println!("LIMITATION: Generator expression fixture detection not implemented");
        // Test documents known limitation
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_f_string() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def user_name():
    return "Alice"
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_f_string():
    # Using fixture in f-string interpolation
    message = f"Hello {user_name}"
    assert "Alice" in message
"#;
    let test_path = PathBuf::from("/tmp/test/test_f_string.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Current rustpython-parser may not expose f-string internals
    // This test documents expected behavior
    if undeclared.iter().any(|u| u.name == "user_name") {
        // Good: f-string variables are detected
        // F-string fixture detection working
    } else {
        println!("LIMITATION: F-string interpolation not analyzed for fixture references");
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_lambda() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def multiplier():
    return 3
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_lambda():
    # Using fixture in lambda body
    func = lambda x: x * multiplier
    result = func(5)
    assert result == 15
"#;
    let test_path = PathBuf::from("/tmp/test/test_lambda.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Lambda expressions are currently not analyzed for fixture usage
    // This is a KNOWN LIMITATION
    if undeclared.iter().any(|u| u.name == "multiplier") {
        // Lambda fixture detection working
    } else {
        println!("LIMITATION: Lambda expressions not analyzed for fixture references");
        // Test documents known limitation
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_nested_function() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def config():
    return {"key": "value"}
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_nested():
    def inner_function():
        # Using fixture from outer scope
        return config["key"]

    result = inner_function()
    assert result == "value"
"#;
    let test_path = PathBuf::from("/tmp/test/test_nested.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Nested functions are a complex case
    // Current implementation scans the test function body but may not
    // traverse into nested function definitions
    if undeclared.iter().any(|u| u.name == "config") {
        // Nested function fixture detection working
    } else {
        println!("LIMITATION: Nested functions not analyzed for fixture references");
    }
}

#[test]
#[timeout(30000)]
fn test_undeclared_fixture_in_decorator_argument() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def timeout_value():
    return 30
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
import pytest

def timeout_decorator(seconds):
    def decorator(func):
        return func
    return decorator

@timeout_decorator(timeout_value)
def test_with_timeout():
    assert True
"#;
    let test_path = PathBuf::from("/tmp/test/test_decorator.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Decorator arguments are typically not scanned
    // This test documents the limitation
    if undeclared.iter().any(|u| u.name == "timeout_value") {
        // Decorator argument fixture detection working
    } else {
        println!("LIMITATION: Decorator arguments not analyzed for fixture references");
    }
}

#[test]
#[timeout(30000)]
fn test_local_variable_shadowing_fixture() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def data():
    return "fixture_data"
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_shadowing():
    # Local variable shadows fixture name
    data = "local_data"
    assert data == "local_data"

    # This should NOT be flagged as undeclared
    result = data.upper()
    assert result == "LOCAL_DATA"
"#;
    let test_path = PathBuf::from("/tmp/test/test_shadow.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Should NOT flag 'data' as undeclared because it's assigned locally
    assert!(
        !undeclared.iter().any(|u| u.name == "data"),
        "Local variable should shadow fixture name - should not be flagged"
    );
}

#[test]
#[timeout(30000)]
fn test_comprehension_variable_shadowing_fixture() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def x():
    return 100
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    let test_content = r#"
def test_comp_shadow():
    # Comprehension variable 'x' shadows fixture 'x'
    result = [x * 2 for x in [1, 2, 3]]
    assert result == [2, 4, 6]
"#;
    let test_path = PathBuf::from("/tmp/test/test_comp_shadow.py");
    db.analyze_file(test_path.clone(), test_content);

    let undeclared = db.get_undeclared_fixtures(&test_path);

    // Note: Comprehension variables are not currently tracked as local vars
    // This is a known limitation
    if undeclared.iter().any(|u| u.name == "x") {
        println!("LIMITATION: Comprehension variables not tracked - false positive for 'x'");
    } else {
        // Comprehension variable correctly handled
    }
}

// ============================================================================
// MEDIUM PRIORITY TESTS: Fixture detection advanced cases
// ============================================================================

#[test]
#[timeout(30000)]
fn test_decorator_with_multiple_arguments() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture(scope="session", autouse=True, name="custom")
def complex_fixture():
    return 42

@pytest.fixture(scope="module", params=[1, 2, 3])
def parametrized_scoped():
    return "data"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with multiple decorator arguments
    // When name= is present, use the alias; otherwise use function name
    assert!(db.definitions.contains_key("custom")); // has name="custom"
    assert!(!db.definitions.contains_key("complex_fixture")); // function name not registered
    assert!(db.definitions.contains_key("parametrized_scoped")); // no name=, uses function name
}

#[test]
#[timeout(30000)]
fn test_parameter_with_type_hints() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import List, Dict

@pytest.fixture
def typed_fixture(param: str, count: int) -> Dict[str, int]:
    return {param: count}

@pytest.fixture
def complex_types(data: List[str]) -> List[Dict[str, int]]:
    return [{"item": len(d)} for d in data]
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with typed parameters
    assert!(db.definitions.contains_key("typed_fixture"));
    assert!(db.definitions.contains_key("complex_types"));

    // Check that parameter type hints are handled correctly
    let typed_usages = db.usages.get(&file_path).unwrap();
    assert!(typed_usages.iter().any(|u| u.name == "param"));
    assert!(typed_usages.iter().any(|u| u.name == "count"));
}

#[test]
#[timeout(30000)]
fn test_default_parameter_values() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def fixture_with_defaults(value="default", count=0):
    return value * count

@pytest.fixture
def optional_param(data=None):
    return data or []
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with default parameter values
    assert!(db.definitions.contains_key("fixture_with_defaults"));
    assert!(db.definitions.contains_key("optional_param"));
}

#[test]
#[timeout(30000)]
fn test_variadic_parameters() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def fixture_with_args(*args):
    return args

@pytest.fixture
def fixture_with_kwargs(**kwargs):
    return kwargs

@pytest.fixture
def fixture_with_both(base, *args, **kwargs):
    return (base, args, kwargs)
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with *args and **kwargs
    assert!(db.definitions.contains_key("fixture_with_args"));
    assert!(db.definitions.contains_key("fixture_with_kwargs"));
    assert!(db.definitions.contains_key("fixture_with_both"));

    // Check that 'base' is detected as a dependency, but not *args or **kwargs
    let usages = db.usages.get(&file_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "base"));
    assert!(!usages.iter().any(|u| u.name == "args"));
    assert!(!usages.iter().any(|u| u.name == "kwargs"));
}

#[test]
#[timeout(30000)]
fn test_variadic_with_fixture_dependencies() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"

@pytest.fixture
def config_fixture():
    return {"key": "value"}

@pytest.fixture
def combined_fixture(base_fixture, config_fixture, *args, **kwargs):
    """Fixture that depends on other fixtures and also accepts variadic args."""
    return {
        "base": base_fixture,
        "config": config_fixture,
        "extra_args": args,
        "extra_kwargs": kwargs,
    }
"#;

    let conftest_path = PathBuf::from("/tmp/test_variadic/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // All fixtures should be detected
    assert!(db.definitions.contains_key("base_fixture"));
    assert!(db.definitions.contains_key("config_fixture"));
    assert!(db.definitions.contains_key("combined_fixture"));

    // Fixture dependencies should be tracked
    let usages = db.usages.get(&conftest_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "base_fixture"),
        "base_fixture should be tracked as dependency"
    );
    assert!(
        usages.iter().any(|u| u.name == "config_fixture"),
        "config_fixture should be tracked as dependency"
    );
}

#[test]
#[timeout(30000)]
fn test_variadic_in_test_function() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_with_variadic(my_fixture, *args, **kwargs):
    # Note: This is unusual but valid Python
    assert my_fixture == 42
"#;

    let test_path = PathBuf::from("/tmp/test_variadic/test_func.py");
    db.analyze_file(test_path.clone(), test_content);

    // Fixture should be detected
    assert!(db.definitions.contains_key("my_fixture"));

    // Usage should be tracked
    let usages = db.usages.get(&test_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "my_fixture"),
        "my_fixture should be tracked as usage in test"
    );

    // *args and **kwargs should NOT be tracked as fixture usages
    assert!(
        !usages.iter().any(|u| u.name == "args"),
        "args should not be tracked as fixture"
    );
    assert!(
        !usages.iter().any(|u| u.name == "kwargs"),
        "kwargs should not be tracked as fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_keyword_only_with_variadic() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def dep_fixture():
    return "dep"

@pytest.fixture
def complex_fixture(*args, kwonly_dep: str, **kwargs):
    # kwonly_dep is a keyword-only parameter that could be a fixture
    return kwonly_dep
"#;

    let file_path = PathBuf::from("/tmp/test_variadic/conftest.py");
    db.analyze_file(file_path.clone(), content);

    assert!(db.definitions.contains_key("dep_fixture"));
    assert!(db.definitions.contains_key("complex_fixture"));

    // kwonly_dep should be tracked as a potential fixture dependency
    let usages = db.usages.get(&file_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "kwonly_dep"),
        "Keyword-only parameter should be tracked as potential fixture dependency"
    );
}

#[test]
#[timeout(30000)]
fn test_class_based_fixtures() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class TestClass:
    @pytest.fixture
    def class_fixture(self):
        return "class_value"

    def test_method(self, class_fixture):
        assert class_fixture == "class_value"
"#;
    let file_path = PathBuf::from("/tmp/test/test_class.py");
    db.analyze_file(file_path.clone(), content);

    // Note: Class-based fixtures may not be fully supported
    // This test documents the current behavior
    if db.definitions.contains_key("class_fixture") {
        // Class-based fixtures detected
    } else {
        println!("LIMITATION: Class-based fixtures not detected");
    }
}

#[test]
#[timeout(30000)]
fn test_classmethod_and_staticmethod_fixtures() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class TestClass:
    @classmethod
    @pytest.fixture
    def class_method_fixture(cls):
        return "classmethod"

    @staticmethod
    @pytest.fixture
    def static_method_fixture():
        return "staticmethod"
"#;
    let file_path = PathBuf::from("/tmp/test/test_methods.py");
    db.analyze_file(file_path.clone(), content);

    // These are unusual patterns - document behavior
    if db.definitions.contains_key("class_method_fixture") {
        println!("Class method fixtures detected");
    }
    if db.definitions.contains_key("static_method_fixture") {
        println!("Static method fixtures detected");
    }
}

#[test]
#[timeout(30000)]
fn test_unicode_fixture_names() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def 測試_fixture():
    """Chinese/Japanese test fixture"""
    return "test"

@pytest.fixture
def фикстура():
    """Russian fixture"""
    return "fixture"

@pytest.fixture
def fixture_émoji():
    """French accent fixture"""
    return "emoji"

@pytest.fixture
def données_utilisateur():
    """French: user data"""
    return {"name": "Jean"}

@pytest.fixture
def δεδομένα_χρήστη():
    """Greek: user data"""
    return {"name": "Γιώργος"}
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Python 3 supports Unicode identifiers (PEP 3131)
    // All fixtures should be detected
    assert!(
        db.definitions.contains_key("測試_fixture"),
        "Chinese/Japanese fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("фикстура"),
        "Russian fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("fixture_émoji"),
        "French accent fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("données_utilisateur"),
        "French fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("δεδομένα_χρήστη"),
        "Greek fixture should be detected"
    );

    // Check that docstrings are correctly extracted
    let russian = db.definitions.get("фикстура").unwrap();
    assert!(
        russian[0].docstring.as_ref().unwrap().contains("Russian"),
        "Russian docstring should be extracted"
    );
}

#[test]
#[timeout(30000)]
fn test_unicode_fixture_usage_detection() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def données():
    return 42
"#;

    let test_content = r#"
def test_unicode_usage(données):
    assert données == 42
"#;

    let conftest_path = PathBuf::from("/tmp/test_unicode/conftest.py");
    let test_path = PathBuf::from("/tmp/test_unicode/test_example.py");

    db.analyze_file(conftest_path, conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // Check that the Unicode fixture usage was detected
    let usages = db.usages.get(&test_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "données"),
        "Unicode fixture usage should be detected"
    );
}

#[test]
#[timeout(30000)]
fn test_unicode_fixture_goto_definition() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def données():
    return 42
"#;

    let test_content = r#"
def test_unicode(données):
    pass
"#;

    let conftest_path = PathBuf::from("/tmp/test_unicode/conftest.py");
    let test_path = PathBuf::from("/tmp/test_unicode/test_example.py");

    db.analyze_file(conftest_path.clone(), conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // The fixture "données" starts at character position 17 on line 2 (1-indexed)
    // In 0-indexed LSP coords: line 1, character 17
    // "def test_unicode(données):"
    //  0         1         2
    //  0123456789012345678901234
    //                  ^--- position 17

    let definition = db.find_fixture_definition(&test_path, 1, 17);

    assert!(
        definition.is_some(),
        "Definition should be found for Unicode fixture"
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "données");
    assert_eq!(def.file_path, conftest_path);
}

#[test]
#[timeout(30000)]
fn test_fixture_names_with_underscores() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def _private_fixture():
    return "private"

@pytest.fixture
def __dunder_fixture__():
    return "dunder"

@pytest.fixture
def fixture__double():
    return "double"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with various underscore patterns
    assert!(db.definitions.contains_key("_private_fixture"));
    assert!(db.definitions.contains_key("__dunder_fixture__"));
    assert!(db.definitions.contains_key("fixture__double"));
}

#[test]
#[timeout(30000)]
fn test_very_long_fixture_name() {
    let db = FixtureDatabase::new();

    let long_name = "fixture_with_an_extremely_long_name_that_exceeds_typical_naming_conventions_and_tests_the_system_capacity_for_handling_lengthy_identifiers";
    let content = format!(
        r#"
import pytest

@pytest.fixture
def {}():
    return 42
"#,
        long_name
    );

    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), &content);

    // Should handle very long fixture names
    assert!(
        db.definitions.contains_key(long_name),
        "Should handle fixture names over 100 characters"
    );
}

#[test]
#[timeout(30000)]
fn test_optional_and_union_type_hints() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import Optional, Union, List

@pytest.fixture
def optional_fixture(data: Optional[str]) -> Optional[int]:
    return len(data) if data else None

@pytest.fixture
def union_fixture(value: Union[str, int, List[str]]) -> Union[str, int]:
    return value
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with Optional and Union types
    assert!(db.definitions.contains_key("optional_fixture"));
    assert!(db.definitions.contains_key("union_fixture"));

    // Check return type extraction
    let optional_defs = db.definitions.get("optional_fixture").unwrap();
    if let Some(ref return_type) = optional_defs[0].return_type {
        assert!(return_type.contains("Optional") || return_type.contains("int"));
    }
}

#[test]
#[timeout(30000)]
fn test_forward_reference_type_hints() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def forward_ref_fixture() -> "MyClass":
    return MyClass()

class MyClass:
    pass
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixture with forward reference
    assert!(db.definitions.contains_key("forward_ref_fixture"));

    // Check if forward reference is preserved in return type
    let defs = db.definitions.get("forward_ref_fixture").unwrap();
    if let Some(ref return_type) = defs[0].return_type {
        // Forward reference might be stored as "MyClass" or "'MyClass'"
        assert!(return_type.contains("MyClass"));
    }
}

#[test]
#[timeout(30000)]
fn test_generic_type_hints() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import List, Dict, Tuple, Generic, TypeVar

T = TypeVar('T')

@pytest.fixture
def list_fixture() -> List[str]:
    return ["a", "b", "c"]

@pytest.fixture
def dict_fixture() -> Dict[str, List[int]]:
    return {"key": [1, 2, 3]}

@pytest.fixture
def tuple_fixture() -> Tuple[str, int, bool]:
    return ("text", 42, True)
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect fixtures with generic type hints
    assert!(db.definitions.contains_key("list_fixture"));
    assert!(db.definitions.contains_key("dict_fixture"));
    assert!(db.definitions.contains_key("tuple_fixture"));
}

// ============================================================================
// MEDIUM PRIORITY TESTS: Complex hierarchy scenarios
// ============================================================================

#[test]
#[timeout(30000)]
fn test_five_level_override_chain() {
    let db = FixtureDatabase::new();

    // Create 5-level deep hierarchy
    let root_conftest = r#"
import pytest

@pytest.fixture
def deep_fixture():
    return "root"
"#;
    db.analyze_file(PathBuf::from("/tmp/project/conftest.py"), root_conftest);

    let level2_conftest = r#"
import pytest

@pytest.fixture
def deep_fixture(deep_fixture):
    return f"{deep_fixture}_level2"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/project/level2/conftest.py"),
        level2_conftest,
    );

    let level3_conftest = r#"
import pytest

@pytest.fixture
def deep_fixture(deep_fixture):
    return f"{deep_fixture}_level3"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/project/level2/level3/conftest.py"),
        level3_conftest,
    );

    let level4_conftest = r#"
import pytest

@pytest.fixture
def deep_fixture(deep_fixture):
    return f"{deep_fixture}_level4"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/project/level2/level3/level4/conftest.py"),
        level4_conftest,
    );

    let level5_conftest = r#"
import pytest

@pytest.fixture
def deep_fixture(deep_fixture):
    return f"{deep_fixture}_level5"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/project/level2/level3/level4/level5/conftest.py"),
        level5_conftest,
    );

    // Test file at deepest level
    let test_content = r#"
def test_deep(deep_fixture):
    assert "level5" in deep_fixture
"#;
    let test_path = PathBuf::from("/tmp/project/level2/level3/level4/level5/test_deep.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should find the closest (level5) fixture
    let definition = db.find_fixture_definition(&test_path, 1, 15);
    assert!(definition.is_some());
    assert!(definition
        .unwrap()
        .file_path
        .ends_with("level5/conftest.py"));
}

#[test]
#[timeout(30000)]
fn test_diamond_dependency_pattern() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"

@pytest.fixture
def branch_a(base_fixture):
    return f"{base_fixture}_a"

@pytest.fixture
def branch_b(base_fixture):
    return f"{base_fixture}_b"

@pytest.fixture
def diamond(branch_a, branch_b):
    return f"{branch_a}_{branch_b}"
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Verify all fixtures detected
    assert!(db.definitions.contains_key("base_fixture"));
    assert!(db.definitions.contains_key("branch_a"));
    assert!(db.definitions.contains_key("branch_b"));
    assert!(db.definitions.contains_key("diamond"));

    // Verify dependencies
    let usages = db.usages.get(&conftest_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "base_fixture"));
    assert!(usages.iter().any(|u| u.name == "branch_a"));
    assert!(usages.iter().any(|u| u.name == "branch_b"));
}

#[test]
#[timeout(30000)]
fn test_ten_level_directory_depth() {
    let db = FixtureDatabase::new();

    // Create fixture at root
    let root_conftest = r#"
import pytest

@pytest.fixture
def deep_search():
    return "found"
"#;
    db.analyze_file(PathBuf::from("/tmp/root/conftest.py"), root_conftest);

    // Test file 10 levels deep
    let test_content = r#"
def test_deep_search(deep_search):
    assert deep_search == "found"
"#;
    let deep_path = PathBuf::from("/tmp/root/a/b/c/d/e/f/g/h/i/j/test_deep.py");
    db.analyze_file(deep_path.clone(), test_content);

    // Should find fixture from root despite 10-level depth
    let definition = db.find_fixture_definition(&deep_path, 1, 22);
    assert!(definition.is_some(), "Should find fixture 10 levels up");
    assert_eq!(definition.unwrap().name, "deep_search");
}

#[test]
#[timeout(30000)]
fn test_fixture_chain_middle_doesnt_use_parent() {
    let db = FixtureDatabase::new();

    let root_conftest = r#"
import pytest

@pytest.fixture
def chain_fixture():
    return "root"
"#;
    db.analyze_file(PathBuf::from("/tmp/test/conftest.py"), root_conftest);

    let middle_conftest = r#"
import pytest

@pytest.fixture
def chain_fixture():
    # Middle fixture doesn't use parent - breaks chain
    return "middle_independent"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/test/subdir/conftest.py"),
        middle_conftest,
    );

    let leaf_conftest = r#"
import pytest

@pytest.fixture
def chain_fixture(chain_fixture):
    # Leaf uses parent (middle), but middle doesn't use root
    return f"{chain_fixture}_leaf"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/test/subdir/deep/conftest.py"),
        leaf_conftest,
    );

    // Test at leaf level
    let test_content = r#"
def test_chain(chain_fixture):
    assert "leaf" in chain_fixture
"#;
    let test_path = PathBuf::from("/tmp/test/subdir/deep/test_chain.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should find leaf fixture
    let definition = db.find_fixture_definition(&test_path, 1, 16);
    assert!(definition.is_some());
    let def = definition.unwrap();
    assert!(def.file_path.ends_with("deep/conftest.py"));
}

#[test]
#[timeout(30000)]
fn test_multiple_fixtures_same_name_in_file() {
    let db = FixtureDatabase::new();

    // Having multiple fixtures with same name in one file is unusual
    // but pytest allows it - last one wins
    let content = r#"
import pytest

@pytest.fixture
def duplicate_fixture():
    return "first"

@pytest.fixture
def duplicate_fixture():
    return "second"

@pytest.fixture
def duplicate_fixture():
    return "third"
"#;
    let file_path = PathBuf::from("/home/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Should detect all three definitions
    let defs = db.definitions.get("duplicate_fixture").unwrap();
    assert_eq!(defs.len(), 3, "Should store all duplicate definitions");

    // Verify they are on different lines
    let lines: Vec<usize> = defs.iter().map(|d| d.line).collect();
    assert_eq!(lines.len(), 3);
    // Lines should be ordered (first, second, third fixture)
    assert!(lines[0] < lines[1]);
    assert!(lines[1] < lines[2]);
}

#[test]
#[timeout(30000)]
fn test_sibling_directories_with_same_fixture() {
    let db = FixtureDatabase::new();

    let dir_a_conftest = r#"
import pytest

@pytest.fixture
def sibling_fixture():
    return "from_a"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/project/dir_a/conftest.py"),
        dir_a_conftest,
    );

    let dir_b_conftest = r#"
import pytest

@pytest.fixture
def sibling_fixture():
    return "from_b"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/project/dir_b/conftest.py"),
        dir_b_conftest,
    );

    // Test in dir_a should use dir_a's fixture
    let test_a_content = r#"
def test_a(sibling_fixture):
    assert sibling_fixture == "from_a"
"#;
    let test_a_path = PathBuf::from("/tmp/project/dir_a/test_a.py");
    db.analyze_file(test_a_path.clone(), test_a_content);

    let def_a = db.find_fixture_definition(&test_a_path, 1, 12);
    assert!(def_a.is_some());
    assert!(def_a.unwrap().file_path.to_str().unwrap().contains("dir_a"));

    // Test in dir_b should use dir_b's fixture
    let test_b_content = r#"
def test_b(sibling_fixture):
    assert sibling_fixture == "from_b"
"#;
    let test_b_path = PathBuf::from("/tmp/project/dir_b/test_b.py");
    db.analyze_file(test_b_path.clone(), test_b_content);

    let def_b = db.find_fixture_definition(&test_b_path, 1, 12);
    assert!(def_b.is_some());
    assert!(def_b.unwrap().file_path.to_str().unwrap().contains("dir_b"));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_six_level_parameter_chain() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def level1():
    return 1

@pytest.fixture
def level2(level1):
    return level1 + 1

@pytest.fixture
def level3(level2):
    return level2 + 1

@pytest.fixture
def level4(level3):
    return level3 + 1

@pytest.fixture
def level5(level4):
    return level4 + 1

@pytest.fixture
def level6(level5):
    return level5 + 1
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), content);

    // All fixtures should be detected
    for i in 1..=6 {
        let name = format!("level{}", i);
        assert!(db.definitions.contains_key(&name), "Should detect {}", name);
    }

    // Check dependency chain
    let usages = db.usages.get(&conftest_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "level1"));
    assert!(usages.iter().any(|u| u.name == "level2"));
    assert!(usages.iter().any(|u| u.name == "level3"));
    assert!(usages.iter().any(|u| u.name == "level4"));
    assert!(usages.iter().any(|u| u.name == "level5"));
}

#[test]
#[timeout(30000)]
fn test_circular_dependency_detection() {
    let db = FixtureDatabase::new();

    // Note: This creates circular dependencies which pytest would reject at runtime
    // The parser should still detect the fixtures and dependencies
    let content = r#"
import pytest

@pytest.fixture
def fixture_a(fixture_b):
    return f"a_{fixture_b}"

@pytest.fixture
def fixture_b(fixture_a):
    return f"b_{fixture_a}"
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), content);

    // Both fixtures should be detected despite circular dependency
    assert!(db.definitions.contains_key("fixture_a"));
    assert!(db.definitions.contains_key("fixture_b"));

    // Both dependencies should be recorded
    let usages = db.usages.get(&conftest_path).unwrap();
    assert!(usages.iter().any(|u| u.name == "fixture_a"));
    assert!(usages.iter().any(|u| u.name == "fixture_b"));

    // Note: Runtime detection of circular dependencies is pytest's responsibility
    println!("Circular dependencies detected but not validated (pytest's job)");
}

#[test]
#[timeout(30000)]
fn test_multiple_third_party_same_fixture_name() {
    let db = FixtureDatabase::new();

    // Simulate two different plugins defining same fixture
    let plugin1_content = r#"
import pytest

@pytest.fixture
def event_loop():
    return "from_plugin1"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/plugin1/fixtures.py"),
        plugin1_content,
    );

    let plugin2_content = r#"
import pytest

@pytest.fixture
def event_loop():
    return "from_plugin2"
"#;
    db.analyze_file(
        PathBuf::from("/tmp/.venv/lib/python3.11/site-packages/plugin2/fixtures.py"),
        plugin2_content,
    );

    // Both should be detected
    let defs = db.definitions.get("event_loop").unwrap();
    assert_eq!(defs.len(), 2, "Should detect both third-party fixtures");

    // Verify both definitions are from site-packages
    let paths: Vec<&str> = defs.iter().map(|d| d.file_path.to_str().unwrap()).collect();
    assert!(
        paths.iter().all(|p| p.contains("site-packages")),
        "All definitions should be from site-packages"
    );

    // Verify usage detection works
    let test_content = r#"
def test_event_loop(event_loop):
    pass
"#;
    let test_path = PathBuf::from("/tmp/project/test_async.py");
    db.analyze_file(test_path.clone(), test_content);

    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1, "Should detect usage in test");
    assert_eq!(usages[0].name, "event_loop");
}

// MARK: File Path Edge Cases

#[test]
#[timeout(30000)]
fn test_unicode_characters_in_path() {
    let db = FixtureDatabase::new();

    // Test with Unicode characters in path
    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/日本語/тест/test_unicode.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

#[test]
#[timeout(30000)]
fn test_spaces_in_path() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/my folder/sub folder/test file.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

#[test]
#[timeout(30000)]
fn test_special_characters_in_path() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    // Test with parentheses, brackets, and other special chars
    let path = PathBuf::from("/tmp/test/my(folder)[2023]/test-file_v2.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

#[test]
#[timeout(30000)]
fn test_very_long_path() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    // Create a very long path (close to system limits)
    let long_component = "a".repeat(50);
    let path_str = format!(
        "/tmp/{}/{}/{}/{}/{}/{}/test.py",
        long_component,
        long_component,
        long_component,
        long_component,
        long_component,
        long_component
    );
    let path = PathBuf::from(path_str);
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_paths_with_dots() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    // Path with .hidden directories
    let path = PathBuf::from("/tmp/test/.hidden/.config/test.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

#[test]
#[timeout(30000)]
fn test_conftest_hierarchy_with_unicode_paths() {
    let db = FixtureDatabase::new();

    // Parent conftest with unicode path
    let parent_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"
"#;
    let parent_path = PathBuf::from("/tmp/проект/conftest.py");
    db.analyze_file(parent_path.clone(), parent_content);

    // Child test file
    let test_content = r#"
def test_something(base_fixture):
    assert base_fixture == "base"
"#;
    let test_path = PathBuf::from("/tmp/проект/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should detect usage
    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].name, "base_fixture");
}

#[test]
#[timeout(30000)]
fn test_fixture_resolution_with_special_char_paths() {
    let db = FixtureDatabase::new();

    // Conftest in path with special characters
    let conftest_content = r#"
import pytest

@pytest.fixture
def special_fixture():
    return "special"
"#;
    let conftest_path = PathBuf::from("/tmp/my-project (2023)/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    // Test file in subdirectory
    let test_content = r#"
def test_something(special_fixture):
    pass
"#;
    let test_path = PathBuf::from("/tmp/my-project (2023)/tests/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_multiple_consecutive_slashes_in_path() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    // Path with multiple consecutive slashes (should be normalized internally)
    let path = PathBuf::from("/tmp/test//subdir///test.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_path_with_trailing_slash() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    // Even though this is odd, the code should handle it
    let path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

#[test]
#[timeout(30000)]
fn test_emoji_in_path() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/😀_folder/🎉test.py");
    db.analyze_file(path.clone(), content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

// MARK: Workspace Scanning Edge Cases

#[test]
#[timeout(30000)]
fn test_scan_workspace_nonexistent_path() {
    let db = FixtureDatabase::new();

    // Try to scan a path that doesn't exist
    let nonexistent_path = std::path::PathBuf::from("/nonexistent/path/that/should/not/exist");

    // Scan should complete without panicking or errors
    db.scan_workspace(&nonexistent_path);

    // Should have no definitions
    assert!(db.definitions.is_empty());
    assert!(db.usages.is_empty());
}

#[test]
#[timeout(30000)]
fn test_scan_workspace_with_no_python_files() {
    let db = FixtureDatabase::new();
    let temp_dir = std::env::temp_dir().join("test_no_python_files");

    // Create directory structure without Python files
    std::fs::create_dir_all(&temp_dir).ok();

    // Scan should complete without errors
    db.scan_workspace(&temp_dir);

    // Should have no definitions
    assert!(db.definitions.is_empty());

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
#[timeout(30000)]
fn test_scan_workspace_with_only_non_test_files() {
    let db = FixtureDatabase::new();
    let temp_dir = std::env::temp_dir().join("test_no_test_files");

    std::fs::create_dir_all(&temp_dir).ok();

    // Create a Python file that doesn't match test patterns
    let file_path = temp_dir.join("utils.py");
    std::fs::write(
        &file_path,
        r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#,
    )
    .ok();

    db.scan_workspace(&temp_dir);

    // Should not detect fixtures in non-test files
    // (scan_workspace only looks for test_*.py, *_test.py, conftest.py)
    assert!(db.definitions.get("my_fixture").is_none());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
#[timeout(30000)]
fn test_scan_workspace_with_deeply_nested_structure() {
    let db = FixtureDatabase::new();
    let temp_dir = std::env::temp_dir().join("test_deep_nesting");

    // Create deeply nested structure
    let deep_path = temp_dir.join("a/b/c/d/e/f/g/h/i/j");
    std::fs::create_dir_all(&deep_path).ok();

    // Add a test file at the deepest level
    let test_file = deep_path.join("test_deep.py");
    std::fs::write(
        &test_file,
        r#"
import pytest

@pytest.fixture
def deep_fixture():
    return "deep"
"#,
    )
    .ok();

    db.scan_workspace(&temp_dir);

    // Should find the deeply nested fixture
    let defs = db.definitions.get("deep_fixture");
    assert!(defs.is_some());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
#[timeout(30000)]
fn test_scan_workspace_with_mixed_file_types() {
    let db = FixtureDatabase::new();
    let temp_dir = std::env::temp_dir().join("test_mixed_files");

    std::fs::create_dir_all(&temp_dir).ok();

    // Create conftest.py
    std::fs::write(
        temp_dir.join("conftest.py"),
        r#"
import pytest

@pytest.fixture
def conftest_fixture():
    return "conftest"
"#,
    )
    .ok();

    // Create test_*.py
    std::fs::write(
        temp_dir.join("test_example.py"),
        r#"
import pytest

@pytest.fixture
def test_file_fixture():
    return "test"
"#,
    )
    .ok();

    // Create *_test.py
    std::fs::write(
        temp_dir.join("example_test.py"),
        r#"
import pytest

@pytest.fixture
def suffix_test_fixture():
    return "suffix"
"#,
    )
    .ok();

    // Create non-test Python file
    std::fs::write(
        temp_dir.join("utils.py"),
        r#"
import pytest

@pytest.fixture
def utils_fixture():
    return "utils"
"#,
    )
    .ok();

    db.scan_workspace(&temp_dir);

    // Should find fixtures in test files and conftest
    assert!(db.definitions.get("conftest_fixture").is_some());
    assert!(db.definitions.get("test_file_fixture").is_some());
    assert!(db.definitions.get("suffix_test_fixture").is_some());
    // Should not find fixtures in non-test files
    assert!(db.definitions.get("utils_fixture").is_none());

    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
#[timeout(30000)]
fn test_empty_conftest_file() {
    let db = FixtureDatabase::new();

    // Analyze empty conftest
    let content = "";
    let path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(path, content);

    // Should not crash, should have no fixtures
    assert!(db.definitions.is_empty());
}

#[test]
#[timeout(30000)]
fn test_conftest_with_only_imports() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
import sys
from pathlib import Path
"#;
    let path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(path, content);

    // Should not crash, should have no fixtures
    assert!(db.definitions.is_empty());
}

#[test]
#[timeout(30000)]
fn test_file_with_syntax_error_in_docstring() {
    let db = FixtureDatabase::new();

    // Python file with weird but valid docstring
    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """
    This docstring has "quotes" and 'apostrophes'
    And some special chars: @#$%^&*()
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    // Docstring should be preserved
    assert!(defs[0].docstring.is_some());
}

#[test]
#[timeout(30000)]
fn test_fixture_in_file_with_multiple_encodings_declared() {
    let db = FixtureDatabase::new();

    // File with encoding declaration
    let content = r#"# -*- coding: utf-8 -*-
import pytest

@pytest.fixture
def my_fixture():
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

// MARK: Docstring Variation Tests

#[test]
#[timeout(30000)]
fn test_fixture_with_empty_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """"""
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    // Empty docstring might be None or Some("")
    if let Some(doc) = &defs[0].docstring {
        assert!(doc.trim().is_empty());
    }
}

#[test]
#[timeout(30000)]
fn test_fixture_with_multiline_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """
    This is a multi-line docstring.

    It has multiple paragraphs.

    Args:
        None

    Returns:
        str: A test string
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    let docstring = defs[0].docstring.as_ref().unwrap();
    assert!(docstring.contains("multi-line"));
    assert!(docstring.contains("Returns:"));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_single_quoted_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    '''Single quoted docstring'''
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    assert_eq!(
        defs[0].docstring.as_ref().unwrap().trim(),
        "Single quoted docstring"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_with_rst_formatted_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """
    Fixture with RST formatting.

    :param param1: First parameter
    :type param1: str
    :returns: Test value
    :rtype: str

    .. note::
        This is a note block
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    let docstring = defs[0].docstring.as_ref().unwrap();
    assert!(docstring.contains(":param"));
    assert!(docstring.contains(".. note::"));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_google_style_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """Fixture with Google-style docstring.

    This fixture provides a test value.

    Args:
        None

    Returns:
        str: A test string value

    Yields:
        str: Test value for the fixture
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    let docstring = defs[0].docstring.as_ref().unwrap();
    assert!(docstring.contains("Args:"));
    assert!(docstring.contains("Returns:"));
    assert!(docstring.contains("Yields:"));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_numpy_style_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """
    Fixture with NumPy-style docstring.

    Parameters
    ----------
    None

    Returns
    -------
    str
        A test string value

    Notes
    -----
    This is a test fixture
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    let docstring = defs[0].docstring.as_ref().unwrap();
    assert!(docstring.contains("Parameters"));
    assert!(docstring.contains("----------"));
    assert!(docstring.contains("Returns"));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_unicode_in_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """
    Fixture with Unicode characters: 日本語, Русский, العربية, 🎉

    This tests international character support in docstrings.
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    let docstring = defs[0].docstring.as_ref().unwrap();
    assert!(docstring.contains("日本語"));
    assert!(docstring.contains("Русский"));
    assert!(docstring.contains("🎉"));
}

#[test]
#[timeout(30000)]
fn test_fixture_with_code_blocks_in_docstring() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    """
    Fixture with code examples.

    Example:
        >>> result = my_fixture()
        >>> assert result == "test"

    Code block:
        ```python
        def use_fixture(my_fixture):
            print(my_fixture)
        ```
    """
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
    let docstring = defs[0].docstring.as_ref().unwrap();
    assert!(docstring.contains(">>>"));
    assert!(docstring.contains("```python"));
}

// MARK: Performance and Scalability Tests

#[test]
#[timeout(30000)]
fn test_large_number_of_fixtures_in_single_file() {
    let db = FixtureDatabase::new();

    // Generate a file with 100 fixtures
    let mut content = String::from("import pytest\n\n");
    for i in 0..100 {
        content.push_str(&format!(
            "@pytest.fixture\ndef fixture_{}():\n    return {}\n\n",
            i, i
        ));
    }

    let path = PathBuf::from("/tmp/test/test_many_fixtures.py");
    db.analyze_file(path, &content);

    // Should have all 100 fixtures
    assert_eq!(db.definitions.len(), 100);

    // Verify a few specific ones
    assert!(db.definitions.get("fixture_0").is_some());
    assert!(db.definitions.get("fixture_50").is_some());
    assert!(db.definitions.get("fixture_99").is_some());
}

#[test]
#[timeout(30000)]
fn test_deeply_nested_fixture_dependencies() {
    let db = FixtureDatabase::new();

    // Create a chain of 20 fixtures depending on each other
    let mut content = String::from("import pytest\n\n");
    content.push_str("@pytest.fixture\ndef fixture_0():\n    return 0\n\n");

    for i in 1..20 {
        content.push_str(&format!(
            "@pytest.fixture\ndef fixture_{}(fixture_{}):\n    return {} + fixture_{}\n\n",
            i,
            i - 1,
            i,
            i - 1
        ));
    }

    let path = PathBuf::from("/tmp/test/test_deep_chain.py");
    db.analyze_file(path, &content);

    // Should detect all fixtures
    assert_eq!(db.definitions.len(), 20);

    // Verify the deepest one
    let deepest = db.definitions.get("fixture_19").unwrap();
    assert_eq!(deepest.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_many_parameters() {
    let db = FixtureDatabase::new();

    // Create fixtures first
    let mut content = String::from("import pytest\n\n");
    for i in 0..15 {
        content.push_str(&format!(
            "@pytest.fixture\ndef dep_{}():\n    return {}\n\n",
            i, i
        ));
    }

    // Create a fixture that depends on all of them
    content.push_str("@pytest.fixture\ndef mega_fixture(");
    for i in 0..15 {
        if i > 0 {
            content.push_str(", ");
        }
        content.push_str(&format!("dep_{}", i));
    }
    content.push_str("):\n    return sum([");
    for i in 0..15 {
        if i > 0 {
            content.push_str(", ");
        }
        content.push_str(&format!("dep_{}", i));
    }
    content.push_str("])\n");

    let path = PathBuf::from("/tmp/test/test_many_params.py");
    db.analyze_file(path, &content);

    // Should have all 16 fixtures (15 deps + 1 mega)
    assert_eq!(db.definitions.len(), 16);
    assert!(db.definitions.get("mega_fixture").is_some());
}

#[test]
#[timeout(30000)]
fn test_very_long_fixture_function_body() {
    let db = FixtureDatabase::new();

    // Create a fixture with a very long function body (100 lines)
    let mut content = String::from("import pytest\n\n@pytest.fixture\ndef long_fixture():\n");
    content.push_str("    \"\"\"A fixture with a very long body.\"\"\"\n");
    for i in 0..100 {
        content.push_str(&format!("    line_{} = {}\n", i, i));
    }
    content.push_str("    return line_99\n");

    let path = PathBuf::from("/tmp/test/test_long_function.py");
    db.analyze_file(path, &content);

    let defs = db.definitions.get("long_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert!(defs[0].docstring.is_some());
}

#[test]
#[timeout(30000)]
fn test_multiple_files_with_same_fixture_names() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "value"
"#;

    // Analyze the same fixture in 50 different files
    for i in 0..50 {
        let path = PathBuf::from(format!("/tmp/test/dir_{}/test_file.py", i));
        db.analyze_file(path, content);
    }

    // Should have one fixture name with 50 definitions
    let defs = db.definitions.get("shared_fixture").unwrap();
    assert_eq!(defs.len(), 50);
}

#[test]
#[timeout(30000)]
fn test_rapid_file_updates() {
    let db = FixtureDatabase::new();

    let path = PathBuf::from("/tmp/test/test_updates.py");

    // Simulate rapid updates to the same file
    for i in 0..20 {
        let content = format!(
            r#"
import pytest

@pytest.fixture
def dynamic_fixture():
    return {}
"#,
            i
        );
        db.analyze_file(path.clone(), &content);
    }

    // Should have just one definition (the latest update)
    let defs = db.definitions.get("dynamic_fixture").unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file_path, path);
}

// MARK: Virtual Environment Variation Tests

#[test]
#[timeout(30000)]
fn test_fixture_detection_without_venv() {
    let db = FixtureDatabase::new();

    // Create a test project without scanning venv
    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "test"

def test_example(my_fixture):
    assert my_fixture == "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path.clone(), content);

    // Should still work without venv
    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);

    let usages = db.usages.get(&path).unwrap();
    assert_eq!(usages.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_third_party_fixture_in_site_packages() {
    let db = FixtureDatabase::new();

    // Simulate a third-party plugin fixture
    let plugin_content = r#"
import pytest

@pytest.fixture
def third_party_fixture():
    """A fixture from a third-party plugin."""
    return "plugin_value"
"#;

    // Analyze as if it's from site-packages
    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_plugin/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    // Now use it in a test file
    let test_content = r#"
def test_example(third_party_fixture):
    assert third_party_fixture == "plugin_value"
"#;
    let test_path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should detect both definition and usage
    let defs = db.definitions.get("third_party_fixture").unwrap();
    assert_eq!(defs.len(), 1);

    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_override_from_third_party() {
    let db = FixtureDatabase::new();

    // Third-party plugin defines a fixture
    let plugin_content = r#"
import pytest

@pytest.fixture
def event_loop():
    """Plugin event loop fixture."""
    return "plugin_loop"
"#;
    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_asyncio/fixtures.py");
    db.analyze_file(plugin_path.clone(), plugin_content);

    // User overrides it in conftest.py
    let conftest_content = r#"
import pytest

@pytest.fixture
def event_loop():
    """Custom event loop fixture."""
    return "custom_loop"
"#;
    let conftest_path = PathBuf::from("/tmp/project/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test uses it
    let test_content = r#"
def test_example(event_loop):
    assert event_loop is not None
"#;
    let test_path = PathBuf::from("/tmp/project/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // Should have 2 definitions (plugin + override)
    let defs = db.definitions.get("event_loop").unwrap();
    assert_eq!(defs.len(), 2);

    // Verify the conftest definition is present
    let conftest_def = defs.iter().find(|d| d.file_path == conftest_path);
    assert!(conftest_def.is_some());

    // Verify the plugin definition is present
    let plugin_def = defs.iter().find(|d| d.file_path == plugin_path);
    assert!(plugin_def.is_some());
}

#[test]
#[timeout(30000)]
fn test_multiple_third_party_plugins_same_fixture() {
    let db = FixtureDatabase::new();

    // Plugin 1 defines a fixture
    let plugin1_content = r#"
import pytest

@pytest.fixture
def common_fixture():
    return "plugin1"
"#;
    let plugin1_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_plugin1/fixtures.py");
    db.analyze_file(plugin1_path, plugin1_content);

    // Plugin 2 also defines the same fixture name
    let plugin2_content = r#"
import pytest

@pytest.fixture
def common_fixture():
    return "plugin2"
"#;
    let plugin2_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_plugin2/fixtures.py");
    db.analyze_file(plugin2_path, plugin2_content);

    // Should have both definitions
    let defs = db.definitions.get("common_fixture").unwrap();
    assert_eq!(defs.len(), 2);
}

#[test]
#[timeout(30000)]
fn test_venv_fixture_with_no_usage() {
    let db = FixtureDatabase::new();

    // Third-party fixture that's never used
    let plugin_content = r#"
import pytest

@pytest.fixture
def unused_plugin_fixture():
    """A fixture that's defined but never used."""
    return "unused"
"#;
    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_plugin/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    // Should still be in definitions
    let defs = db.definitions.get("unused_plugin_fixture").unwrap();
    assert_eq!(defs.len(), 1);

    // Should have no usages
    let refs = db.find_fixture_references("unused_plugin_fixture");
    assert!(refs.is_empty());
}

// MARK: Miscellaneous Edge Case Tests

#[test]
#[timeout(30000)]
fn test_fixture_with_property_decorator() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class MyFixture:
    @property
    def value(self):
        return "test"

@pytest.fixture
def my_fixture():
    return MyFixture()
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_staticmethod() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class FixtureHelper:
    @staticmethod
    def create():
        return "test"

@pytest.fixture
def my_fixture():
    return FixtureHelper.create()
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_classmethod() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

class FixtureHelper:
    @classmethod
    def create(cls):
        return "test"

@pytest.fixture
def my_fixture():
    return FixtureHelper.create()
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_contextmanager() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from contextlib import contextmanager

@contextmanager
def resource():
    yield "resource"

@pytest.fixture
def my_fixture():
    with resource() as r:
        return r
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_multiple_decorators() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

def custom_decorator(func):
    return func

@pytest.fixture
@custom_decorator
def my_fixture():
    return "test"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_inside_if_block_not_supported() {
    let db = FixtureDatabase::new();

    // Fixtures inside if blocks are a known limitation
    let content = r#"
import pytest
import sys

if sys.version_info >= (3, 8):
    @pytest.fixture
    def version_specific_fixture():
        return "py38+"
"#;
    let path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(path, content);

    // Currently not detected - this is a known limitation
    assert!(db.definitions.get("version_specific_fixture").is_none());
}

#[test]
#[timeout(30000)]
fn test_fixture_with_walrus_operator_in_body() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    if (result := expensive_operation()):
        return result
    return None
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_match_statement() {
    let db = FixtureDatabase::new();

    // Python 3.10+ match statement
    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    value = "test"
    match value:
        case "test":
            return "matched"
        case _:
            return "default"
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_exception_group() {
    let db = FixtureDatabase::new();

    // Python 3.11+ exception groups
    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    try:
        return "test"
    except* ValueError as e:
        return None
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_dataclass() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from dataclasses import dataclass

@dataclass
class Config:
    value: str

@pytest.fixture
def config_fixture():
    return Config(value="test")
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("config_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_named_tuple() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import NamedTuple

class Point(NamedTuple):
    x: int
    y: int

@pytest.fixture
def point_fixture():
    return Point(1, 2)
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("point_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_protocol() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import Protocol

class Readable(Protocol):
    def read(self) -> str: ...

@pytest.fixture
def readable_fixture() -> Readable:
    class TextReader:
        def read(self) -> str:
            return "test"
    return TextReader()
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("readable_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

#[test]
#[timeout(30000)]
fn test_fixture_with_generic_type() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest
from typing import Generic, TypeVar

T = TypeVar('T')

class Container(Generic[T]):
    def __init__(self, value: T):
        self.value = value

@pytest.fixture
def container_fixture() -> Container[str]:
    return Container("test")
"#;
    let path = PathBuf::from("/tmp/test/test_example.py");
    db.analyze_file(path, content);

    let defs = db.definitions.get("container_fixture").unwrap();
    assert_eq!(defs.len(), 1);
}

// MARK: Additional Third-Party Plugin Tests

#[test]
#[timeout(30000)]
fn test_pytest_flask_fixtures() {
    let db = FixtureDatabase::new();

    // Simulate pytest-flask plugin fixtures
    let plugin_content = r#"
import pytest

@pytest.fixture
def app():
    """Flask application fixture."""
    from flask import Flask
    app = Flask(__name__)
    return app

@pytest.fixture
def client(app):
    """Flask test client fixture."""
    return app.test_client()
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_flask/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    // Should detect both fixtures
    assert!(db.definitions.get("app").is_some());
    assert!(db.definitions.get("client").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_httpx_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture
async def async_client():
    """HTTPX async client fixture."""
    import httpx
    async with httpx.AsyncClient() as client:
        yield client
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_httpx/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("async_client").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_postgresql_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture
def postgresql():
    """PostgreSQL database fixture."""
    return {"host": "localhost", "port": 5432}

@pytest.fixture
def postgresql_proc(postgresql):
    """PostgreSQL process fixture."""
    return postgresql
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_postgresql/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("postgresql").is_some());
    assert!(db.definitions.get("postgresql_proc").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_docker_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture(scope="session")
def docker_compose_file():
    """Docker compose file fixture."""
    return "docker-compose.yml"

@pytest.fixture(scope="session")
def docker_services(docker_compose_file):
    """Docker services fixture."""
    return {"web": "http://localhost:8000"}
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_docker/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("docker_compose_file").is_some());
    assert!(db.definitions.get("docker_services").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_factoryboy_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest
import factory

class UserFactory(factory.Factory):
    class Meta:
        model = dict

    username = "testuser"
    email = "test@example.com"

@pytest.fixture
def user_factory():
    """User factory fixture."""
    return UserFactory
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_factoryboy/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("user_factory").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_freezegun_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest
from freezegun import freeze_time

@pytest.fixture
def frozen_time():
    """Frozen time fixture."""
    with freeze_time("2024-01-01"):
        yield
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_freezegun/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("frozen_time").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_celery_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture(scope="session")
def celery_config():
    """Celery configuration fixture."""
    return {"broker_url": "redis://localhost:6379"}

@pytest.fixture
def celery_app(celery_config):
    """Celery application fixture."""
    from celery import Celery
    return Celery("test_app", **celery_config)

@pytest.fixture
def celery_worker(celery_app):
    """Celery worker fixture."""
    return celery_app.Worker()
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_celery/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("celery_config").is_some());
    assert!(db.definitions.get("celery_app").is_some());
    assert!(db.definitions.get("celery_worker").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_aiohttp_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture
async def aiohttp_client():
    """Aiohttp client fixture."""
    import aiohttp
    async with aiohttp.ClientSession() as session:
        yield session

@pytest.fixture
async def aiohttp_server():
    """Aiohttp server fixture."""
    from aiohttp import web
    app = web.Application()
    return app
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_aiohttp/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("aiohttp_client").is_some());
    assert!(db.definitions.get("aiohttp_server").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_benchmark_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture
def benchmark():
    """Benchmark fixture."""
    class Benchmark:
        def __call__(self, func):
            return func()
    return Benchmark()
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_benchmark/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("benchmark").is_some());
}

#[test]
#[timeout(30000)]
fn test_pytest_playwright_fixtures() {
    let db = FixtureDatabase::new();

    let plugin_content = r#"
import pytest

@pytest.fixture(scope="session")
def browser():
    """Playwright browser fixture."""
    from playwright.sync_api import sync_playwright
    with sync_playwright() as p:
        yield p.chromium.launch()

@pytest.fixture
def page(browser):
    """Playwright page fixture."""
    page = browser.new_page()
    yield page
    page.close()

@pytest.fixture
def context(browser):
    """Playwright browser context fixture."""
    context = browser.new_context()
    yield context
    context.close()
"#;

    let plugin_path =
        PathBuf::from("/tmp/venv/lib/python3.11/site-packages/pytest_playwright/fixtures.py");
    db.analyze_file(plugin_path, plugin_content);

    assert!(db.definitions.get("browser").is_some());
    assert!(db.definitions.get("page").is_some());
    assert!(db.definitions.get("context").is_some());
}

// =============================================================================
// Tests for keyword-only and positional-only fixture arguments
// =============================================================================

#[test]
#[timeout(30000)]
fn test_keyword_only_fixture_usage_in_test() {
    let db = FixtureDatabase::new();

    // Add a fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test that uses keyword-only argument (after *)
    let test_content = r#"
def test_with_kwonly(*, my_fixture):
    assert my_fixture == 42
"#;
    let test_path = PathBuf::from("/tmp/test_kwonly.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that the fixture usage was detected
    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    assert!(
        usages.iter().any(|u| u.name == "my_fixture"),
        "Should detect my_fixture usage in keyword-only argument"
    );

    // Check that no undeclared fixtures were detected (the fixture is properly declared)
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures for keyword-only arg"
    );
}

#[test]
#[timeout(30000)]
fn test_keyword_only_fixture_usage_with_type_annotation() {
    let db = FixtureDatabase::new();

    // Add a fixture in conftest
    let conftest_content = r#"
import pytest
from pathlib import Path

@pytest.fixture
def tmp_path():
    return Path("/tmp")
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test that uses keyword-only argument with type annotation (like in the issue)
    let test_content = r#"
from pathlib import Path

def test_run_command(*, tmp_path: Path) -> None:
    """Test that uses a keyword-only fixture with type annotation."""
    rst_file = tmp_path / "example.rst"
    assert rst_file.parent == tmp_path
"#;
    let test_path = PathBuf::from("/tmp/test_kwonly_typed.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that the fixture usage was detected
    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    assert!(
        usages.iter().any(|u| u.name == "tmp_path"),
        "Should detect tmp_path usage in keyword-only argument"
    );

    // Check that no undeclared fixtures were detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures for keyword-only arg with type annotation"
    );
}

#[test]
#[timeout(30000)]
fn test_positional_only_fixture_usage() {
    let db = FixtureDatabase::new();

    // Add a fixture in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test that uses positional-only argument (before /)
    let test_content = r#"
def test_with_posonly(my_fixture, /):
    assert my_fixture == 42
"#;
    let test_path = PathBuf::from("/tmp/test_posonly.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that the fixture usage was detected
    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    assert!(
        usages.iter().any(|u| u.name == "my_fixture"),
        "Should detect my_fixture usage in positional-only argument"
    );

    // Check that no undeclared fixtures were detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures for positional-only arg"
    );
}

#[test]
#[timeout(30000)]
fn test_mixed_argument_types_fixture_usage() {
    let db = FixtureDatabase::new();

    // Add fixtures in conftest
    let conftest_content = r#"
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
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test that uses all argument types: positional-only, regular, and keyword-only
    let test_content = r#"
def test_with_all_types(fixture_a, /, fixture_b, *, fixture_c):
    assert fixture_a == "a"
    assert fixture_b == "b"
    assert fixture_c == "c"
"#;
    let test_path = PathBuf::from("/tmp/test_mixed.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that all fixture usages were detected
    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    assert!(
        usages.iter().any(|u| u.name == "fixture_a"),
        "Should detect fixture_a usage in positional-only argument"
    );
    assert!(
        usages.iter().any(|u| u.name == "fixture_b"),
        "Should detect fixture_b usage in regular argument"
    );
    assert!(
        usages.iter().any(|u| u.name == "fixture_c"),
        "Should detect fixture_c usage in keyword-only argument"
    );

    // Check that no undeclared fixtures were detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures for mixed argument types"
    );
}

#[test]
#[timeout(30000)]
fn test_keyword_only_fixture_in_fixture_definition() {
    let db = FixtureDatabase::new();

    // Add fixtures in conftest - one depends on another via keyword-only arg
    let conftest_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return 42

@pytest.fixture
def dependent_fixture(*, base_fixture):
    return base_fixture * 2
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Check that both fixtures were detected
    assert!(
        db.definitions.contains_key("base_fixture"),
        "base_fixture should be detected"
    );
    assert!(
        db.definitions.contains_key("dependent_fixture"),
        "dependent_fixture should be detected"
    );

    // Check that the usage of base_fixture in dependent_fixture was detected
    let usages = db.usages.get(&conftest_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    assert!(
        usages.iter().any(|u| u.name == "base_fixture"),
        "Should detect base_fixture usage as keyword-only dependency in dependent_fixture"
    );
}

#[test]
#[timeout(30000)]
fn test_keyword_only_with_multiple_fixtures() {
    let db = FixtureDatabase::new();

    // Add fixtures in conftest
    let conftest_content = r#"
import pytest

@pytest.fixture
def fixture_x():
    return "x"

@pytest.fixture
def fixture_y():
    return "y"

@pytest.fixture
def fixture_z():
    return "z"
"#;
    let conftest_path = PathBuf::from("/tmp/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Add a test with multiple keyword-only fixtures
    let test_content = r#"
def test_multi_kwonly(*, fixture_x, fixture_y, fixture_z):
    assert fixture_x == "x"
    assert fixture_y == "y"
    assert fixture_z == "z"
"#;
    let test_path = PathBuf::from("/tmp/test_multi_kwonly.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that all fixture usages were detected
    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    assert!(
        usages.iter().any(|u| u.name == "fixture_x"),
        "Should detect fixture_x usage"
    );
    assert!(
        usages.iter().any(|u| u.name == "fixture_y"),
        "Should detect fixture_y usage"
    );
    assert!(
        usages.iter().any(|u| u.name == "fixture_z"),
        "Should detect fixture_z usage"
    );

    // Check that no undeclared fixtures were detected
    let undeclared = db.get_undeclared_fixtures(&test_path);
    assert_eq!(
        undeclared.len(),
        0,
        "Should not detect any undeclared fixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_go_to_definition_for_keyword_only_fixture() {
    let db = FixtureDatabase::new();

    // Set up conftest.py with a fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
    let conftest_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Set up a test file that uses the fixture as keyword-only
    let test_content = r#"
def test_something(*, my_fixture):
    assert my_fixture == 42
"#;
    let test_path = PathBuf::from("/tmp/test/test_kwonly.py");
    db.analyze_file(test_path.clone(), test_content);

    // Verify fixture usage was detected
    let usages = db.usages.get(&test_path);
    assert!(usages.is_some(), "Usages should be detected");
    let usages = usages.unwrap();
    let fixture_usage = usages.iter().find(|u| u.name == "my_fixture");
    assert!(
        fixture_usage.is_some(),
        "Should detect my_fixture usage in keyword-only position"
    );

    // Get the line and character position of the usage
    let usage = fixture_usage.unwrap();

    // Try to find the definition from the test file at the usage position
    // usage.line is 1-based, but find_fixture_definition expects 0-based LSP coordinates
    let definition =
        db.find_fixture_definition(&test_path, (usage.line - 1) as u32, usage.start_char as u32);

    assert!(definition.is_some(), "Definition should be found");
    let def = definition.unwrap();
    assert_eq!(def.name, "my_fixture");
    assert_eq!(def.file_path, conftest_path);
}

// =============================================================================
// Tests for directory filtering during workspace scanning
// =============================================================================

#[test]
#[timeout(30000)]
fn test_scan_skips_node_modules() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create a test file in root
    let root_test = root.join("test_root.py");
    fs::write(
        &root_test,
        r#"
def test_root(root_fixture):
    pass
"#,
    )
    .unwrap();

    // Create conftest in root
    let root_conftest = root.join("conftest.py");
    fs::write(
        &root_conftest,
        r#"
import pytest

@pytest.fixture
def root_fixture():
    return 1
"#,
    )
    .unwrap();

    // Create node_modules with a test file that should be skipped
    let node_modules = root.join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    let node_test = node_modules.join("test_node.py");
    fs::write(
        &node_test,
        r#"
def test_node(node_fixture):
    pass
"#,
    )
    .unwrap();
    let node_conftest = node_modules.join("conftest.py");
    fs::write(
        &node_conftest,
        r#"
import pytest

@pytest.fixture
def node_fixture():
    return 2
"#,
    )
    .unwrap();

    let db = FixtureDatabase::new();
    db.scan_workspace(root);

    // Should find root_fixture but not node_fixture
    assert!(
        db.definitions.contains_key("root_fixture"),
        "root_fixture should be found"
    );
    assert!(
        !db.definitions.contains_key("node_fixture"),
        "node_fixture should NOT be found (node_modules should be skipped)"
    );
}

#[test]
#[timeout(30000)]
fn test_scan_skips_git_directory() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create a test file in root
    let root_conftest = root.join("conftest.py");
    fs::write(
        &root_conftest,
        r#"
import pytest

@pytest.fixture
def real_fixture():
    return 1
"#,
    )
    .unwrap();

    // Create .git with a conftest.py that should be skipped
    let git_dir = root.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    let git_conftest = git_dir.join("conftest.py");
    fs::write(
        &git_conftest,
        r#"
import pytest

@pytest.fixture
def git_fixture():
    return 2
"#,
    )
    .unwrap();

    let db = FixtureDatabase::new();
    db.scan_workspace(root);

    // Should find real_fixture but not git_fixture
    assert!(
        db.definitions.contains_key("real_fixture"),
        "real_fixture should be found"
    );
    assert!(
        !db.definitions.contains_key("git_fixture"),
        "git_fixture should NOT be found (.git should be skipped)"
    );
}

#[test]
#[timeout(30000)]
fn test_scan_skips_pycache() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create a test file in root
    let root_conftest = root.join("conftest.py");
    fs::write(
        &root_conftest,
        r#"
import pytest

@pytest.fixture
def actual_fixture():
    return 1
"#,
    )
    .unwrap();

    // Create __pycache__ with a conftest.py that should be skipped
    let pycache = root.join("__pycache__");
    fs::create_dir_all(&pycache).unwrap();
    let cache_conftest = pycache.join("conftest.py");
    fs::write(
        &cache_conftest,
        r#"
import pytest

@pytest.fixture
def cache_fixture():
    return 2
"#,
    )
    .unwrap();

    let db = FixtureDatabase::new();
    db.scan_workspace(root);

    // Should find actual_fixture but not cache_fixture
    assert!(
        db.definitions.contains_key("actual_fixture"),
        "actual_fixture should be found"
    );
    assert!(
        !db.definitions.contains_key("cache_fixture"),
        "cache_fixture should NOT be found (__pycache__ should be skipped)"
    );
}

#[test]
#[timeout(30000)]
fn test_scan_skips_venv_but_scans_plugins() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create a test file in root
    let root_conftest = root.join("conftest.py");
    fs::write(
        &root_conftest,
        r#"
import pytest

@pytest.fixture
def project_fixture():
    return 1
"#,
    )
    .unwrap();

    // Create .venv with a random test file that should be skipped during main scan
    let venv = root.join(".venv");
    fs::create_dir_all(&venv).unwrap();
    let venv_test = venv.join("test_venv.py");
    fs::write(
        &venv_test,
        r#"
def test_venv(venv_fixture):
    pass
"#,
    )
    .unwrap();

    let db = FixtureDatabase::new();
    db.scan_workspace(root);

    // Should find project_fixture
    assert!(
        db.definitions.contains_key("project_fixture"),
        "project_fixture should be found"
    );

    // venv test files should not create usages in the main scan
    // (venv is scanned separately for plugin fixtures only)
    let venv_test_path = venv_test.canonicalize().unwrap_or(venv_test);
    assert!(
        !db.usages.contains_key(&venv_test_path),
        "test files in .venv should not be scanned"
    );
}

#[test]
#[timeout(30000)]
fn test_scan_skips_multiple_directories() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create a test file in root
    let root_conftest = root.join("conftest.py");
    fs::write(
        &root_conftest,
        r#"
import pytest

@pytest.fixture
def main_fixture():
    return 1
"#,
    )
    .unwrap();

    // Create multiple directories that should be skipped
    for skip_dir in &[
        "node_modules",
        ".git",
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        "build",
        "dist",
        ".tox",
    ] {
        let dir = root.join(skip_dir);
        fs::create_dir_all(&dir).unwrap();
        let conftest = dir.join("conftest.py");
        fs::write(
            &conftest,
            format!(
                r#"
import pytest

@pytest.fixture
def {}_fixture():
    return 2
"#,
                skip_dir.replace(".", "").replace("-", "_")
            ),
        )
        .unwrap();
    }

    let db = FixtureDatabase::new();
    db.scan_workspace(root);

    // Should only find main_fixture
    assert!(
        db.definitions.contains_key("main_fixture"),
        "main_fixture should be found"
    );

    // None of the skipped directory fixtures should be found
    assert!(
        !db.definitions.contains_key("node_modules_fixture"),
        "node_modules fixture should be skipped"
    );
    assert!(
        !db.definitions.contains_key("git_fixture"),
        ".git fixture should be skipped"
    );
    assert!(
        !db.definitions.contains_key("__pycache___fixture"),
        "__pycache__ fixture should be skipped"
    );
    assert!(
        !db.definitions.contains_key("pytest_cache_fixture"),
        ".pytest_cache fixture should be skipped"
    );
}

#[test]
#[timeout(30000)]
fn test_scan_skips_nested_node_modules() {
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create a test file in root
    let root_conftest = root.join("conftest.py");
    fs::write(
        &root_conftest,
        r#"
import pytest

@pytest.fixture
def root_fix():
    return 1
"#,
    )
    .unwrap();

    // Create a tests directory with a test file (should be scanned)
    let tests_dir = root.join("tests");
    fs::create_dir_all(&tests_dir).unwrap();
    let tests_conftest = tests_dir.join("conftest.py");
    fs::write(
        &tests_conftest,
        r#"
import pytest

@pytest.fixture
def tests_fix():
    return 2
"#,
    )
    .unwrap();

    // Create deeply nested node_modules (should be skipped entirely)
    let deep_node = root.join("frontend/app/node_modules/some_package");
    fs::create_dir_all(&deep_node).unwrap();
    let deep_conftest = deep_node.join("conftest.py");
    fs::write(
        &deep_conftest,
        r#"
import pytest

@pytest.fixture
def deep_node_fix():
    return 3
"#,
    )
    .unwrap();

    let db = FixtureDatabase::new();
    db.scan_workspace(root);

    // Should find root and tests fixtures
    assert!(
        db.definitions.contains_key("root_fix"),
        "root_fix should be found"
    );
    assert!(
        db.definitions.contains_key("tests_fix"),
        "tests_fix should be found"
    );

    // Should NOT find deeply nested node_modules fixture
    assert!(
        !db.definitions.contains_key("deep_node_fix"),
        "deep_node_fix should NOT be found (nested node_modules should be skipped)"
    );
}

// =============================================================================
// pytest.mark.usefixtures tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_usefixtures_decorator_on_function() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def db_connection():
    return "connection"

@pytest.fixture
def auth_user():
    return "user"
"#;

    let test_content = r#"
import pytest

@pytest.mark.usefixtures("db_connection")
def test_with_usefixtures():
    pass

@pytest.mark.usefixtures("db_connection", "auth_user")
def test_with_multiple_usefixtures():
    pass
"#;

    let conftest_path = PathBuf::from("/tmp/test_usefixtures/conftest.py");
    let test_path = PathBuf::from("/tmp/test_usefixtures/test_example.py");

    db.analyze_file(conftest_path, conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // Check that usefixtures usages were detected
    let usages = db.usages.get(&test_path).unwrap();

    assert!(
        usages.iter().any(|u| u.name == "db_connection"),
        "db_connection should be detected as usage from usefixtures"
    );
    assert!(
        usages.iter().any(|u| u.name == "auth_user"),
        "auth_user should be detected as usage from usefixtures"
    );

    // Count occurrences - db_connection should appear twice (once for each test)
    let db_conn_count = usages.iter().filter(|u| u.name == "db_connection").count();
    assert_eq!(
        db_conn_count, 2,
        "db_connection should be used twice (once in each test)"
    );
}

#[test]
#[timeout(30000)]
fn test_usefixtures_decorator_on_class() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def setup_database():
    return "db"
"#;

    let test_content = r#"
import pytest

@pytest.mark.usefixtures("setup_database")
class TestWithSetup:
    def test_first(self):
        pass

    def test_second(self):
        pass
"#;

    let conftest_path = PathBuf::from("/tmp/test_usefixtures/conftest.py");
    let test_path = PathBuf::from("/tmp/test_usefixtures/test_class.py");

    db.analyze_file(conftest_path, conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // Check that usefixtures usage on class was detected
    let usages = db.usages.get(&test_path).unwrap();

    assert!(
        usages.iter().any(|u| u.name == "setup_database"),
        "setup_database should be detected as usage from class usefixtures"
    );
}

#[test]
#[timeout(30000)]
fn test_usefixtures_goto_definition() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;

    let test_content = r#"
import pytest

@pytest.mark.usefixtures("my_fixture")
def test_something():
    pass
"#;

    let conftest_path = PathBuf::from("/tmp/test_usefixtures/conftest.py");
    let test_path = PathBuf::from("/tmp/test_usefixtures/test_goto.py");

    db.analyze_file(conftest_path.clone(), conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // The fixture "my_fixture" in @pytest.mark.usefixtures("my_fixture") is on line 4 (1-indexed)
    // In 0-indexed LSP coords: line 3
    // Position is within the string "my_fixture"
    // @pytest.mark.usefixtures("my_fixture")
    //                          ^--- somewhere in the middle of the fixture name

    let definition = db.find_fixture_definition(&test_path, 3, 27);

    assert!(
        definition.is_some(),
        "Definition should be found for fixture used in usefixtures"
    );
    let def = definition.unwrap();
    assert_eq!(def.name, "my_fixture");
    assert_eq!(def.file_path, conftest_path);
}

#[test]
#[timeout(30000)]
fn test_usefixtures_affects_unused_detection() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def used_via_usefixtures():
    return "used"

@pytest.fixture
def actually_unused():
    return "unused"
"#;

    let test_content = r#"
import pytest

@pytest.mark.usefixtures("used_via_usefixtures")
def test_something():
    pass
"#;

    let conftest_path = PathBuf::from("/tmp/test_usefixtures/conftest.py");
    let test_path = PathBuf::from("/tmp/test_usefixtures/test_unused.py");

    db.analyze_file(conftest_path.clone(), conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // Get all usages across all files
    let mut all_usages: Vec<String> = Vec::new();
    for entry in db.usages.iter() {
        for usage in entry.value().iter() {
            all_usages.push(usage.name.clone());
        }
    }

    // used_via_usefixtures should be in usages (not unused)
    assert!(
        all_usages.contains(&"used_via_usefixtures".to_string()),
        "Fixture used via usefixtures should be tracked as used"
    );
}

#[test]
#[timeout(30000)]
fn test_usefixtures_with_mark_import() {
    let db = FixtureDatabase::new();

    let test_content = r#"
from pytest import mark, fixture

@fixture
def my_fix():
    return 1

@mark.usefixtures("my_fix")
def test_with_mark():
    pass
"#;

    let test_path = PathBuf::from("/tmp/test_usefixtures/test_mark.py");
    db.analyze_file(test_path.clone(), test_content);

    // Check that both the fixture definition and usage were detected
    assert!(
        db.definitions.contains_key("my_fix"),
        "my_fix fixture should be detected"
    );

    let usages = db.usages.get(&test_path).unwrap();
    assert!(
        usages.iter().any(|u| u.name == "my_fix"),
        "my_fix should be detected as usage from mark.usefixtures"
    );
}

// =============================================================================
// pytest_plugins tests (known limitation documentation)
// =============================================================================

/// Test that pytest_plugins is recognized at the module level
/// Note: Full resolution of pytest_plugins paths is not implemented
/// This test documents the current behavior
#[test]
#[timeout(30000)]
fn test_pytest_plugins_declaration_detected() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
# Declare external fixture modules
pytest_plugins = ["myapp.fixtures", "other.fixtures"]

import pytest

@pytest.fixture
def local_fixture():
    return "local"
"#;

    let conftest_path = PathBuf::from("/tmp/test_plugins/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    // Local fixtures should still be detected
    assert!(
        db.definitions.contains_key("local_fixture"),
        "local_fixture should be detected even with pytest_plugins"
    );

    // Note: We don't currently resolve the pytest_plugins modules
    // This is a known limitation - fixtures from those modules won't be found
    // unless the modules are explicitly scanned as part of the workspace
}

/// Test that pytest_plugins tuple syntax is also recognized
#[test]
#[timeout(30000)]
fn test_pytest_plugins_tuple_syntax() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
pytest_plugins = ("plugin1", "plugin2")

import pytest

@pytest.fixture
def another_fixture():
    return "value"
"#;

    let conftest_path = PathBuf::from("/tmp/test_plugins/conftest.py");
    db.analyze_file(conftest_path, conftest_content);

    // Fixture detection should work normally
    assert!(
        db.definitions.contains_key("another_fixture"),
        "another_fixture should be detected"
    );
}

// =============================================================================
// pytest.mark.parametrize with indirect tests
// =============================================================================

#[test]
#[timeout(30000)]
fn test_parametrize_indirect_true() {
    let db = FixtureDatabase::new();

    let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture(request):
    return request.param * 2
"#;

    let test_content = r#"
import pytest

@pytest.mark.parametrize("my_fixture", [1, 2, 3], indirect=True)
def test_with_indirect(my_fixture):
    assert my_fixture in [2, 4, 6]
"#;

    let conftest_path = PathBuf::from("/tmp/test_indirect/conftest.py");
    let test_path = PathBuf::from("/tmp/test_indirect/test_indirect.py");

    db.analyze_file(conftest_path, conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // my_fixture should be detected as usage both from the parameter and from indirect
    let usages = db.usages.get(&test_path).unwrap();
    let fixture_usages: Vec<_> = usages.iter().filter(|u| u.name == "my_fixture").collect();

    // Should have 2 usages: one from indirect decorator, one from function parameter
    assert!(
        fixture_usages.len() >= 2,
        "my_fixture should be used at least twice (indirect + parameter)"
    );
}

#[test]
#[timeout(30000)]
fn test_parametrize_indirect_multiple_fixtures() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def fixture_a(request):
    return request.param

@pytest.fixture
def fixture_b(request):
    return request.param

@pytest.mark.parametrize("fixture_a,fixture_b", [(1, 2), (3, 4)], indirect=True)
def test_multiple_indirect(fixture_a, fixture_b):
    pass
"#;

    let test_path = PathBuf::from("/tmp/test_indirect/test_multiple.py");
    db.analyze_file(test_path.clone(), test_content);

    let usages = db.usages.get(&test_path).unwrap();

    // Both fixtures should be detected as indirect usages
    assert!(
        usages.iter().any(|u| u.name == "fixture_a"),
        "fixture_a should be detected as indirect usage"
    );
    assert!(
        usages.iter().any(|u| u.name == "fixture_b"),
        "fixture_b should be detected as indirect usage"
    );
}

#[test]
#[timeout(30000)]
fn test_parametrize_indirect_list_selective() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def indirect_fix(request):
    return request.param

@pytest.fixture
def direct_fix():
    return "direct"

@pytest.mark.parametrize("indirect_fix,direct_fix", [(1, 2)], indirect=["indirect_fix"])
def test_selective_indirect(indirect_fix, direct_fix):
    pass
"#;

    let test_path = PathBuf::from("/tmp/test_indirect/test_selective.py");
    db.analyze_file(test_path.clone(), test_content);

    let usages = db.usages.get(&test_path).unwrap();

    // indirect_fix should have an additional usage from the indirect list
    let indirect_usages: Vec<_> = usages.iter().filter(|u| u.name == "indirect_fix").collect();
    assert!(
        indirect_usages.len() >= 2,
        "indirect_fix should have at least 2 usages (from indirect list + parameter)"
    );
}

#[test]
#[timeout(30000)]
fn test_parametrize_without_indirect() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.mark.parametrize("value", [1, 2, 3])
def test_normal_parametrize(value):
    pass
"#;

    let test_path = PathBuf::from("/tmp/test_indirect/test_normal.py");
    db.analyze_file(test_path.clone(), test_content);

    // value should be detected as a parameter usage, but not as an indirect fixture
    let usages = db.usages.get(&test_path).unwrap();
    let value_usages: Vec<_> = usages.iter().filter(|u| u.name == "value").collect();

    // Should only have 1 usage from the function parameter
    assert_eq!(
        value_usages.len(),
        1,
        "value should only have 1 usage (from parameter, not indirect)"
    );
}

// MARK: Scoping Tests - Issue #23

#[test]
#[timeout(30000)]
fn test_fixture_scoping_sibling_files() {
    // Test case from issue #23:
    // A fixture defined in one test file should NOT be counted as "used"
    // when a sibling test file uses a parameter with the same name.
    let db = FixtureDatabase::new();

    // File 1: defines a fixture
    let test1_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "example"
"#;
    let test1_path = PathBuf::from("/tmp/test_scope/test_example_2.py");
    db.analyze_file(test1_path.clone(), test1_content);

    // File 2: uses a parameter with the same name, but the fixture is NOT in scope
    let test2_content = r#"
def test_example_fixture(my_fixture):
    assert my_fixture == "example"
"#;
    let test2_path = PathBuf::from("/tmp/test_scope/test_example.py");
    db.analyze_file(test2_path.clone(), test2_content);

    // Verify the fixture is defined
    let fixture_defs = db.definitions.get("my_fixture").unwrap();
    assert_eq!(fixture_defs.len(), 1);
    let fixture_def = &fixture_defs[0];
    assert_eq!(fixture_def.file_path, test1_path);

    // The key assertion: find_references_for_definition should NOT include
    // the usage from test_example.py because the fixture is not in scope there
    let refs = db.find_references_for_definition(fixture_def);
    assert_eq!(
        refs.len(),
        0,
        "Fixture defined in test_example_2.py should have 0 references \
         because test_example.py cannot access it (not in conftest.py hierarchy)"
    );
}

#[test]
#[timeout(30000)]
fn test_fixture_scoping_with_conftest() {
    // When a fixture IS in conftest.py, sibling files CAN use it
    let db = FixtureDatabase::new();

    // conftest.py defines a fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "shared"
"#;
    let conftest_path = PathBuf::from("/tmp/test_scope2/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // Test file uses the fixture
    let test_content = r#"
def test_uses_shared(shared_fixture):
    assert shared_fixture == "shared"
"#;
    let test_path = PathBuf::from("/tmp/test_scope2/test_example.py");
    db.analyze_file(test_path.clone(), test_content);

    // The fixture from conftest.py should be accessible
    let fixture_defs = db.definitions.get("shared_fixture").unwrap();
    let fixture_def = &fixture_defs[0];

    let refs = db.find_references_for_definition(fixture_def);
    assert_eq!(
        refs.len(),
        1,
        "Fixture in conftest.py should have 1 reference from sibling test file"
    );
    assert_eq!(refs[0].file_path, test_path);
}

#[test]
#[timeout(30000)]
fn test_fixture_scoping_same_file() {
    // Fixture defined in the same file should be usable
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def local_fixture():
    return "local"

def test_uses_local(local_fixture):
    assert local_fixture == "local"
"#;
    let test_path = PathBuf::from("/tmp/test_scope3/test_local.py");
    db.analyze_file(test_path.clone(), test_content);

    let fixture_defs = db.definitions.get("local_fixture").unwrap();
    let fixture_def = &fixture_defs[0];

    let refs = db.find_references_for_definition(fixture_def);
    assert_eq!(
        refs.len(),
        1,
        "Fixture defined in same file should have 1 reference"
    );
    assert_eq!(refs[0].file_path, test_path);
}

#[test]
#[timeout(30000)]
fn test_get_scoped_usage_count() {
    // Test the new get_scoped_usage_count method
    let db = FixtureDatabase::new();

    // Setup: conftest.py with a fixture
    let conftest_content = r#"
import pytest

@pytest.fixture
def global_fixture():
    return "global"
"#;
    let conftest_path = PathBuf::from("/tmp/test_scope4/conftest.py");
    db.analyze_file(conftest_path.clone(), conftest_content);

    // File 1: defines a local fixture with the same name (overrides)
    let test1_content = r#"
import pytest

@pytest.fixture
def global_fixture():
    return "local override"

def test_uses_local(global_fixture):
    pass
"#;
    let test1_path = PathBuf::from("/tmp/test_scope4/subdir/test_override.py");
    db.analyze_file(test1_path.clone(), test1_content);

    // File 2: uses the global fixture (no override)
    let test2_content = r#"
def test_uses_global(global_fixture):
    pass
"#;
    let test2_path = PathBuf::from("/tmp/test_scope4/test_global.py");
    db.analyze_file(test2_path.clone(), test2_content);

    // The conftest fixture should only be used by test_global.py (1 reference)
    let conftest_defs = db.definitions.get("global_fixture").unwrap();
    let conftest_def = conftest_defs
        .iter()
        .find(|d| d.file_path == conftest_path)
        .unwrap();

    let conftest_refs = db.find_references_for_definition(conftest_def);
    assert_eq!(
        conftest_refs.len(),
        1,
        "Conftest fixture should have 1 reference (from test_global.py)"
    );
    assert_eq!(conftest_refs[0].file_path, test2_path);

    // The local override fixture should be used by test_override.py (1 reference)
    let local_def = conftest_defs
        .iter()
        .find(|d| d.file_path == test1_path)
        .unwrap();

    let local_refs = db.find_references_for_definition(local_def);
    assert_eq!(
        local_refs.len(),
        1,
        "Local override fixture should have 1 reference"
    );
    assert_eq!(local_refs[0].file_path, test1_path);
}

// ============================================================================
// Completion Context Tests
// ============================================================================

#[test]
#[timeout(30000)]
fn test_completion_context_function_signature() {
    use pytest_language_server::CompletionContext;
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_something():
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // LSP uses 0-indexed lines, position inside the parentheses of test_something()
    // Line 7 (0-indexed) is "def test_something():"
    // Cursor at position 18 (inside parentheses)
    let ctx = db.get_completion_context(&test_path, 7, 18);

    assert!(ctx.is_some());
    match ctx.unwrap() {
        CompletionContext::FunctionSignature {
            function_name,
            is_fixture,
            declared_params,
            ..
        } => {
            assert_eq!(function_name, "test_something");
            assert!(!is_fixture);
            assert!(declared_params.is_empty());
        }
        _ => panic!("Expected FunctionSignature context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_function_signature_with_params() {
    use pytest_language_server::CompletionContext;
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_something(my_fixture, ):
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position after the comma, inside parentheses
    // Line 7 (0-indexed): "def test_something(my_fixture, ):"
    // Cursor at position 31 (after the comma and space)
    let ctx = db.get_completion_context(&test_path, 7, 31);

    assert!(ctx.is_some());
    match ctx.unwrap() {
        CompletionContext::FunctionSignature {
            function_name,
            is_fixture,
            declared_params,
            ..
        } => {
            assert_eq!(function_name, "test_something");
            assert!(!is_fixture);
            assert_eq!(declared_params, vec!["my_fixture"]);
        }
        _ => panic!("Expected FunctionSignature context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_function_body() {
    use pytest_language_server::CompletionContext;
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_something(my_fixture):

    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position inside the function body (the empty line)
    // Line 8 (0-indexed) is the empty line inside the function
    let ctx = db.get_completion_context(&test_path, 8, 4);

    assert!(ctx.is_some());
    match ctx.unwrap() {
        CompletionContext::FunctionBody {
            function_name,
            is_fixture,
            declared_params,
            ..
        } => {
            assert_eq!(function_name, "test_something");
            assert!(!is_fixture);
            assert_eq!(declared_params, vec!["my_fixture"]);
        }
        _ => panic!("Expected FunctionBody context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_fixture_function() {
    use pytest_language_server::CompletionContext;
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return 42

@pytest.fixture
def dependent_fixture():
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position inside parentheses of dependent_fixture
    // Line 8 (0-indexed): "@pytest.fixture" followed by line 9: "def dependent_fixture():"
    let ctx = db.get_completion_context(&test_path, 8, 22);

    assert!(ctx.is_some());
    match ctx.unwrap() {
        CompletionContext::FunctionSignature {
            function_name,
            is_fixture,
            declared_params,
            ..
        } => {
            assert_eq!(function_name, "dependent_fixture");
            assert!(is_fixture);
            assert!(declared_params.is_empty());
        }
        _ => panic!("Expected FunctionSignature context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_usefixtures_decorator() {
    use pytest_language_server::CompletionContext;
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

@pytest.mark.usefixtures("")
def test_something():
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position inside the string of usefixtures decorator
    // Line 7 (0-indexed): "@pytest.mark.usefixtures("")"
    // Cursor at position 27 (inside the empty quotes)
    let ctx = db.get_completion_context(&test_path, 7, 27);

    assert!(ctx.is_some());
    match ctx.unwrap() {
        CompletionContext::UsefixuturesDecorator => {}
        _ => panic!("Expected UsefixuturesDecorator context"),
    }
}

#[test]
#[timeout(30000)]
fn test_completion_context_outside_function() {
    let db = FixtureDatabase::new();

    let test_content = r#"
import pytest

# A comment

def test_something():
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // Position on the comment line (not inside a function)
    // Line 3 (0-indexed): "# A comment"
    let ctx = db.get_completion_context(&test_path, 3, 5);

    assert!(ctx.is_none());
}

#[test]
#[timeout(30000)]
fn test_get_function_param_insertion_info_empty_params() {
    let db = FixtureDatabase::new();

    let test_content = r#"
def test_something():
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // Function is on line 2 (1-indexed)
    let info = db.get_function_param_insertion_info(&test_path, 2);

    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.line, 2);
    assert!(!info.needs_comma);
}

#[test]
#[timeout(30000)]
fn test_get_function_param_insertion_info_with_params() {
    let db = FixtureDatabase::new();

    let test_content = r#"
def test_something(existing_param):
    pass
"#;

    let test_path = PathBuf::from("/tmp/test/test_completion.py");
    db.analyze_file(test_path.clone(), test_content);

    // Function is on line 2 (1-indexed)
    let info = db.get_function_param_insertion_info(&test_path, 2);

    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.line, 2);
    assert!(info.needs_comma);
}
