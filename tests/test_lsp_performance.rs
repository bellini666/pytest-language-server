//! Performance and real-world LSP scenario tests.
//!
//! Tests for typical editor use cases like file changes, file moves, and
//! rapid consecutive operations that might occur in a real editor.
//!
//! All tests have a 10-second timeout to prevent hangs from blocking CI.

use ntest::timeout;
use pytest_language_server::FixtureDatabase;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper to create a temporary test file with given content
fn create_temp_test_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(name);
    std::fs::write(&file_path, content).unwrap();
    file_path
}

#[test]
#[timeout(10000)]
fn test_rapid_file_changes() {
    // Simulate a user rapidly editing a file in an editor
    let temp_dir = TempDir::new().unwrap();
    let file_path = create_temp_test_file(
        &temp_dir,
        "test_file.py",
        r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_something(my_fixture):
    assert my_fixture == 42
"#,
    );

    let db = FixtureDatabase::new();

    // Initial file analysis
    let content1 = std::fs::read_to_string(&file_path).unwrap();
    db.analyze_file(file_path.clone(), &content1);

    // Verify initial state
    assert_eq!(db.definitions.len(), 1);
    assert!(db.definitions.contains_key("my_fixture"));

    // Simulate rapid edits - user types more content
    let content2 = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

@pytest.fixture
def another_fixture():
    return "hello"

def test_something(my_fixture, another_fixture):
    assert my_fixture == 42
    assert another_fixture == "hello"
"#;
    db.analyze_file(file_path.clone(), content2);

    // Should now have 2 fixtures
    assert_eq!(db.definitions.len(), 2);
    assert!(db.definitions.contains_key("my_fixture"));
    assert!(db.definitions.contains_key("another_fixture"));

    // Simulate user deleting a fixture
    let content3 = r#"
import pytest

@pytest.fixture
def another_fixture():
    return "hello"

def test_something(another_fixture):
    assert another_fixture == "hello"
"#;
    db.analyze_file(file_path.clone(), content3);

    // Should only have 1 fixture now
    assert_eq!(db.definitions.len(), 1);
    assert!(!db.definitions.contains_key("my_fixture"));
    assert!(db.definitions.contains_key("another_fixture"));
}

#[test]
#[timeout(10000)]
fn test_file_rename_scenario() {
    // Simulate renaming a test file (editor removes from one path, adds to another)
    let temp_dir = TempDir::new().unwrap();

    let original_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "data"

def test_one(shared_fixture):
    assert shared_fixture == "data"
"#;

    // Create original file
    let old_path = create_temp_test_file(&temp_dir, "test_old.py", original_content);

    let db = FixtureDatabase::new();
    db.analyze_file(old_path.clone(), original_content);

    // Verify fixture is tracked
    assert_eq!(db.definitions.len(), 1);
    assert!(db.definitions.contains_key("shared_fixture"));

    // Simulate file rename by analyzing new path with same content
    let new_path = temp_dir.path().join("test_new.py");
    std::fs::write(&new_path, original_content).unwrap();
    db.analyze_file(new_path.clone(), original_content);

    // Fixture should still be in the database (from both paths until old is cleaned up)
    assert!(db.definitions.contains_key("shared_fixture"));

    // In a real LSP scenario, the server would clean up the old file's data
    // when notified of file deletion. For this test, we verify both are accessible.
}

#[test]
#[timeout(10000)]
fn test_multiple_files_simultaneous_changes() {
    // Simulate multiple files being edited at the same time (e.g., multi-cursor edit)
    let temp_dir = TempDir::new().unwrap();

    let file1_content = r#"
import pytest

@pytest.fixture
def fixture_a():
    return 1
"#;

    let file2_content = r#"
import pytest

@pytest.fixture
def fixture_b():
    return 2
"#;

    let file1 = create_temp_test_file(&temp_dir, "test_1.py", file1_content);
    let file2 = create_temp_test_file(&temp_dir, "test_2.py", file2_content);

    let db = FixtureDatabase::new();

    // Analyze both files
    db.analyze_file(file1.clone(), file1_content);
    db.analyze_file(file2.clone(), file2_content);

    assert_eq!(db.definitions.len(), 2);

    // Simulate both files being edited to add the same fixture name (conflict scenario)
    let new_content = r#"
import pytest

@pytest.fixture
def shared_name():
    return 42
"#;

    db.analyze_file(file1.clone(), new_content);
    db.analyze_file(file2.clone(), new_content);

    // Should have 1 fixture name with 2 definitions
    assert_eq!(db.definitions.len(), 1);
    let defs = db.definitions.get("shared_name").unwrap();
    assert_eq!(defs.len(), 2);
}

