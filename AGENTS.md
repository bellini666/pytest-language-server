# AGENTS.md - AI Agent Development Guide

This document helps AI agents understand the pytest-language-server codebase structure, architecture, and development practices.

## AI Agent Workflow Rules

**IMPORTANT**: When the user asks you to make changes or complete a task, follow this workflow:

1. Complete the requested task fully
2. **ALWAYS** ask the user if they want to:
   - Commit the changes
   - Commit and push the changes
   - Leave the changes uncommitted
3. **NEVER** commit or push changes automatically without explicit user confirmation
4. If the user confirms they want to commit, create an appropriate commit message following the project's commit style
5. Only push to remote if the user explicitly requests it

This ensures the user maintains full control over their git workflow.

## Project Overview

**pytest-language-server** is a Language Server Protocol (LSP) implementation for pytest fixtures, written in Rust. It provides IDE features like go-to-definition, find-references, and hover documentation for pytest fixtures.

- **Language**: Rust (Edition 2021, MSRV 1.83)
- **Lines of Code**: ~4,100 lines across modular structure
- **Architecture**: Async LSP server using tower-lsp with CLI support via clap
- **Key Features**: Fixture go-to-definition, find-references, hover docs, **code completion**, fixture overriding, undeclared fixture diagnostics, CLI commands, `@pytest.mark.usefixtures` support, `@pytest.mark.parametrize` indirect fixtures

## Core Architecture

### Module Structure

```
src/
├── lib.rs              # Library exports (~7 lines)
├── main.rs             # LanguageServer trait impl + CLI (~310 lines)
├── fixtures/           # Fixture analysis engine
│   ├── mod.rs          # FixtureDatabase struct + helpers (~135 lines)
│   ├── types.rs        # Data types (~50 lines)
│   ├── scanner.rs      # Workspace scanning (~320 lines)
│   ├── analyzer.rs     # AST parsing (~1100 lines)
│   ├── resolver.rs     # Query methods (~860 lines)
│   ├── cli.rs          # CLI tree printing (~360 lines)
│   ├── decorators.rs   # Decorator analysis utilities (~180 lines)
│   └── string_utils.rs # String manipulation utilities (~180 lines)
└── providers/          # LSP protocol handlers
    ├── mod.rs          # Backend struct + helpers (~195 lines)
    ├── definition.rs   # Go-to-definition (~53 lines)
    ├── references.rs   # Find-references (~199 lines)
    ├── hover.rs        # Hover documentation (~55 lines)
    ├── completion.rs   # Code completion (~219 lines)
    ├── diagnostics.rs  # Publish diagnostics (~44 lines)
    └── code_action.rs  # Quick fixes (~171 lines)
```

### Key Components

1. **FixtureDatabase** (`src/fixtures/`)
   - Central data structure for storing fixture definitions and usages
   - Uses `DashMap` for lock-free concurrent access
   - Split into focused modules:
     - `mod.rs` - Database struct and common helpers
     - `types.rs` - Data types (FixtureDefinition, FixtureUsage, etc.)
     - `scanner.rs` - Workspace scanning and venv fixture detection
     - `analyzer.rs` - Python AST parsing and fixture extraction
     - `resolver.rs` - Fixture resolution and query methods
     - `cli.rs` - CLI tree printing for `fixtures list` command
     - `decorators.rs` - Decorator analysis utilities (pytest.fixture, usefixtures, parametrize)
     - `string_utils.rs` - String manipulation utilities (docstring formatting, word extraction)

2. **Backend** (`src/providers/`)
   - LSP server implementation using `tower-lsp`
   - Each LSP feature in its own file:
     - `definition.rs` - Go-to-definition handler
     - `references.rs` - Find-references handler
     - `hover.rs` - Hover documentation handler
     - `completion.rs` - Code completion with auto-add parameter support
     - `diagnostics.rs` - Undeclared fixture warnings
     - `code_action.rs` - Quick fixes to add missing parameters

