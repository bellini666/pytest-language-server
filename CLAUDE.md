# CLAUDE.md - AI Agent Guide

## Workflow Rules

**IMPORTANT**: Never commit or push without explicit user confirmation. Always ask first.

## Project Overview

**pytest-language-server** is a Rust LSP for pytest fixtures providing go-to-definition, find-references, hover, completions, diagnostics, and more.

- **Language**: Rust (Edition 2021, MSRV 1.85)
- **Framework**: `tower-lsp-server` + `rustpython-parser`
- **Run tests**: `cargo test`
- **Lint**: `cargo clippy`
- **Debug**: `RUST_LOG=debug cargo run`

## Architecture

```
src/
├── main.rs             # LanguageServer trait impl + CLI entry point
├── lib.rs              # Library exports
├── config/mod.rs       # Config from pyproject.toml [tool.pytest-language-server]
├── fixtures/           # Core analysis engine
│   ├── mod.rs          # FixtureDatabase struct (DashMap-based concurrent storage)
│   ├── types.rs        # FixtureDefinition, FixtureUsage, TypeImportSpec, etc.
│   ├── analyzer.rs     # Python AST parsing, fixture extraction, return-type import resolution
│   ├── imports.rs      # Import handling, is_stdlib_module(), build_name_to_import_map(), file_path_to_module_path()
│   ├── resolver.rs     # Fixture resolution with pytest priority rules
│   ├── scanner.rs      # Workspace + venv scanning
│   └── cli.rs          # CLI commands (fixtures list/unused)
└── providers/          # LSP handlers (one file per feature)
    ├── mod.rs          # Backend struct, URI/path helpers
    ├── code_action.rs  # Code actions: quickfix, source.pytest-lsp, source.fixAll.pytest-lsp + isort-aware import insertion
    ├── definition.rs, references.rs, hover.rs, completion.rs, ...
```

**Key pattern**: `FixtureDatabase` in `src/fixtures/` handles all data; `Backend` in `src/providers/` delegates LSP requests to it.

## Critical Knowledge

### Code Action Kinds
The code action provider (`src/providers/code_action.rs`) emits three kinds:

| Kind | Trigger | Behaviour |
|------|---------|-----------|
| `quickfix` | `undeclared-fixture` diagnostic | Adds missing fixture param with type annotation + import |
| `source.pytest-lsp` | Cursor on unannotated fixture param | Adds `: ReturnType` + import for that fixture |
| `source.fixAll.pytest-lsp` | Anywhere in file | Adds all missing type annotations + imports in one edit |

**Import insertion** is isort/ruff-aware:
- `parse_import_groups()` classifies existing import blocks as stdlib vs third-party
- `emit_kind_import_edits()` inserts into the correct group with proper blank-line separators
- `find_matching_from_import_line()` merges names into existing `from X import Y` lines
- `build_import_edits()` orchestrates deduplication, skip-if-already-imported, and group routing

### TypeImportSpec & Return-Type Import Resolution
`TypeImportSpec` (in `types.rs`) captures `check_name` + `import_statement` for each type used in a fixture's return annotation. Resolved at analysis time:
1. `build_name_to_import_map()` (in `imports.rs`) builds a name→spec map from all imports in the fixture file (including stdlib/typing)
2. `resolve_return_type_imports()` (in `analyzer.rs`) tokenises the return type string, skips builtins, looks up each identifier in the import map, and falls back to locally-defined names via `file_path_to_module_path()`
3. Results are stored in `FixtureDefinition::return_type_imports` for use by code actions

`is_stdlib_module()` is exposed as a free function in `imports.rs` (re-exported from `mod.rs`) so both `FixtureDatabase` methods and the code-action provider can classify imports without coupling.

### Pytest Fixture Resolution Priority
1. Same file (highest)
2. Closest conftest.py (walk up directory tree)
3. Plugin fixtures (pytest11 entry points, e.g. workspace editable installs)
4. Third-party from venv site-packages (lowest)

