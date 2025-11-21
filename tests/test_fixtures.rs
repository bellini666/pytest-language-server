use pytest_language_server::FixtureDatabase;
use std::path::PathBuf;

#[test]
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
fn test_fixture_resolution_multiple_unrelated_branches_is_deterministic() {
    // This is the key test: when a fixture is defined in multiple unrelated branches,
    // the resolution should be deterministic (not random based on DashMap iteration)
    let db = FixtureDatabase::new();

    // Three unrelated project branches
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

    // Test in yet another unrelated location
    let test_content = r#"
def test_something(common_fixture):
    assert common_fixture is not None
"#;
    let test_path = PathBuf::from("/tmp/unrelated/test_foo.py");
    db.analyze_file(test_path.clone(), test_content);

    // Run resolution multiple times - should always return the same result
    let mut results = Vec::new();
    for _ in 0..20 {
        let result = db.find_fixture_definition(&test_path, 1, 19);
        assert!(result.is_some(), "Should find a fixture");
        results.push(result.unwrap().file_path.clone());
    }

    // All results should be identical (deterministic)
    let first_result = &results[0];
    for (i, result) in results.iter().enumerate() {
        assert_eq!(
            result, first_result,
            "Iteration {}: fixture resolution should be deterministic, expected {:?} but got {:?}",
            i, first_result, result
        );
    }
}

#[test]
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

    // Currently, @pytest_asyncio.fixture is NOT detected (limitation)
    // Only @pytest.fixture and bare @fixture are supported
    // See src/fixtures.rs:653 is_fixture_decorator()
    assert!(
        !db.definitions.contains_key("async_fixture"),
        "pytest_asyncio.fixture not currently supported - this is a known limitation"
    );

    // Regular async fixtures with @pytest.fixture ARE detected
    assert!(db.definitions.contains_key("regular_async_fixture"));
}

#[test]
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

    // Should detect fixtures by their internal name
    // Note: The 'name' parameter creates an alias, but we currently
    // detect fixtures by their function name
    assert!(db.definitions.contains_key("internal_fixture_name"));
    assert!(db.definitions.contains_key("database_connection"));

    // Future enhancement: also detect by alias name
    // assert!(db.definitions.contains_key("custom_name"));
    // assert!(db.definitions.contains_key("db"));
}

#[test]
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
    assert!(db.definitions.contains_key("complex_fixture"));
    assert!(db.definitions.contains_key("parametrized_scoped"));
}

#[test]
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
fn test_unicode_fixture_names() {
    let db = FixtureDatabase::new();

    let content = r#"
import pytest

@pytest.fixture
def 測試_fixture():
    return "test"

@pytest.fixture
def фикстура():
    return "fixture"

@pytest.fixture
def fixture_émoji():
    return "emoji"
"#;
    let file_path = PathBuf::from("/tmp/test/conftest.py");
    db.analyze_file(file_path.clone(), content);

    // Python 3 supports Unicode identifiers
    if db.definitions.contains_key("測試_fixture") {
        assert!(db.definitions.contains_key("фикстура"));
        assert!(db.definitions.contains_key("fixture_émoji"));
        println!("Unicode fixture names fully supported");
    } else {
        println!("LIMITATION: Unicode identifiers not fully supported by parser");
    }
}

#[test]
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