3. **Main** (`src/main.rs`)
   - `LanguageServer` trait implementation delegating to providers
   - CLI argument parsing with clap
   - LSP server startup and shutdown logic

### Data Structures

```rust
// Core types from src/fixtures.rs:

pub struct FixtureDefinition {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize,  // Character position of name on line
    pub end_char: usize,    // Character position of name end on line
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

// Completion-related types:

pub enum CompletionContext {
    /// Inside a function signature (parameter list) - suggest fixtures as parameters
    FunctionSignature {
        function_name: String,
        function_line: usize,
        is_fixture: bool,
        declared_params: Vec<String>,
    },
    /// Inside a function body - suggest fixtures with auto-add to parameters
    FunctionBody {
        function_name: String,
        function_line: usize,
        is_fixture: bool,
        declared_params: Vec<String>,
    },
    /// Inside @pytest.mark.usefixtures("...") decorator
    UsefixuturesDecorator,
    /// Inside @pytest.mark.parametrize(..., indirect=...) decorator
    ParametrizeIndirect,
}

pub struct ParamInsertionInfo {
    pub line: usize,        // Line number (1-indexed) where to insert
    pub char_pos: usize,    // Character position for insertion
    pub needs_comma: bool,  // Whether to prepend comma before new param
}
```

## Pytest Fixture Resolution Rules

The LSP correctly implements pytest's fixture priority/shadowing rules:

1. **Same file**: Fixtures defined in the same file have highest priority
2. **Closest conftest.py**: Walk up directory tree looking for conftest.py
3. **Virtual environment**: Third-party plugin fixtures (50+ plugins supported including pytest-mock, pytest-asyncio, pytest-flask, pytest-docker, etc.)

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

### src/fixtures/ modules

**Core Methods (resolver.rs):**
- `find_fixture_definition(&self, file_path: &Path, line: u32, char: u32)` - Resolves fixture based on priority rules
- `find_fixture_at_position(&self, file_path: &Path, line: u32, char: u32)` - Finds fixture name at cursor
- `find_references_for_definition(&self, definition: &FixtureDefinition)` - Finds all usages of a specific fixture
- `get_undeclared_fixtures(&self, file_path: &Path)` - Gets all undeclared fixture usages in a file
- `get_completion_context(&self, file_path: &Path, line: u32, char: u32)` - Determines completion context (signature, body, decorator)
- `get_function_param_insertion_info(&self, file_path: &Path, function_line: usize)` - Gets where to insert new parameters
- `get_available_fixtures(&self, file_path: &Path)` - Returns all fixtures available at a file location

**Workspace Scanning (scanner.rs):**
- `scan_workspace(&self, root_path: &Path)` - Walks directory tree, finds test files
- `scan_venv_fixtures(&self, root_path: &Path)` - Scans virtual environment for third-party fixtures

**AST Parsing (analyzer.rs):**
- `analyze_file(&self, file_path: PathBuf, content: &str)` - Parses Python AST, extracts fixtures
- Uses `rustpython-parser` to parse Python files
- Looks for `@pytest.fixture` decorators
- Handles assignment-style fixtures (pytest-mock pattern: `mocker = pytest.fixture()(_mocker)`)
- Extracts function signatures, docstrings, and parameter dependencies
- Walks function body AST to find Name expressions that reference available fixtures

**CLI (cli.rs):**
- `print_fixtures_tree(&self, root_path: &Path, skip_unused: bool, only_unused: bool)` - Prints fixture tree
- `compute_definition_usage_counts(&self, root_path: &Path)` - Computes per-definition usage counts

### src/providers/ modules

**LSP Handlers:**
- `handle_goto_definition()` (definition.rs) - Go-to-definition for fixtures
- `handle_references()` (references.rs) - Find all references, includes definition
- `handle_hover()` (hover.rs) - Shows fixture signature and docstring in Markdown
- `handle_completion()` (completion.rs) - Context-aware fixture completions
- `handle_code_action()` (code_action.rs) - Quick fixes to add missing parameters
- `publish_diagnostics_for_file()` (diagnostics.rs) - Publishes undeclared fixture warnings

