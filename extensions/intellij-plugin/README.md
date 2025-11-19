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

The plugin uses the bundled pytest-language-server binary by default. No configuration is needed.

### Optional: Use Custom Binary

If you want to use your own installation instead of the bundled binary, you can configure it via JVM properties in your IDE's VM options (Help → Edit Custom VM Options):

**Option 1: Use system PATH**
```
-Dpytest.lsp.useSystemPath=true
```

**Option 2: Specify exact path**
```
-Dpytest.lsp.executable=/path/to/pytest-language-server
```

## Requirements

None! The plugin includes pre-built binaries for:
- macOS (Intel and Apple Silicon)
- Linux (x86_64 and ARM64)
- Windows (x86_64)

The plugin works out of the box with no additional setup required.

## Usage

The language server automatically activates for Python files in your workspace. No additional configuration is needed.

## Development

### Prerequisites

- Java 17 or later
- Gradle (wrapper included)
- pytest-language-server binary installed (for testing)

### Building

```bash
./gradlew buildPlugin
```

The plugin ZIP will be in `build/distributions/`.

### Testing

First, install the pytest-language-server binary so it's available in your PATH:

```bash
# From the project root directory
cargo install --path .
```

Then launch the IDE with the plugin:

```bash
./gradlew runIde
```

This will launch an IDE with the plugin installed for testing.

**Note**: By default, the plugin uses the bundled binary. For development testing:
- Install the binary: `cargo install --path .` (from project root)
- Configure the plugin to use system PATH: `-Dpytest.lsp.useSystemPath=true`

## Issues

Report issues at: https://github.com/bellini666/pytest-language-server/issues

## License

MIT
