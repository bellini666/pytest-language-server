# pytest-language-server 🔥

[![CI](https://github.com/bellini666/pytest-language-server/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/bellini666/pytest-language-server/actions/workflows/ci.yml)
[![Security Audit](https://github.com/bellini666/pytest-language-server/actions/workflows/security.yml/badge.svg?branch=master)](https://github.com/bellini666/pytest-language-server/actions/workflows/security.yml)
[![PyPI version](https://badge.fury.io/py/pytest-language-server.svg)](https://badge.fury.io/py/pytest-language-server)
[![Downloads](https://static.pepy.tech/badge/pytest-language-server)](https://pepy.tech/project/pytest-language-server)
[![Crates.io](https://img.shields.io/crates/v/pytest-language-server.svg)](https://crates.io/crates/pytest-language-server)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Python Version](https://img.shields.io/pypi/pyversions/pytest-language-server.svg)](https://pypi.org/project/pytest-language-server/)

A blazingly fast Language Server Protocol (LSP) implementation for pytest, built with Rust, with
full support for fixture discovery, go to definition, code completion, find references, hover
documentation, diagnostics, and more!

[![pytest-language-server demo](demo/demo.gif)](demo/demo.mp4)

## Table of Contents

- [Features](#features)
  - [Go to Definition](#-go-to-definition)
  - [Code Completion](#-code-completion)
  - [Find References](#-find-references)
  - [Hover Documentation](#-hover-documentation)
  - [Code Actions (Quick Fixes)](#-code-actions-quick-fixes)
  - [Diagnostics & Quick Fixes](#️-diagnostics--quick-fixes)
  - [Performance](#️-performance)
- [Installation](#installation)
- [Setup](#setup)
  - [Neovim](#neovim)
  - [Zed](#zed)
  - [VS Code](#vs-code)
  - [IntelliJ IDEA / PyCharm](#intellij-idea--pycharm)
  - [Other Editors](#other-editors)
- [Configuration](#configuration)
- [CLI Commands](#cli-commands)
- [Supported Fixture Patterns](#supported-fixture-patterns)
- [Fixture Priority Rules](#fixture-priority-rules)
- [Supported Third-Party Fixtures](#supported-third-party-fixtures)
- [Architecture](#architecture)
- [Development](#development)
- [Security](#security)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgments](#acknowledgments)

## Features

> **Built with AI, maintained with care** 🤖
>
> This project was built with the help of AI coding agents, but carefully reviewed to ensure
> correctness, reliability, security and performance. If you find any issues, please open an issue
> or submit a pull request!

### 🎯 Go to Definition
Jump directly to fixture definitions from anywhere they're used:
- Local fixtures in the same file
- Fixtures in `conftest.py` files
- Third-party fixtures from pytest plugins (pytest-mock, pytest-asyncio, etc.)
- Respects pytest's fixture shadowing/priority rules

### ✨ Code Completion
Smart auto-completion for pytest fixtures:
- **Context-aware**: Only triggers inside test functions and fixture functions
- **Hierarchy-respecting**: Suggests fixtures based on pytest's priority rules (same file > conftest.py > third-party)
- **Rich information**: Shows fixture source file and docstring
- **No duplicates**: Automatically filters out shadowed fixtures
- **Works everywhere**: Completions available in both function parameters and function bodies
- Supports both sync and async functions

### 🔍 Find References
Find all usages of a fixture across your entire test suite:
- Works from fixture definitions or usage sites
- Character-position aware (distinguishes between fixture name and parameters)
- Shows references in all test files
- Correctly handles fixture overriding and hierarchies
- **LSP spec compliant**: Always includes the current position in results

### 📚 Hover Documentation
View fixture information on hover:
- Fixture signature
- Source file location
- Docstring (with proper formatting and dedenting)
- Markdown support in docstrings

### 💡 Code Actions (Quick Fixes)
One-click fixes for common pytest issues:
- **Add missing fixture parameters**: Automatically add undeclared fixtures to function signatures
- **Smart insertion**: Handles both empty and existing parameter lists
- **Editor integration**: Works with any LSP-compatible editor's quick fix menu
- **LSP compliant**: Full support for `CodeActionKind::QUICKFIX`

### ⚠️ Diagnostics & Quick Fixes
Detect and fix common pytest fixture issues with intelligent code actions:

**Undeclared Fixture Detection:**
- Detects when fixtures are used in function bodies but not declared as parameters
- **Line-aware scoping**: Correctly handles local variables assigned later in the function
- **Hierarchy-aware**: Only reports fixtures that are actually available in the current file's scope
- **Works in tests and fixtures**: Detects undeclared usage in both test functions and fixture functions
- Excludes built-in names (`self`, `request`) and actual local variables

**One-Click Quick Fixes:**
- **Code actions** to automatically add missing fixture parameters
- Intelligent parameter insertion (handles both empty and existing parameter lists)
- Works with both single-line and multi-line function signatures
- Triggered directly from diagnostic warnings

Example:
```python
@pytest.fixture
def user_db():
    return Database()

def test_user(user_db):  # ✅ user_db properly declared
    user = user_db.get_user(1)
    assert user.name == "Alice"

def test_broken():  # ⚠️ Warning: 'user_db' used but not declared
    user = user_db.get_user(1)  # 💡 Quick fix: Add 'user_db' fixture parameter
    assert user.name == "Alice"
```

**How to use quick fixes:**
1. Place cursor on the warning squiggle
2. Trigger code actions menu (usually Cmd+. or Ctrl+. in most editors)
3. Select "Add 'fixture_name' fixture parameter"
4. The parameter is automatically added to your function signature

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

### Neovim

Add this to your config:

```lua
vim.lsp.config('pytest_lsp', {
  cmd = { 'pytest-language-server' },
  filetypes = { 'python' },
  root_markers = { 'pyproject.toml', 'setup.py', 'setup.cfg', 'pytest.ini', '.git' },
})

vim.lsp.enable('pytest_lsp')
```

### Zed

Install from the [Zed Extensions Marketplace](https://zed.dev/extensions/pytest-language-server):

1. Open Zed
2. Open the command palette (Cmd+Shift+P / Ctrl+Shift+P)
3. Search for "zed: extensions"
4. Search for "pytest Language Server"
5. Click "Install"

The extension downloads platform-specific binaries from GitHub Releases. If you prefer to use your own installation (via pip, cargo, or brew), place `pytest-language-server` in your PATH.

After installing the extension, you need to enable the language server for Python files. Add the following to your Zed `settings.json`:

```json
{
  "languages": {
    "Python": {
      "language_servers": ["pyright", "pytest-language-server", "..."]
    }
  }
}
```

### VS Code

**The extension includes pre-built binaries - no separate installation required!**

Install from the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=bellini666.pytest-language-server):

1. Open VS Code
2. Go to Extensions (Cmd+Shift+X / Ctrl+Shift+X)
3. Search for "pytest Language Server"
4. Click "Install"

Works out of the box with zero configuration!

### IntelliJ IDEA / PyCharm

**The plugin includes pre-built binaries - no separate installation required!**

Install from the [JetBrains Marketplace](https://plugins.jetbrains.com/plugin/29056-pytest-language-server):

1. Open PyCharm or IntelliJ IDEA
2. Go to Settings/Preferences → Plugins
3. Search for "pytest Language Server"
4. Click "Install"

Requires PyCharm 2024.2+ or IntelliJ IDEA 2024.2+ with Python plugin.

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

### Code Actions / Quick Fixes

Code actions are automatically available on diagnostic warnings. If code actions don't appear in your editor:

1. **Check LSP capabilities**: Ensure your editor supports code actions (most modern editors do)
2. **Enable debug logging**: Use `RUST_LOG=info` to see if actions are being created
3. **Verify diagnostics**: Code actions only appear where there are warnings
4. **Trigger manually**: Use your editor's code action keybinding (Cmd+. / Ctrl+.)

## CLI Commands

In addition to the LSP server mode, pytest-language-server provides useful command-line tools:

### Fixtures List

View all fixtures in your test suite with a hierarchical tree view:

```bash
# List all fixtures
pytest-language-server fixtures list tests/

# Skip unused fixtures
pytest-language-server fixtures list tests/ --skip-unused

# Show only unused fixtures
pytest-language-server fixtures list tests/ --only-unused
```

The output includes:
- **Color-coded display**: Files in cyan, directories in blue, used fixtures in green, unused in gray
- **Usage statistics**: Shows how many times each fixture is used
- **Smart filtering**: Hides files and directories with no matching fixtures
- **Hierarchical structure**: Visualizes fixture organization across conftest.py files

Example output:
```
Fixtures tree for: /path/to/tests

conftest.py (7 fixtures)
├── another_fixture (used 2 times)
├── cli_runner (used 7 times)
├── database (used 6 times)
├── generator_fixture (used 1 time)
├── iterator_fixture (unused)
├── sample_fixture (used 7 times)
└── shared_resource (used 5 times)
subdir/
└── conftest.py (4 fixtures)
    ├── cli_runner (used 7 times)
    ├── database (used 6 times)
    ├── local_fixture (used 4 times)
    └── sample_fixture (used 7 times)
```

This command is useful for:
- **Auditing fixture usage** across your test suite
- **Finding unused fixtures** that can be removed
- **Understanding fixture organization** and hierarchy
- **Documentation** - visualizing available fixtures for developers

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

### @pytest.mark.usefixtures
```python
@pytest.mark.usefixtures("database", "cache")
class TestWithFixtures:
    def test_something(self):
        pass  # database and cache are available
```

### @pytest.mark.parametrize with indirect
```python
@pytest.fixture
def user(request):
    return User(name=request.param)

# All parameters treated as fixtures
@pytest.mark.parametrize("user", ["alice", "bob"], indirect=True)
def test_user(user):
    pass

# Selective indirect fixtures
@pytest.mark.parametrize("user,value", [("alice", 1)], indirect=["user"])
def test_user_value(user, value):
    pass
```

## Fixture Priority Rules

pytest-language-server correctly implements pytest's fixture shadowing rules:
1. **Same file**: Fixtures defined in the same file have highest priority
2. **Closest conftest.py**: Searches parent directories for conftest.py files
3. **Virtual environment**: Third-party plugin fixtures

### Fixture Overriding

The LSP correctly handles complex fixture overriding scenarios:

```python
# conftest.py (parent)
@pytest.fixture
def cli_runner():
    return "parent runner"

# tests/conftest.py (child)
@pytest.fixture
def cli_runner(cli_runner):  # Overrides parent
    return cli_runner  # Uses parent

# tests/test_example.py
def test_example(cli_runner):  # Uses child
    pass
```

When using find-references:
- Clicking on the **function name** `def cli_runner(...)` shows references to the child fixture
- Clicking on the **parameter** `cli_runner(cli_runner)` shows references to the parent fixture
- Character-position aware to distinguish between the two

## Supported Third-Party Fixtures

Automatically discovers fixtures from **50+ popular pytest plugins**, including:

- **Testing frameworks**: pytest-mock, pytest-asyncio, pytest-bdd, pytest-cases
- **Web frameworks**: pytest-flask, pytest-django, pytest-aiohttp, pytest-tornado, pytest-sanic, pytest-fastapi
- **HTTP clients**: pytest-httpx
- **Databases**: pytest-postgresql, pytest-mongodb, pytest-redis, pytest-mysql, pytest-elasticsearch
- **Infrastructure**: pytest-docker, pytest-kubernetes, pytest-rabbitmq, pytest-celery
- **Browser testing**: pytest-selenium, pytest-playwright, pytest-splinter
- **Performance**: pytest-benchmark, pytest-timeout
- **Test data**: pytest-factoryboy, pytest-freezegun, pytest-mimesis
- And many more...

The server automatically scans your virtual environment for any pytest plugin and makes their fixtures available.

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

## Security

Security is a priority. This project includes:
- Automated dependency vulnerability scanning (cargo-audit)
- License compliance checking (cargo-deny)
- Daily security audits in CI/CD
- Dependency review on pull requests
- Pre-commit security hooks

See [SECURITY.md](SECURITY.md) for our security policy and how to report vulnerabilities.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Development Setup

1. Install pre-commit hooks:
   ```bash
   pre-commit install
   ```

2. Run security checks locally:
   ```bash
   cargo audit
   cargo clippy
   cargo test
   ```

## License

MIT License - see LICENSE file for details.

## Acknowledgments

Built with:
- [tower-lsp](https://github.com/ebkalderon/tower-lsp) - LSP framework
- [rustpython-parser](https://github.com/RustPython/RustPython) - Python AST parsing
- [tokio](https://tokio.rs/) - Async runtime

Special thanks to the pytest team for creating such an amazing testing framework.

---

**Made with ❤️ and Rust. Blazingly fast 🔥**

*Built with AI assistance, maintained with care.*