**Helper Methods (mod.rs):**
- `uri_to_path()` / `path_to_uri()` - URI/path conversion with symlink handling
- `lsp_line_to_internal()` / `internal_line_to_lsp()` - Line number conversion (0-based vs 1-based)
- `format_fixture_documentation()` - Formats fixture info for hover/completion

### src/main.rs

**LanguageServer trait implementation:**
- `initialize()` - Scans workspace on startup, returns capabilities
- `did_open()`, `did_change()` - Re-analyzes files, publishes diagnostics
- `goto_definition()`, `hover()`, `references()`, `completion()`, `code_action()` - Delegate to providers
- `shutdown()` - Cancels background tasks, forces exit

## Testing

### Test Structure

```
src/
├── fixtures/            # Fixture analysis (~3140 lines total)
├── providers/           # LSP providers (~936 lines total)
├── main.rs              # LanguageServer impl + CLI (~310 lines)
└── lib.rs               # Library exports (~7 lines)

tests/
├── test_fixtures.rs     # FixtureDatabase integration tests (218 tests)
├── test_lsp.rs          # LSP protocol tests (34 tests)
├── test_e2e.rs          # End-to-end integration tests (32 tests)
├── test_decorators.rs   # Decorator utility unit tests (9 tests)
├── test_lsp_performance.rs # LSP performance/caching tests (7 tests)
└── test_project/        # Fixture test files for integration tests
    ├── conftest.py
    ├── test_example.py
    ├── test_parent_usage.py
    ├── api/            # API fixtures and tests
    ├── database/       # Database fixtures with 3-level dependency chain
    ├── utils/          # Utility fixtures with autouse
    ├── integration/    # Scoped fixtures (session, module)
    └── subdir/
        ├── conftest.py
        ├── test_hierarchy.py
        └── test_override.py
```

### Running Tests

```bash
cargo test                          # Run all tests (215 total)
cargo test --test test_fixtures     # Run FixtureDatabase tests (164 tests)
cargo test --test test_lsp         # Run LSP protocol tests (29 tests)
cargo test --test test_e2e         # Run E2E integration tests (22 tests)
RUST_LOG=debug cargo test          # Run with debug logging
```

### Test Coverage

- **310 total tests passing** (as of latest)
  - 218 integration tests in `tests/test_fixtures.rs` (FixtureDatabase API)
  - 34 integration tests in `tests/test_lsp.rs` (LSP protocol handlers)
  - 32 integration tests in `tests/test_e2e.rs` (End-to-end CLI and workspace tests)
  - 9 unit tests in `tests/test_decorators.rs` (Decorator utilities)
  - 7 tests in `tests/test_lsp_performance.rs` (Performance and caching)
  - 5 embedded unit tests in `src/fixtures/string_utils.rs`

**Key test areas:**

**Core Functionality:**
- Fixture definition extraction from various patterns
- Fixture usage detection in test functions and other fixtures
- Fixture priority/shadowing rules (8 comprehensive hierarchy tests)
- Character-position awareness for self-referencing fixtures
- LSP spec compliance (references always include current position)
- Multiline function signatures
- Third-party fixture detection (50+ plugins)
- Undeclared fixture detection (hierarchy-aware)
- Deterministic fixture resolution
- Path normalization and canonicalization
- Deep directory hierarchy support
- Sibling directory isolation
- `@pytest.mark.usefixtures` decorator support
- `@pytest.mark.parametrize` with `indirect=True` fixtures
**Docstring Variations (8 tests):**
- Empty, multiline, single-quoted docstrings
- RST, Google, NumPy documentation styles
- Unicode characters and code blocks in docstrings

