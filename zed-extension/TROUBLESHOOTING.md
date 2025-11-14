# Troubleshooting Zed Extension Development

## Compilation Error: "can't find crate for `core`"

If you see this error when trying to install or build the extension:

```
Error: Failed to install dev extension: failed to compile Rust extension
error[E0463]: can't find crate for `core`
  = note: the `wasm32-wasip1` target may not be installed
```

### Cause

This happens when you have Rust installed via Homebrew (or another package manager) and it's taking precedence over rustup in your PATH.

### Solution

Zed extensions **require** Rust to be installed via rustup (not Homebrew). You have two options:

#### Option 1: Fix your PATH (Recommended)

Make sure `~/.cargo/bin` is at the beginning of your PATH:

```bash
# For bash (~/.bashrc or ~/.bash_profile)
export PATH="$HOME/.cargo/bin:$PATH"

# For zsh (~/.zshrc)
export PATH="$HOME/.cargo/bin:$PATH"

# For fish (~/.config/fish/config.fish)
set -gx PATH $HOME/.cargo/bin $PATH
```

Then restart your terminal and verify:

```bash
which rustc
# Should output: /Users/YOUR_USERNAME/.cargo/bin/rustc

which cargo  
# Should output: /Users/YOUR_USERNAME/.cargo/bin/cargo
```

#### Option 2: Uninstall Homebrew Rust

If you don't need the Homebrew version:

```bash
brew uninstall rust
```

### Verification

After fixing your PATH, verify the wasm32-wasip1 target is installed:

```bash
rustup target list --installed | grep wasm32-wasip1
```

If not listed, install it:

```bash
rustup target add wasm32-wasip1
```

### Building the Extension

Once your PATH is correct, you can build the extension:

```bash
cd zed-extension
cargo build --release --target wasm32-wasip1
```

The compiled WASM file will be at:
```
target/wasm32-wasip1/release/zed_pytest_lsp.wasm
```

## Installation in Zed

After fixing the PATH issue:

1. **Restart Zed** (important - Zed needs to pick up the new PATH)
2. Open command palette (Cmd+Shift+P)
3. Search for "zed: install dev extension"
4. Select the `zed-extension` directory
5. The extension should now compile and install successfully

## Verifying Installation

Once installed, open a Python file in a pytest project. You should see:

1. The extension loaded in Zed's extension list
2. LSP features working (if pytest-language-server is installed)
3. No compilation errors in Zed's logs

## Getting Help

If you still have issues:

1. Check Zed's logs: Command palette → "zed: open log"
2. Try building manually (as shown above) to see the full error
3. Ensure you're using the latest stable Rust: `rustup update stable`
4. File an issue at https://github.com/bellini666/pytest-language-server/issues
