# Zed Extension for pytest-language-server

## What was created

A complete Zed editor extension for the pytest-language-server has been created in the `zed-extension/` directory.

## Files Created

```
zed-extension/
├── extension.toml          # Extension metadata and configuration
├── Cargo.toml             # Rust dependencies for the extension
├── src/
│   └── lib.rs             # Extension implementation
├── .gitignore             # Git ignore rules
├── README.md              # User-facing documentation
├── PUBLISHING.md          # Guide for publishing to Zed registry
└── TODO.md                # Future improvements

```

## Features

The extension provides:

1. **Automatic Detection**: Finds `pytest-language-server` in your PATH
2. **Manual Configuration**: Supports custom binary paths via settings
3. **Future-Ready**: Code prepared for automatic binary downloads (requires standalone binaries in releases)
4. **Cross-Platform**: Supports macOS, Linux, and Windows
5. **Standard LSP Features**: All pytest LSP features work (go to definition, find references, hover docs)

## Current Status

✅ Extension code complete and compiles successfully
✅ User documentation written
✅ Publishing guide created
⏳ Requires manual installation of `pytest-language-server` (via pip, uv, cargo, or homebrew)
⏳ Not yet published to Zed registry (requires PR to zed-industries/extensions)

## Installation (Current)

Users need to:
1. Install `pytest-language-server` manually (see main README)
2. Install the extension in Zed (once published)

## Next Steps

### For Publishing

1. **Publish to Zed Registry**: Follow the steps in `PUBLISHING.md`
2. **Test Locally**: Use "zed: install dev extension" in Zed to test before publishing

### For Future Improvements

See `TODO.md` for details on adding standalone binary releases, which would enable:
- Automatic download and installation of the language server
- No manual installation required
- Automatic updates when new versions are released

## Testing Locally

Before publishing, you can test the extension:

```bash
# In Zed:
# 1. Open command palette (Cmd+Shift+P)
# 2. Search for "zed: install dev extension"
# 3. Select the zed-extension directory
# 4. Open a Python project with pytest
# 5. Test features like "Go to Definition" on fixtures
```

## How It Works

1. When a Python file is opened, Zed loads the pytest-lsp extension
2. The extension looks for `pytest-language-server` in:
   - User-configured path (from settings)
   - System PATH
   - Cached download (when standalone binaries are available)
3. Once found, it starts the language server
4. All LSP features are provided by the pytest-language-server binary

## Documentation Updated

The main README.md has been updated to include Zed setup instructions in the "Setup" section.
