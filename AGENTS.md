# AGENTS.md - AI Agent Development Guide

This document helps AI agents understand the pytest-language-server codebase structure, architecture, and development practices.

## Project Overview

**pytest-language-server** is a Language Server Protocol (LSP) implementation for pytest fixtures, written in Rust. It provides IDE features like go-to-definition, find-references, and hover documentation for pytest fixtures.

- **Language**: Rust (Edition 2021, MSRV 1.83)
- **Lines of Code**: ~3,050 lines (2,256 in fixtures.rs, 794 in main.rs)
- **Architecture**: Async LSP server using tower-lsp
- **Key Features**: Fixture go-to-definition, find-references, hover docs, fixture overriding, undeclared fixture diagnostics

## Core Architecture

### Module Structure

```
src/
├── lib.rs          # Library exports (3 lines)
├── main.rs         # LSP server implementation (794 lines)
└── fixtures.rs     # Fixture analysis engine (2,256 lines)
```

### Key Components

1. **FixtureDatabase** (`src/fixtures.rs`)
   - Central data structure for storing fixture definitions and usages
   - Uses `DashMap` for lock-free concurrent access
   - Handles workspace scanning, file analysis, and fixture resolution
   - Implements pytest's fixture priority/shadowing rules

2. **Backend** (`src/main.rs`)
   - LSP server implementation using `tower-lsp`
   - Handles LSP protocol requests (initialize, goto_definition, references, hover)
   - Coordinates with FixtureDatabase for fixture information
   - Manages text document lifecycle (did_open, did_change)

### Data Structures

```rust
// Core types from src/fixtures.rs:

pub struct FixtureDefinition {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub docstring: Option<String>,
}

pub struct FixtureUsage {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize,  // Character position on line
    pub end_char: usize,    // Character position on line
}

pub struct UndeclaredFixture {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub function_name: String,  // Name of test/fixture where used
    pub function_line: usize,   // Line where function is defined
}

pub struct FixtureDatabase {
    // Map: fixture name -> all definitions (multiple conftest.py files)
    definitions: Arc<DashMap<String, Vec<FixtureDefinition>>>,
    // Map: file path -> usages in that file
    usages: Arc<DashMap<PathBuf, Vec<FixtureUsage>>>,
    // Cache of analyzed file contents
    file_cache: Arc<DashMap<PathBuf, String>>,
    // Map: file path -> undeclared fixtures in function bodies
    undeclared_fixtures: Arc<DashMap<PathBuf, Vec<UndeclaredFixture>>>,
}
```

## Pytest Fixture Resolution Rules

The LSP correctly implements pytest's fixture priority/shadowing rules:

1. **Same file**: Fixtures defined in the same file have highest priority
2. **Closest conftest.py**: Walk up directory tree looking for conftest.py
3. **Virtual environment**: Third-party plugin fixtures (pytest-mock, pytest-asyncio, etc.)

### Character-Position Awareness

A critical feature added in v0.4.0: when a fixture overrides another fixture with the same name, the LSP distinguishes between the function name and parameter:

```python
@pytest.fixture
def cli_runner(cli_runner):  # Self-referencing fixture
    return cli_runner
```

- Cursor at position 4 (function name) → refers to child fixture
- Cursor at position 16+ (parameter) → refers to parent fixture

This is handled by `start_char` and `end_char` in `FixtureUsage`.

## Key Methods & Logic

### src/fixtures.rs

**Core Methods:**
- `scan_workspace(&self, root_path: &Path)` - Walks directory tree, finds test files
- `analyze_file(&self, file_path: PathBuf, content: &str)` - Parses Python AST, extracts fixtures
- `find_fixture_definition(&self, file_path: &Path, fixture_name: &str, line: usize, char: usize)` - Resolves fixture based on priority rules
- `find_fixture_at_position(&self, file_path: &Path, line: usize, char: usize)` - Finds fixture name at cursor
- `find_all_references(&self, fixture_name: &str, def_file: &Path)` - Finds all usages of a fixture
- `get_char_position_from_offset(&self, file_path: &Path, line: usize, char_offset: usize)` - Converts byte offset to character position
- `get_undeclared_fixtures(&self, file_path: &Path)` - Gets all undeclared fixture usages in a file
- `scan_function_body_for_undeclared_fixtures()` - Detects fixtures used in function bodies without parameter declaration

**AST Parsing:**
- Uses `rustpython-parser` to parse Python files
- Looks for `@pytest.fixture` decorators
- Handles assignment-style fixtures (pytest-mock pattern: `mocker = pytest.fixture()(_mocker)`)
- Extracts function signatures, docstrings, and parameter dependencies
- Walks function body AST to find Name expressions that reference available fixtures

**Undeclared Fixture Detection:**
- Scans test and fixture function bodies for name references
- Checks if each name is an available fixture (respects hierarchy)
- Excludes declared parameters and built-in names (self, request)
- Tracks line/character position for diagnostics
- Only reports fixtures that are actually available in the current scope