### Self-Referencing Fixtures
```python
@pytest.fixture
def cli_runner(cli_runner):  # Parameter refers to PARENT fixture
    return cli_runner
```
Position matters: cursor on function name → child; cursor on parameter → parent. Uses `start_char`/`end_char` in `FixtureUsage`.

### Line Number Conventions
- LSP uses 0-based lines
- Internal storage uses 1-based lines
- Use `lsp_line_to_internal()` / `internal_line_to_lsp()` helpers

### DashMap Deadlock Prevention
Never hold `.get()` references across `analyze_file()` calls. Scope references in blocks:
```rust
// CORRECT
{
    let entry = db.definitions.get("name").unwrap();
    // use entry
}  // Reference dropped
db.analyze_file(...);  // Safe
```

## Common Tasks

### Adding a New LSP Feature
1. Add capability in `main.rs` `initialize()` → `ServerCapabilities`
2. Create `src/providers/new_feature.rs`
3. Add `pub mod new_feature;` to `src/providers/mod.rs`
4. Implement handler method in new file
5. Wire up in `main.rs` LanguageServer trait impl
6. Add tests in `tests/test_lsp.rs`

### Adding a New Code Action Kind
1. Register the kind in `main.rs` `initialize()` → `code_action_kinds`
2. Add a `const` for the kind in `src/providers/code_action.rs`
3. Gate the new logic behind `kind_requested(&context.only, &YOUR_KIND)`
4. Build `TextEdit`s for the action; use `build_import_edits()` if imports are needed
5. Add unit tests in the `mod tests` block inside `code_action.rs`
6. Add integration tests in `tests/test_lsp.rs`

### Version Bumping
**Always use the script** (updates Cargo.toml, pyproject.toml, extensions):
```bash
./bump-version.sh X.Y.Z
```

### Extension Documentation
When adding new LSP features, update the feature lists in all extension READMEs:
- `extensions/vscode-extension/README.md`
- `extensions/intellij-plugin/README.md`
- `extensions/zed-extension/README.md`

Keep them in sync with the main `README.md` features section.

### Imported Fixtures
Fixtures imported via star imports in `conftest.py` are discovered:
```python
# conftest.py
from .fixtures import *  # Fixtures from fixtures.py are now available
```

The scanner:
1. First scans `conftest.py` and test files
2. Then iteratively discovers modules imported by conftest files
3. Handles transitive imports (A → B → C)

Performance optimizations:
- `imported_fixtures_cache` stores results with dual invalidation (content hash + definitions version)
- `is_standard_library_module()` uses O(1) HashSet lookup instead of linear array search
- Iterative module scanning prevents redundant AST parsing

## Known Limitations

- Fixtures defined inside `if` blocks are not detected
- Only scans `conftest.py`, `test_*.py`, `*_test.py` files (but also scans modules imported by conftest)

## Tests

Run `cargo test`. Test files:

**Integration tests** (`tests/`):
- `tests/test_fixtures.rs` - FixtureDatabase unit tests
- `tests/test_lsp.rs` - LSP protocol tests (includes code action, hover, TypeImportSpec tests)
- `tests/test_lsp_performance.rs` - LSP performance/stress tests
- `tests/test_e2e.rs` - End-to-end CLI tests
- `tests/test_config.rs` - Configuration loading tests
- `tests/test_decorators.rs` - Decorator recognition tests
- `tests/test_project/` - Sample pytest project for testing

**Inline unit tests** (`#[cfg(test)] mod tests`):
- `src/providers/code_action.rs` - Import grouping, merging, sorting, and edit generation
- `src/providers/completion.rs` - Completion context detection
- `src/fixtures/imports.rs` - `file_path_to_module_path`, import extraction
- `src/fixtures/scanner.rs` - Workspace/venv scanning
- `src/fixtures/string_utils.rs` - Parameter annotation parsing
- `src/config/mod.rs` - Config parsing from pyproject.toml