#[test]
#[timeout(10000)]
fn test_large_file_incremental_changes() {
    // Test performance with a large file that gets incrementally edited
    let temp_dir = TempDir::new().unwrap();

    // Generate a large file with many fixtures
    let mut content = String::from("import pytest\n\n");
    for i in 0..50 {
        content.push_str(&format!(
            "@pytest.fixture\ndef fixture_{}():\n    return {}\n\n",
            i, i
        ));
    }

    let file_path = create_temp_test_file(&temp_dir, "test_large.py", &content);

    let db = FixtureDatabase::new();

    // Initial analysis
    let start = std::time::Instant::now();
    db.analyze_file(file_path.clone(), &content);
    let initial_duration = start.elapsed();

    assert_eq!(db.definitions.len(), 50);

    // Add one more fixture (simulating user typing)
    content.push_str("@pytest.fixture\ndef new_fixture():\n    return 999\n");

    // Re-analyze
    let start = std::time::Instant::now();
    db.analyze_file(file_path.clone(), &content);
    let update_duration = start.elapsed();

    assert_eq!(db.definitions.len(), 51);

    // Performance check: incremental update should be reasonably fast
    // This is a regression test - if changes make it much slower, this will fail
    // Using 1 second to be generous for CI/slower systems
    const MAX_UPDATE_TIME_MS: u64 = 1000;
    assert!(
        update_duration < std::time::Duration::from_millis(MAX_UPDATE_TIME_MS),
        "File re-analysis took too long: {:?} (max: {}ms)",
        update_duration,
        MAX_UPDATE_TIME_MS
    );

    println!(
        "Large file analysis: initial={:?}, update={:?}",
        initial_duration, update_duration
    );
}

#[test]
#[timeout(10000)]
fn test_conftest_hierarchy_with_changes() {
    // Test that fixture resolution remains correct when conftest files change
    let temp_dir = TempDir::new().unwrap();

    // Create directory structure
    let root = temp_dir.path();
    let subdir = root.join("subdir");
    std::fs::create_dir(&subdir).unwrap();

    // Root conftest
    let root_conftest = r#"
import pytest

@pytest.fixture
def root_fixture():
    return "root"
"#;

    // Subdir test file using the fixture
    let test_content = r#"
def test_something(root_fixture):
    assert root_fixture == "root"
"#;

    let root_conftest_path = root.join("conftest.py");
    std::fs::write(&root_conftest_path, root_conftest).unwrap();

    let test_path = subdir.join("test_sub.py");
    std::fs::write(&test_path, test_content).unwrap();
    // Canonicalize path since database stores canonical paths
    let test_path = test_path.canonicalize().unwrap();

    let db = FixtureDatabase::new();

    // Analyze both files
    db.analyze_file(root_conftest_path.clone(), root_conftest);
    db.analyze_file(test_path.clone(), test_content);

    // Verify fixture is found
    assert!(db.definitions.contains_key("root_fixture"));
    {
        // Scope the DashMap reference to avoid holding read lock across analyze_file
        let usages = db.usages.get(&test_path).unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].name, "root_fixture");
    }

    // Simulate conftest.py being edited to add another fixture
    let updated_conftest = r#"
import pytest

@pytest.fixture
def root_fixture():
    return "root"

@pytest.fixture
def new_root_fixture():
    return "new"
"#;

    db.analyze_file(root_conftest_path.clone(), updated_conftest);

    // Both fixtures should be available now
    assert_eq!(db.definitions.len(), 2);
    assert!(db.definitions.contains_key("root_fixture"));
    assert!(db.definitions.contains_key("new_root_fixture"));

    // Test file usages shouldn't change (only uses root_fixture)
    let usages = db.usages.get(&test_path).unwrap();
    assert_eq!(usages.len(), 1);
}