### src/main.rs

**LSP Handlers:**
- `initialize()` - Scans workspace on startup
- `goto_definition()` - Calls `find_fixture_at_position()` then `find_fixture_definition()`
- `references()` - Finds all references, ensures current position is included (LSP spec compliance)
- `hover()` - Shows fixture signature and docstring in Markdown format
- `did_open()`, `did_change()` - Re-analyzes files when opened/modified, publishes diagnostics
- `code_action()` - Provides quick fixes to add missing fixture parameters
- `publish_diagnostics_for_file()` - Publishes warnings for undeclared fixtures

## Testing

### Test Structure

```
src/
├── fixtures.rs           # Main code (2,256 lines)
└── main.rs              # LSP server (794 lines)

tests/
├── test_fixtures.rs     # FixtureDatabase integration tests (60 tests, 2,951 lines)
├── test_lsp.rs         # LSP protocol tests (13 tests, 1,187 lines)
└── test_project/       # Fixture test files for integration tests
    ├── conftest.py
    ├── test_example.py
    ├── test_parent_usage.py
    └── subdir/
        ├── conftest.py
        ├── test_hierarchy.py
        └── test_override.py
```

### Running Tests

```bash
cargo test                          # Run all tests (73 total)
cargo test --test test_fixtures     # Run FixtureDatabase tests (60 tests)
cargo test --test test_lsp         # Run LSP protocol tests (13 tests)
RUST_LOG=debug cargo test          # Run with debug logging
```

### Test Coverage

- **73 total tests passing** (as of v0.6.0)
  - 60 integration tests in `tests/test_fixtures.rs` (FixtureDatabase API)
  - 13 integration tests in `tests/test_lsp.rs` (LSP protocol handlers)

Key test areas:
- Fixture definition extraction from various patterns
- Fixture usage detection in test functions and other fixtures
- Fixture priority/shadowing rules (8 comprehensive hierarchy tests)
- Character-position awareness for self-referencing fixtures
- LSP spec compliance (references always include current position)
- Multiline function signatures
- Third-party fixture detection
- Undeclared fixture detection (5 tests)
- Hierarchy-aware undeclared fixture reporting
- Deterministic fixture resolution (ensures no random behavior with multiple definitions)
- Path normalization and canonicalization
- Deep directory hierarchy support
- Sibling directory isolation

## Development Workflow

### Build & Run

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run with logging
RUST_LOG=debug cargo run

# Format code
cargo fmt

# Lint
cargo clippy