**Performance/Scalability (6 tests):**
- 100 fixtures in single file
- 20-level deep fixture dependency chains
- Fixtures with 15 parameters
- Very long function bodies (100 lines)
- 50 files with same fixture name
- Rapid file updates simulation

**Virtual Environment (5 tests):**
- Third-party fixtures in site-packages
- Fixture overrides from plugins
- Multiple plugins with same fixture
- Unused venv fixtures

**Edge Cases (15+ tests):**
- Property, staticmethod, classmethod decorators
- Context managers, multiple decorators
- Modern Python: walrus operator, match statement, exception groups
- Type system: dataclass, NamedTuple, Protocol, Generic types
- Fixtures in if blocks (documented as unsupported)
- Unicode fixture names and docstrings
- Yield fixtures with complex teardown
- Nested test classes
- Variadic parameters (`*args`, `**kwargs`)

**Pytest Markers (9 tests):**
- `@pytest.mark.usefixtures` on functions and classes
- `@pytest.mark.parametrize` with `indirect=True`
- `@pytest.mark.parametrize` with selective `indirect=["fixture"]`

**E2E Tests (32 tests):**
- CLI commands with snapshot testing
- Full workspace scanning
- Performance E2E
- Error handling E2E

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

### CLI Commands

The server supports both LSP mode and standalone CLI commands using `clap` for argument parsing.

**LSP Server Mode (default)**:
```bash
# Start LSP server (reads from stdin, writes to stdout)
pytest-language-server
```

**Fixtures Commands**:
```bash
# List all fixtures in a hierarchical tree view with color-coded output
pytest-language-server fixtures list <path>

# Skip unused fixtures from the output
pytest-language-server fixtures list <path> --skip-unused

# Show only unused fixtures
pytest-language-server fixtures list <path> --only-unused

# Example
pytest-language-server fixtures list tests/test_project
pytest-language-server fixtures list tests/test_project --skip-unused
```

The `fixtures list` command displays:
- File names in **cyan/bold**
- Directory names in **blue/bold**
- Used fixtures in **green** with usage count in **yellow**
- Unused fixtures in **gray/dimmed** with "unused" label
- No indentation for root-level items

Options:
- `--skip-unused`: Filter out unused fixtures from the output
- `--only-unused`: Show only unused fixtures (conflicts with --skip-unused)

**Other Commands**:
```bash
# Show version
pytest-language-server --version

# Show help
pytest-language-server --help
pytest-language-server fixtures --help
pytest-language-server fixtures list --help
```

The CLI uses a subcommand structure to support future expansion:
- `fixtures` namespace - contains fixture-related commands
  - `list` - displays fixtures in tree format
- More namespaces can be added (e.g., `config`, `analyze`, etc.)

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
- `cargo fmt` - Code formatting
- `cargo clippy` - Linting with warnings as errors
- `cargo check` - Build checking
- `cargo audit` - Security vulnerability scanning (pre-push only)
- `cargo deny` - License and security checks (pre-push only)
- General file checks: trailing whitespace, end-of-file fixer, YAML/TOML validation, large file detection, merge conflict detection, mixed line ending detection

Install with: `pre-commit install`

## Common Development Tasks

### Adding a New LSP Feature

1. Add the capability in `main.rs` `initialize()` method's `ServerCapabilities`
2. Create a new file in `src/providers/` (e.g., `new_feature.rs`)
3. Add handler method as `impl Backend` block in the new file
4. Add `pub mod new_feature;` to `src/providers/mod.rs`
5. Add necessary methods to `FixtureDatabase` in appropriate `src/fixtures/` module
6. Implement the `LanguageServer` trait method in `main.rs` delegating to your handler
7. Write integration tests in `tests/test_lsp.rs`
8. Update README.md with feature documentation

### Modifying Fixture Resolution Logic

1. Edit `src/fixtures/resolver.rs` methods:
   - `find_fixture_definition()` for go-to-definition
   - `find_references_for_definition()` for find-references