#[test]
#[timeout(10000)]
fn test_cache_effectiveness_on_repeated_access() {
    // Verify that line index caching improves performance on repeated access
    let temp_dir = TempDir::new().unwrap();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_one(my_fixture):
    assert my_fixture == 42

def test_two(my_fixture):
    assert my_fixture == 42

def test_three(my_fixture):
    assert my_fixture == 42
"#;

    let file_path = create_temp_test_file(&temp_dir, "test_cache.py", content);
    // Canonicalize path since database stores canonical paths
    let file_path = file_path.canonicalize().unwrap();
    let db = FixtureDatabase::new();

    // First analysis - should populate cache
    db.analyze_file(file_path.clone(), content);

    // Check that line index cache is populated
    assert!(db.line_index_cache.contains_key(&file_path));

    // Get the cached hash to verify it's content-based
    let cached_hash = db.line_index_cache.get(&file_path).map(|e| e.value().0);
    assert!(cached_hash.is_some());

    // Perform multiple fixture lookups (simulating hover/goto operations)
    const LOOKUP_COUNT: usize = 10;
    const TEST_LINE: u32 = 7; // Line with "def test_one(my_fixture):"
    const FIXTURE_CHAR_POS: u32 = 15; // Character position of "my_fixture" parameter

    for _ in 0..LOOKUP_COUNT {
        let result = db.find_fixture_definition(&file_path, TEST_LINE, FIXTURE_CHAR_POS);
        assert!(result.is_some());
    }

    // Cache should still be there with the same hash (content unchanged)
    assert!(db.line_index_cache.contains_key(&file_path));
    let new_hash = db.line_index_cache.get(&file_path).map(|e| e.value().0);
    assert_eq!(cached_hash, new_hash, "Cache hash should remain the same");

    // Re-analyze with same content - cache should be reused (same hash)
    db.analyze_file(file_path.clone(), content);
    let reanalyzed_hash = db.line_index_cache.get(&file_path).map(|e| e.value().0);
    assert_eq!(
        cached_hash, reanalyzed_hash,
        "Cache should be reused for same content"
    );

    // Analyze with different content - cache should be updated (different hash)
    let new_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 99  # Changed value
"#;
    db.analyze_file(file_path.clone(), new_content);
    let updated_hash = db.line_index_cache.get(&file_path).map(|e| e.value().0);
    assert_ne!(
        cached_hash, updated_hash,
        "Cache should be invalidated for different content"
    );
}

