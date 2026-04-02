# pytest Language Server for VS Code

A blazingly fast Language Server Protocol implementation for pytest fixtures, written in Rust.

## Features

- **Go to Definition**: Jump to fixture definitions from usage
- **Go to Implementation**: Navigate to yield statements in generator fixtures
- **Call Hierarchy**: Explore fixture dependencies (incoming/outgoing calls)
- **Code Completion**: Smart auto-completion for pytest fixtures with context-aware suggestions
- **Find References**: Find all usages of a fixture
- **Hover Documentation**: View fixture signatures and docstrings
- **Document Symbols**: Navigate fixtures within a file using the outline view
- **Workspace Symbols**: Search for fixtures across your entire workspace
- **Code Lens**: See fixture usage counts directly above definitions
- **Inlay Hints**: See fixture return types inline next to parameters
- **Diagnostics**: Warnings for undeclared fixtures, scope mismatches, and circular dependencies
- **Code Actions**: Quick fixes to add missing fixture parameters with type annotations, cursor-based type annotation insertion, and file-wide "annotate all fixtures" — with isort-aware import management
- **Fixture Priority**: Correctly handles pytest's fixture shadowing rules

## Configuration

### Custom Executable Path

By default, the extension uses the bundled pytest-language-server binary. You can configure a custom path:

```json
{
  "pytestLanguageServer.executable": "/path/to/pytest-language-server"
}
```

To use the system version from PATH:

```json
{
  "pytestLanguageServer.executable": "pytest-language-server"
}
```

### Debug Logging

Enable LSP communication tracing:

```json
{
  "pytestLanguageServer.trace.server": "verbose"
}
```

## Requirements

None! The extension includes pre-built binaries for:
- macOS (Intel and Apple Silicon)
- Linux (x86_64 and ARM64)
- Windows (x86_64)

Alternatively, you can install pytest-language-server from PyPI:

```bash
pip install pytest-language-server
```

## Usage

The language server automatically activates for Python files in your workspace. No additional configuration is needed.

## Issues

Report issues at: https://github.com/bellini666/pytest-language-server/issues

## License

MIT
