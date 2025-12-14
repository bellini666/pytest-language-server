# AGENTS.md - AI Agent Guide

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
│   ├── types.rs        # FixtureDefinition, FixtureUsage, etc.
│   ├── analyzer.rs     # Python AST parsing, fixture extraction
│   ├── resolver.rs     # Fixture resolution with pytest priority rules
│   ├── scanner.rs      # Workspace + venv scanning
│   └── cli.rs          # CLI commands (fixtures list/unused)
└── providers/          # LSP handlers (one file per feature)
    ├── mod.rs          # Backend struct, URI/path helpers
    ├── definition.rs, references.rs, hover.rs, completion.rs, ...
```

**Key pattern**: `FixtureDatabase` in `src/fixtures/` handles all data; `Backend` in `src/providers/` delegates LSP requests to it.

## Critical Knowledge

### Pytest Fixture Resolution Priority
1. Same file (highest)
2. Closest conftest.py (walk up directory tree)
3. Third-party from venv site-packages (lowest)

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

### Version Bumping
**Always use the script** (updates Cargo.toml, pyproject.toml, extensions):
```bash
./bump-version.sh X.Y.Z
```

## Known Limitations

- Fixtures defined inside `if` blocks are not detected
- Only scans `conftest.py`, `test_*.py`, `*_test.py` files

## Tests

Run `cargo test`. Test files:
- `tests/test_fixtures.rs` - FixtureDatabase unit tests
- `tests/test_lsp.rs` - LSP protocol tests
- `tests/test_e2e.rs` - End-to-end CLI tests
- `tests/test_project/` - Sample pytest project for testing