#[test]
#[timeout(10000)]
fn test_concurrent_file_modifications() {
    // Test that the database handles concurrent updates correctly
    // (simulates multiple threads/async tasks updating different files)
    use std::sync::Arc;
    use std::thread;

    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(FixtureDatabase::new());

    let mut handles = vec![];

    // Spawn multiple threads, each analyzing different files
    for i in 0..5 {
        let db_clone = Arc::clone(&db);
        let dir_path = temp_dir.path().to_path_buf();

        let handle = thread::spawn(move || {
            let content = format!(
                r#"
import pytest

@pytest.fixture
def fixture_{}():
    return {}
"#,
                i, i
            );

            let file_path = dir_path.join(format!("test_{}.py", i));
            std::fs::write(&file_path, &content).unwrap();

            // Each thread analyzes its file multiple times
            for _ in 0..3 {
                db_clone.analyze_file(file_path.clone(), &content);
            }
        });

        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all fixtures were recorded
    assert_eq!(db.definitions.len(), 5);
    for i in 0..5 {
        assert!(db.definitions.contains_key(&format!("fixture_{}", i)));
    }
}

#[test]
#[timeout(10000)]
fn test_cleanup_file_cache_removes_cached_data() {
    // Test that cleanup_file_cache properly removes line_index_cache and file_cache entries
    let temp_dir = TempDir::new().unwrap();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_one(my_fixture):
    assert my_fixture == 42
"#;

    let file_path = create_temp_test_file(&temp_dir, "test_cleanup.py", content);
    let canonical_path = file_path.canonicalize().unwrap();

    let db = FixtureDatabase::new();

    // Analyze file to populate caches
    db.analyze_file(file_path.clone(), content);

    // Verify caches are populated
    assert!(
        db.line_index_cache.contains_key(&canonical_path),
        "line_index_cache should contain the file"
    );
    assert!(
        db.file_cache.contains_key(&canonical_path),
        "file_cache should contain the file"
    );

    // Clean up caches for the file
    db.cleanup_file_cache(&file_path);

    // Verify caches are cleared
    assert!(
        !db.line_index_cache.contains_key(&canonical_path),
        "line_index_cache should be empty after cleanup"
    );
    assert!(
        !db.file_cache.contains_key(&canonical_path),
        "file_cache should be empty after cleanup"
    );

    // Definitions and usages should still be present (they're cleaned on next analyze)
    assert!(db.definitions.contains_key("my_fixture"));
    assert!(db.usages.contains_key(&canonical_path));
}

#[test]
#[timeout(10000)]
fn test_cleanup_file_cache_handles_nonexistent_file() {
    // Test that cleanup_file_cache gracefully handles files that were never analyzed
    let db = FixtureDatabase::new();

    // Should not panic when cleaning up a file that doesn't exist in cache
    let fake_path = PathBuf::from("/nonexistent/path/test.py");
    db.cleanup_file_cache(&fake_path);

    // Database should remain functional
    assert!(db.definitions.is_empty());
    assert!(db.line_index_cache.is_empty());
}

#[test]
#[timeout(10000)]
fn test_usage_by_fixture_reverse_index() {
    // Test that the usage_by_fixture reverse index is properly maintained
    let temp_dir = TempDir::new().unwrap();

    let content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return 42

@pytest.fixture
def another_fixture(shared_fixture):
    return shared_fixture + 1

def test_one(shared_fixture):
    assert shared_fixture == 42

def test_two(shared_fixture, another_fixture):
    assert shared_fixture == 42
"#;

    let file_path = create_temp_test_file(&temp_dir, "test_reverse_index.py", content);
    let canonical_path = file_path.canonicalize().unwrap();

    let db = FixtureDatabase::new();
    db.analyze_file(file_path.clone(), content);

    // Verify reverse index contains usages for shared_fixture
    assert!(
        db.usage_by_fixture.contains_key("shared_fixture"),
        "usage_by_fixture should track shared_fixture usages"
    );

    {
        // Scope the DashMap reference to avoid holding read lock across analyze_file
        let usages = db.usage_by_fixture.get("shared_fixture").unwrap();
        // shared_fixture is used in: another_fixture (dependency), test_one, test_two
        assert_eq!(
            usages.len(),
            3,
            "shared_fixture should have 3 usages (1 fixture dep + 2 tests)"
        );

        // Verify all usages point to the correct file
        for (path, usage) in usages.iter() {
            assert_eq!(*path, canonical_path);
            assert_eq!(usage.name, "shared_fixture");
        }
    }

    // Re-analyze with fewer usages - verify cleanup works
    let updated_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return 42

def test_one(shared_fixture):
    assert shared_fixture == 42
"#;

    db.analyze_file(file_path.clone(), updated_content);

    // Now should only have 1 usage
    let usages = db.usage_by_fixture.get("shared_fixture").unwrap();
    assert_eq!(
        usages.len(),
        1,
        "shared_fixture should now have only 1 usage after re-analysis"
    );
}

#[test]
#[timeout(10000)]
fn test_available_fixtures_cache() {
    // Test that get_available_fixtures caches results and invalidates on changes
    let temp_dir = TempDir::new().unwrap();

    let conftest_content = r#"
import pytest

@pytest.fixture
def root_fixture():
    return "root"
"#;

    let test_content = r#"
def test_something(root_fixture):
    pass
"#;

    let conftest_path = create_temp_test_file(&temp_dir, "conftest.py", conftest_content);
    let test_path = create_temp_test_file(&temp_dir, "test_cache.py", test_content);
    let canonical_test_path = test_path.canonicalize().unwrap();

    let db = FixtureDatabase::new();
    db.analyze_file(conftest_path.clone(), conftest_content);
    db.analyze_file(test_path.clone(), test_content);

    // First call should compute and cache
    let fixtures1 = db.get_available_fixtures(&canonical_test_path);
    assert_eq!(fixtures1.len(), 1);
    assert_eq!(fixtures1[0].name, "root_fixture");

    // Cache should now contain this file
    assert!(
        db.available_fixtures_cache
            .contains_key(&canonical_test_path),
        "available_fixtures_cache should contain the file after get_available_fixtures"
    );

    // Second call should use cache (same result)
    let fixtures2 = db.get_available_fixtures(&canonical_test_path);
    assert_eq!(fixtures1.len(), fixtures2.len());
    assert_eq!(fixtures1[0].name, fixtures2[0].name);

    // Add a new fixture - this should invalidate the cache via version bump
    let updated_conftest = r#"
import pytest

@pytest.fixture
def root_fixture():
    return "root"

@pytest.fixture
def new_fixture():
    return "new"
"#;

    db.analyze_file(conftest_path.clone(), updated_conftest);

    // Cache should be invalidated, new fixtures should be returned
    let fixtures3 = db.get_available_fixtures(&canonical_test_path);
    assert_eq!(
        fixtures3.len(),
        2,
        "Should now have 2 fixtures after cache invalidation"
    );

    let fixture_names: Vec<&str> = fixtures3.iter().map(|f| f.name.as_str()).collect();
    assert!(fixture_names.contains(&"root_fixture"));
    assert!(fixture_names.contains(&"new_fixture"));
}

#[test]
#[timeout(10000)]
fn test_cleanup_file_cache_clears_available_fixtures_cache() {
    // Test that cleanup_file_cache also removes available_fixtures_cache entries
    let temp_dir = TempDir::new().unwrap();

    let content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

def test_one(my_fixture):
    pass
"#;

    let file_path = create_temp_test_file(&temp_dir, "test_cleanup_avail.py", content);
    let canonical_path = file_path.canonicalize().unwrap();

    let db = FixtureDatabase::new();
    db.analyze_file(file_path.clone(), content);

    // Trigger caching by calling get_available_fixtures
    let _ = db.get_available_fixtures(&canonical_path);

    // Verify cache is populated
    assert!(
        db.available_fixtures_cache.contains_key(&canonical_path),
        "available_fixtures_cache should contain the file"
    );

    // Clean up caches
    db.cleanup_file_cache(&file_path);

    // Verify available_fixtures_cache is cleared
    assert!(
        !db.available_fixtures_cache.contains_key(&canonical_path),
        "available_fixtures_cache should be empty after cleanup"
    );
}

#[test]
#[timeout(10000)]
fn test_find_references_uses_reverse_index() {
    // Test that find_references_for_definition uses the reverse index efficiently
    let temp_dir = TempDir::new().unwrap();

    let conftest = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"
"#;

    // Create multiple test files that use the fixture
    let test1 = r#"
def test_a(base_fixture):
    pass
"#;
    let test2 = r#"
def test_b(base_fixture):
    pass
"#;
    let test3 = r#"
def test_c(base_fixture):
    pass
"#;

    let _conftest_path = create_temp_test_file(&temp_dir, "conftest.py", conftest);
    create_temp_test_file(&temp_dir, "test_a.py", test1);
    create_temp_test_file(&temp_dir, "test_b.py", test2);
    create_temp_test_file(&temp_dir, "test_c.py", test3);

    let db = FixtureDatabase::new();
    db.scan_workspace(temp_dir.path());

    // Get the fixture definition
    let definition = db.definitions.get("base_fixture").unwrap()[0].clone();

    // Find references using the reverse index
    let references = db.find_references_for_definition(&definition);

    // Should find 3 references (one in each test file)
    assert_eq!(
        references.len(),
        3,
        "Should find 3 references across test files"
    );

    // Verify the reverse index has the right data
    let usage_entries = db.usage_by_fixture.get("base_fixture").unwrap();
    assert_eq!(
        usage_entries.len(),
        3,
        "Reverse index should have 3 entries"
    );
}

#[test]
#[timeout(30000)]
fn test_cache_handles_many_files() {
    // Test that the cache can handle many files without issues
    // The eviction logic is triggered internally when analyzing files
    let db = FixtureDatabase::new();

    // Analyze many files to build up the cache
    let content = r#"
import pytest

@pytest.fixture
def fixture_{n}():
    return {n}
"#;

    // Add many files - the cache eviction is called internally after each analyze
    for i in 0..100 {
        let file_content = content.replace("{n}", &i.to_string());
        let path = PathBuf::from(format!("/tmp/test_cache_many/file_{}.py", i));
        db.analyze_file(path.clone(), &file_content);
    }

    // Verify files were analyzed and cached
    assert!(
        db.file_cache.len() >= 100,
        "Should have at least 100 files in cache"
    );

    // Verify all fixtures were detected
    for i in 0..100 {
        let fixture_name = format!("fixture_{}", i);
        assert!(
            db.definitions.contains_key(&fixture_name),
            "fixture_{} should be detected",
            i
        );
    }
}
