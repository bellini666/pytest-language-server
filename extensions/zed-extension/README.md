# pytest Language Server Extension for Zed

This is a [Zed](https://zed.dev) extension that provides support for the [pytest-language-server](https://github.com/bellini666/pytest-language-server).

## Features

- **Go to Definition**: Jump directly to fixture definitions from anywhere they're used
- **Code Completion**: Smart auto-completion for pytest fixtures with context-aware suggestions
- **Find References**: Find all usages of a fixture across your entire test suite
- **Hover Documentation**: View fixture information including signature, location, and docstring
- **Diagnostics**: Warnings for undeclared fixtures used in function bodies
- **Code Actions**: Quick fixes to add missing fixture parameters
- **Fixture Priority**: Correctly handles pytest's fixture shadowing rules
- **Simple Setup**: Uses your existing pytest-language-server installation
- **Cross-platform**: Works on macOS, Linux, and Windows

## Installation

1. Open Zed
2. Open the command palette (Cmd+Shift+P / Ctrl+Shift+P)
3. Search for "zed: extensions"
4. Search for "pytest Language Server"
5. Click "Install"

**That's it!** The extension automatically downloads the appropriate binary for your platform on first use.

## How It Works

The extension intelligently handles the language server binary:

1. **Check PATH**: First, it looks for `pytest-language-server` in your PATH (if you installed via pip, cargo, or brew)
2. **Auto-Download**: If not found, it automatically downloads the pre-built binary from GitHub Releases for your platform
3. **Cache**: Downloaded binaries are cached in the extension directory for future use

This means you can either:
- **Do nothing** - the extension handles everything automatically
- **Install manually** (via pip/cargo/brew) - the extension will use your installation

## Configuration

### Tips

- **Custom Binary**: If you have `pytest-language-server` in your PATH, the extension will prioritize that over the auto-downloaded binary
- **Offline Use**: Download the binary once while online, and it will be cached for offline use
- **Manual Installation**: Install via `pip install pytest-language-server`, `cargo install pytest-language-server`, or `brew install pytest-language-server`
- **Logging**: Set the `RUST_LOG` environment variable to enable debug logging from the language server

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