2. Edit `src/fixtures/analyzer.rs` if changing what fixtures are detected
3. Add test cases to `tests/test_fixtures.rs`
4. Run `cargo test` to ensure all 284 tests pass
5. Consider edge cases: self-referencing fixtures, multiline signatures, conftest.py hierarchy

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
- **clap** (4.5.53) - Command line argument parsing
- **colored** (2.1) - Terminal color output

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

5. **Third-party fixtures**: 50+ popular pytest plugins supported
   - Scanned from virtual environment site-packages
   - Supported plugins include:
     - **Testing frameworks**: pytest-mock, pytest-asyncio, pytest-bdd, pytest-cases
     - **Web frameworks**: pytest-flask, pytest-django, pytest-aiohttp, pytest-tornado, pytest-sanic, pytest-fastapi
     - **HTTP clients**: pytest-httpx
     - **Databases**: pytest-postgresql, pytest-mongodb, pytest-redis, pytest-mysql, pytest-elasticsearch
     - **Infrastructure**: pytest-docker, pytest-kubernetes, pytest-rabbitmq, pytest-celery
     - **ORM/Database tools**: pytest-sqlalchemy, pytest-alembic
     - **Test data**: pytest-factoryboy, pytest-mimesis, pytest-lazy-fixture, pytest-freezegun
     - **Browser testing**: pytest-selenium, pytest-playwright, pytest-splinter
     - **Performance**: pytest-benchmark, pytest-timeout
     - **Execution control**: pytest-xdist, pytest-retry, pytest-repeat, pytest-rerunfailures, pytest-ordering, pytest-dependency, pytest-random-order
     - **Reporting**: pytest-html, pytest-json-report, pytest-metadata, pytest-cov
     - **Development**: pytest-sugar, pytest-emoji, pytest-clarity, pytest-instafail
     - **Environment**: pytest-env, pytest-dotenv
     - **Test selection**: pytest-picked, pytest-testmon, pytest-split
     - And more...

6. **Path normalization and canonicalization** (fixed in v0.5.1)
   - All file paths are canonicalized in `analyze_file()` to handle symlinks and resolve absolute paths
   - This ensures consistent path comparisons in fixture resolution
   - Prevents random fixture selection when paths have different representations
   - Critical for large projects with multiple conftest.py files

7. **Deterministic fixture resolution** (fixed in v0.5.1)
   - When multiple fixture definitions exist in unrelated directories, resolution is deterministic
   - Priority order: same file > conftest hierarchy > third-party (site-packages) > sorted by path
   - Prevents non-deterministic behavior from DashMap iteration order

8. **DashMap deadlock in analyze_file** (fixed in v0.9.0)
   - Fixed deadlock when processing multiple third-party fixtures with same name
   - Issue: `iter_mut()` held read locks while trying to mutate the map
   - Solution: Collect keys first, then process each key individually without holding locks
   - Critical for projects using multiple pytest plugins with overlapping fixture names

9. **Fixtures in if blocks** (known limitation)
   - Fixtures defined inside if statements are not detected
   - This is a documented limitation of the AST traversal logic
   - Workaround: Define fixtures at module level, use if logic inside function body

10. **Fixture scoping in sibling files** (fixed in v0.11.2)
    - Fixtures defined in sibling test files are not accessible to each other
    - Only fixtures in same file, conftest.py hierarchy, or site-packages are resolved
    - Usage counting now uses per-definition scoped counting instead of global name counting
    - Fixes incorrect "used X times" counts when multiple files have parameters with same name

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
- Check that all 284 tests pass: `cargo test`
- Focus on failing tests in `fixtures/` modules (fixture resolution) or `providers/` (LSP handlers)
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

## Editor Extensions

The project includes extensions for three major editors/IDEs:

