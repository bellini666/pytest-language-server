# pytest Language Server for IntelliJ/PyCharm

A blazingly fast Language Server Protocol implementation for pytest fixtures, written in Rust.

## Features

- **Go to Definition**: Jump to fixture definitions from usage
- **Find References**: Find all usages of a fixture
- **Hover Documentation**: View fixture signatures and docstrings
- **Diagnostics**: Warnings for undeclared fixtures used in function bodies
- **Code Actions**: Quick fixes to add missing fixture parameters
- **Fixture Priority**: Correctly handles pytest's fixture shadowing rules

## Configuration

### Custom Executable Path

By default, the plugin uses the bundled pytest-language-server binary. You can configure a custom path by setting the JVM property:

```
-Dpytest.lsp.executable=/path/to/pytest-language-server
```

Add this to your IDE's VM options (Help → Edit Custom VM Options).

## Requirements

None! The plugin includes pre-built binaries for:
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