# Security audit
cargo audit
```

### Version Bumping

**IMPORTANT**: Always use the provided script to bump versions across all files:

```bash
./bump-version.sh 0.5.0  # Updates Cargo.toml, pyproject.toml, zed-extension/
```

This script automatically updates:
- `Cargo.toml`
- `pyproject.toml`
- `zed-extension/Cargo.toml`
- `zed-extension/extension.toml`
- `Cargo.lock`

The script also updates Cargo.lock and ensures all versions are synchronized. After running, commit with:
```bash
git add -A && git commit -m "chore: bump version to X.Y.Z"
```

### Pre-commit Hooks

The project uses pre-commit hooks defined in `.pre-commit-config.yaml`:
- `cargo fmt --check` - Format checking
- `cargo clippy` - Linting
- `cargo audit` - Security vulnerability scanning

Install with: `pre-commit install`

## Common Development Tasks

### Adding a New LSP Feature

1. Add the capability in `main.rs` `initialize()` method's `ServerCapabilities`
2. Implement the handler method (async trait impl)
3. Add necessary methods to `FixtureDatabase` in `fixtures.rs`
4. Write integration tests in `main.rs` (see existing tests for patterns)
5. Update README.md with feature documentation

### Modifying Fixture Resolution Logic

1. Edit `src/fixtures.rs` methods:
   - `find_fixture_definition()` for go-to-definition
   - `find_all_references()` for find-references
   - `analyze_file()` if changing what fixtures are detected
2. Add test cases to `src/fixtures.rs` tests
3. Run `cargo test` to ensure all 52 tests pass
4. Consider edge cases: self-referencing fixtures, multiline signatures, conftest.py hierarchy

### Debugging LSP Issues

1. Set `RUST_LOG=debug` or `RUST_LOG=trace` environment variable
2. Check logs in stderr (LSP uses stdout for protocol communication)
3. Key log points:
   - "goto_definition request" - shows incoming requests
   - "Looking for fixture definition" - shows resolution logic
   - "Found fixture at position" - shows what fixture was detected
   - "Resolved fixture definition" - shows final resolution
4. Use editor's LSP client logs to see request/response JSON

### Testing in an Editor

1. Build release binary: `cargo build --release`
2. Binary location: `target/release/pytest-language-server`
3. Configure editor to use this binary
4. Test on `tests/test_project/` for quick iteration

## Dependencies

Core dependencies (from `Cargo.toml`):
- **tower-lsp** (0.20.0) - LSP framework
- **tokio** (1.48) - Async runtime
- **rustpython-parser** (0.4.0) - Python AST parsing
- **dashmap** (6.1) - Concurrent hash map
- **walkdir** (2.5) - Directory traversal
- **tracing** (0.1) - Logging framework

## File Naming Conventions

Python test discovery patterns:
- `conftest.py` - Fixture configuration files
- `test_*.py` - Test files (prefix pattern)
- `*_test.py` - Test files (suffix pattern)

## Known Edge Cases

1. **Self-referencing fixtures**: Fixtures that override a parent fixture with the same name
   - Handled via character-position awareness (`start_char`, `end_char`)

2. **Multiline function signatures**: Function definitions spanning multiple lines
   - Handled in `analyze_file()` by checking line bounds during AST traversal

3. **Assignment-style fixtures**: `mocker = pytest.fixture()(_mocker)` pattern
   - Detected in `analyze_file()` via AST pattern matching

4. **Async fixtures**: `async def` fixtures
   - Treated the same as regular fixtures

5. **Third-party fixtures**: pytest-mock, pytest-asyncio, pytest-django
   - Scanned from virtual environment site-packages

6. **Path normalization and canonicalization** (fixed in v0.5.1)
   - All file paths are canonicalized in `analyze_file()` to handle symlinks and resolve absolute paths
   - This ensures consistent path comparisons in fixture resolution
   - Prevents random fixture selection when paths have different representations
   - Critical for large projects with multiple conftest.py files

7. **Deterministic fixture resolution** (fixed in v0.5.1)
   - When multiple fixture definitions exist in unrelated directories, resolution is deterministic
   - Priority order: same file > conftest hierarchy > third-party (site-packages) > sorted by path
   - Prevents non-deterministic behavior from DashMap iteration order

## LSP Spec Compliance

Critical LSP specification requirements:

1. **References must include current position** (added in v0.4.0)
   - When user invokes find-references, the cursor position MUST be in results
   - Handled in `main.rs` references handler

2. **Character ranges must be accurate** (added in v0.4.0)
   - Use actual fixture name character positions, not line start (0)
   - Stored in `FixtureUsage.start_char` and `end_char`

3. **Hover uses Markdown format**
   - Documentation formatted with proper code blocks
   - Docstrings dedented and cleaned

## Performance Considerations

- **Concurrent workspace scanning**: Uses `DashMap` for lock-free parallel file processing
- **Incremental updates**: Re-analyzes only changed files on `did_change`
- **Efficient lookup**: HashMap-based fixture lookup by name
- **AST parsing**: Cached in `file_cache` for re-analysis

## Release Process

1. Make changes and commit
2. Run `./bump-version.sh X.Y.Z` to update version
3. Update CHANGELOG or release notes
4. Create GitHub release with `gh release create vX.Y.Z`
5. CI automatically:
   - Builds binaries for all platforms
   - Publishes to PyPI
   - Publishes to crates.io
   - Updates Homebrew formula

## Troubleshooting

### Tests failing after fixture logic changes
- Check that all 52 tests pass: `cargo test`
- Focus on failing tests in `fixtures.rs` (fixture resolution) or `main.rs` (LSP handlers)
- Common issue: fixture priority rules not respecting conftest.py hierarchy

### LSP not finding fixtures
- Check `RUST_LOG=debug` logs for "Scanning workspace" messages
- Verify workspace root is correct
- Check if files match test patterns (`test_*.py`, `conftest.py`)
- Verify Python AST parsing isn't failing (look for parse errors in logs)

### References not including current position
- Added in v0.4.0 to fix LSP spec violation
- Check that `start_char` and `end_char` are correctly set in `FixtureUsage`
- Verify `find_fixture_at_position()` is checking character bounds

## Additional Resources

- **README.md** - User-facing documentation and setup instructions
- **SECURITY.md** - Security policy and vulnerability reporting
- **RELEASE.md** - Release process documentation
- **tests/test_project/** - Example pytest project for testing
- **zed-extension/** - Zed editor extension source

## Contributing Guidelines

1. Run tests: `cargo test`
2. Run lints: `cargo clippy`
3. Format code: `cargo fmt`
4. Run security audit: `cargo audit`
5. Install pre-commit hooks: `pre-commit install`
6. Write tests for new features
7. Update AGENTS.md if adding significant architectural changes

## Version History

- **v0.5.1** (November 2025) - Critical fix for deterministic fixture resolution, path canonicalization, 8 new comprehensive hierarchy tests
- **v0.5.0** (November 2025) - Undeclared fixture diagnostics, code actions (quick fixes), line-aware scoping, LSP compliance improvements
- **v0.4.0** (November 2025) - Character-position aware references, LSP spec compliance
- **v0.3.1** - Previous stable release
- See GitHub releases for full changelog

---

**Last Updated**: v0.5.1 (November 2025)

This document should be updated when making significant architectural changes or adding new features.
