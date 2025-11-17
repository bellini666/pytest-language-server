# pytest Language Server Extension for Zed

This is a [Zed](https://zed.dev) extension that provides support for the [pytest-language-server](https://github.com/bellini666/pytest-language-server).

## Features

- **Go to Definition**: Jump directly to fixture definitions from anywhere they're used
- **Find References**: Find all usages of a fixture across your entire test suite
- **Hover Documentation**: View fixture information including signature, location, and docstring
- **Simple Setup**: Uses your existing pytest-language-server installation
- **Cross-platform**: Works on macOS, Linux, and Windows

## Installation

### Prerequisites

First, install the pytest-language-server binary using one of these methods:

```bash
# Using uv (recommended)
uv tool install pytest-language-server

# Or with pip
pip install pytest-language-server

# Or with pipx (isolated environment)
pipx install pytest-language-server

# Or with Homebrew (macOS/Linux)
brew install bellini666/tap/pytest-language-server

# Or with Cargo
cargo install pytest-language-server
```

### Install the Extension

1. Open Zed
2. Open the command palette (Cmd+Shift+P / Ctrl+Shift+P)
3. Search for "zed: extensions"
4. Search for "pytest Language Server"
5. Click "Install"

## Configuration

The extension automatically detects `pytest-language-server` if it's in your PATH.

### Tips

- **Virtual Environments**: The extension respects your Python virtual environment, so install pytest-language-server in the same venv as your project
- **Multiple Python Versions**: The extension uses whichever `pytest-language-server` is first in your PATH
- **Logging**: Set the `RUST_LOG` environment variable before starting Zed to enable debug logging from the language server

### Environment Variables

You can control logging verbosity by setting the `RUST_LOG` environment variable:

```json
{
  "lsp": {
    "pytest-lsp": {
      "initialization_options": {
        "environment": {
          "RUST_LOG": "debug"
        }
      }
    }
  }
}
```

## Development

To develop this extension locally:

1. Clone this repository
2. **Install Rust via [rustup](https://rustup.rs)** (required - Homebrew Rust won't work)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
3. Ensure `~/.cargo/bin` is in your PATH before other Rust installations
4. Install the wasm32-wasip1 target:
   ```bash
   rustup target add wasm32-wasip1
   ```
5. Open Zed
6. Run the command "zed: install dev extension"
7. Select the `zed-extension` directory from this repository

**Troubleshooting:** If you get compilation errors, see [TROUBLESHOOTING.md](./TROUBLESHOOTING.md)

## License

MIT License - see [LICENSE](../LICENSE) file for details.

## Links

- [pytest-language-server](https://github.com/bellini666/pytest-language-server)
- [Zed Extensions Documentation](https://zed.dev/docs/extensions)
