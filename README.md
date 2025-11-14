# pytest-language-server 🔥

> **Shamelessly vibed into existence** 🤖✨
>
> This entire LSP implementation was built from scratch in a single AI-assisted coding session.
> No template. No boilerplate. Just pure vibes and Rust. That's right - a complete, working
> Language Server Protocol implementation for pytest, vibed into reality through the power of
> modern AI tooling. Even this message about vibing was vibed into existence.

A blazingly fast Language Server Protocol (LSP) implementation for pytest, built with Rust.

## Features

### 🎯 Go to Definition
Jump directly to fixture definitions from anywhere they're used:
- Local fixtures in the same file
- Fixtures in `conftest.py` files
- Third-party fixtures from pytest plugins (pytest-mock, pytest-asyncio, etc.)
- Respects pytest's fixture shadowing/priority rules

### 🔍 Find References
Find all usages of a fixture across your entire test suite:
- Works from fixture definitions or usage sites
- Character-position aware (distinguishes between fixture name and parameters)
- Shows references in all test files

### 📚 Hover Documentation
View fixture information on hover:
- Fixture signature
- Source file location
- Docstring (with proper formatting and dedenting)
- Markdown support in docstrings

### ⚡️ Performance
Built with Rust for maximum performance:
- Fast workspace scanning with concurrent file processing
- Efficient AST parsing using rustpython-parser
- Lock-free data structures with DashMap
- Minimal memory footprint

## Installation

Choose your preferred installation method:

### 📦 PyPI (Recommended)

The easiest way to install for Python projects:

```bash
# Using uv (recommended)
uv tool install pytest-language-server

# Or with pip
pip install pytest-language-server

# Or with pipx (isolated environment)
pipx install pytest-language-server
```

### 🍺 Homebrew (macOS/Linux)

Install via Homebrew for system-wide availability:

```bash
brew install bellini666/tap/pytest-language-server
```

To add the tap first:
```bash
brew tap bellini666/tap https://github.com/bellini666/pytest-language-server
brew install pytest-language-server
```

### 🦀 Cargo (Rust)

Install from crates.io if you have Rust installed:

```bash
cargo install pytest-language-server
```

### 📥 Pre-built Binaries

Download pre-built binaries from the [GitHub Releases](https://github.com/bellini666/pytest-language-server/releases) page.

Available for:
- **Linux**: x86_64, aarch64, armv7 (glibc and musl)
- **macOS**: Intel and Apple Silicon
- **Windows**: x64 and x86

### 🔨 From Source

Build from source for development or customization:

```bash
git clone https://github.com/bellini666/pytest-language-server
cd pytest-language-server
cargo build --release
```

The binary will be at `target/release/pytest-language-server`.

## Setup

### Neovim (with nvim-lspconfig)

```lua
require'lspconfig'.pytest_lsp.setup{
  cmd = { "pytest-language-server" },
  filetypes = { "python" },
  root_dir = function(fname)
    return require'lspconfig'.util.root_pattern('pyproject.toml', 'setup.py', 'setup.cfg', 'pytest.ini')(fname)
  end,
}
```

### VS Code

Install the extension from the marketplace (coming soon) or configure manually:

```json
{
  "pytest-language-server.enable": true,
  "pytest-language-server.path": "pytest-language-server"
}
```

### Other Editors

Any editor with LSP support can use pytest-language-server. Configure it to run the `pytest-language-server` command.

## Configuration

### Logging

Control log verbosity with the `RUST_LOG` environment variable:

```bash
# Minimal logging (default)
RUST_LOG=warn pytest-language-server

# Info level
RUST_LOG=info pytest-language-server

# Debug level (verbose)
RUST_LOG=debug pytest-language-server

# Trace level (very verbose)
RUST_LOG=trace pytest-language-server
```

Logs are written to stderr, so they won't interfere with LSP communication.

### Virtual Environment Detection

The server automatically detects your Python virtual environment:
1. Checks for `.venv/`, `venv/`, or `env/` in your project root
2. Falls back to `$VIRTUAL_ENV` environment variable
3. Scans third-party pytest plugins for fixtures

## Supported Fixture Patterns

### Decorator Style
```python
@pytest.fixture
def my_fixture():
    """Fixture docstring."""
    return 42
```

### Assignment Style (pytest-mock)
```python
mocker = pytest.fixture()(_mocker)
```

### Async Fixtures
```python
@pytest.fixture
async def async_fixture():
    return await some_async_operation()
```

### Fixture Dependencies
```python
@pytest.fixture
def fixture_a():
    return "a"

@pytest.fixture
def fixture_b(fixture_a):  # Go to definition works on fixture_a
    return fixture_a + "b"
```

## Fixture Priority Rules

pytest-language-server correctly implements pytest's fixture shadowing rules:
1. **Same file**: Fixtures defined in the same file have highest priority
2. **Closest conftest.py**: Searches parent directories for conftest.py files
3. **Virtual environment**: Third-party plugin fixtures

## Supported Third-Party Fixtures

Automatically discovers fixtures from popular pytest plugins:
- **pytest-mock**: `mocker`, `class_mocker`
- **pytest-asyncio**: `event_loop`
- **pytest-django**: Database fixtures
- **pytest-cov**: Coverage fixtures
- And any other pytest plugin in your environment

## Architecture

- **Language**: Rust 🦀
- **LSP Framework**: tower-lsp
- **Parser**: rustpython-parser
- **Concurrency**: tokio async runtime
- **Data Structures**: DashMap for lock-free concurrent access

## Development

### Prerequisites

- Rust 1.83+ (2021 edition)
- Python 3.10+ (for testing)

### Building

```bash
cargo build --release
```

### Running Tests

```bash
cargo test
```

### Logging During Development

```bash
RUST_LOG=debug cargo run
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT License - see LICENSE file for details.

## Acknowledgments

Built with:
- [tower-lsp](https://github.com/ebkalderon/tower-lsp) - LSP framework
- [rustpython-parser](https://github.com/RustPython/RustPython) - Python AST parsing
- [tokio](https://tokio.rs/) - Async runtime

Special thanks to the pytest team for creating such an amazing testing framework.

---

**Made with ❤️ and Rust. Shamelessly vibed into existence. Blazingly fast. 🔥**

*When you need a pytest LSP and the vibes are just right.* ✨