### VSCode Extension (`extensions/vscode-extension/`)
- **Status**: ✅ Production-ready
- **Language**: TypeScript
- **Build**: Webpack bundled
- **Publishing**: Automated via GitHub Actions to VSCode Marketplace
- **Binaries**: Bundles platform-specific binaries in release
- **Key files**: `package.json`, `src/extension.ts`, `.eslintrc.json`

### Zed Extension (`extensions/zed-extension/`)
- **Status**: ✅ Production-ready
- **Language**: Rust (WASM)
- **Build**: Cargo build to WASM
- **Publishing**: Manual submission to Zed extensions repository
- **Binaries**: Checks PATH first, then auto-downloads from GitHub Releases
- **Key files**: `extension.toml`, `Cargo.toml`, `src/lib.rs`, `PUBLISHING.md`

### IntelliJ Plugin (`extensions/intellij-plugin/`)
- **Status**: ✅ Production-ready (LSP4IJ Integration)
- **Language**: Kotlin
- **Build**: Gradle (Modernized)
- **Publishing**: Automated via GitHub Actions to JetBrains Marketplace
- **Binaries**: Bundles platform-specific binaries in release
- **Key files**: `plugin.xml`, `PytestLanguageServerFactory.kt`
- **Features**: Full LSP support, Settings UI, Auto-download binaries

### Extension Development Notes
- All extensions share the same version number (synchronized via `bump-version.sh`)
- VSCode and IntelliJ bundle binaries; Zed expects user installation
- Extension metadata should point to GitHub releases for changelogs
- Copyright holder: Thiago Bellini Ribeiro (updated as of v0.7.2)

## Additional Resources

- **README.md** - User-facing documentation and setup instructions
- **SECURITY.md** - Security policy and vulnerability reporting
- **RELEASE.md** - Release process documentation
- **EXTENSION_PUBLISHING.md** - Extension publishing guide
- **EXTENSION_SETUP.md** - Extension development setup
- **tests/test_project/** - Example pytest project for testing

## Contributing Guidelines

1. Run tests: `cargo test`
2. Run lints: `cargo clippy`
3. Format code: `cargo fmt`
4. Run security audit: `cargo audit`
5. Install pre-commit hooks: `pre-commit install`
6. Write tests for new features
7. Update AGENTS.md if adding significant architectural changes

## Version History

- **v0.13.0** (December 2025) - Current version
  - Added LSP code completion support for fixtures
  - Context-aware completions: function signatures, function bodies, decorators
  - Auto-add fixture to parameters when completing in function body
  - Trigger character `"` for usefixtures decorator completions
  - Consistent documentation format between hover and completions
  - Test suite: 284 tests (218 unit + 34 LSP + 32 E2E)
- **v0.11.2** (December 2025)
  - Fixed fixture scoping: sibling test files no longer incorrectly share fixtures (#23)
  - Usage counting now uses per-definition scoped counting instead of global name counting
  - Added 4 new scoping tests
  - Test suite: 276 tests (210 unit + 34 LSP + 32 E2E)
- **v0.9.0** (November 2025)
  - Fixed critical DashMap deadlock in analyze_file
  - Added support for 50+ pytest third-party plugins
  - Comprehensive test suite: 272 tests (206 unit + 34 LSP + 32 E2E)
  - Added E2E tests with snapshot testing
  - Added docstring variation, performance, virtual environment, and edge case tests
- **v0.7.2** (November 2025) - Improved extension metadata
- **v0.5.1** (November 2025) - Critical fix for deterministic fixture resolution, path canonicalization, 8 new comprehensive hierarchy tests
- **v0.5.0** (November 2025) - Undeclared fixture diagnostics, code actions (quick fixes), line-aware scoping, LSP compliance improvements
- **v0.4.0** (November 2025) - Character-position aware references, LSP spec compliance
- **v0.3.1** - Previous stable release
- See GitHub releases for full changelog

---

**Last Updated**: v0.13.0 (December 2025)

This document should be updated when making significant architectural changes or adding new features.
