# TODO for Zed Extension

## Design Philosophy

This extension follows the **simple approach**: users install `pytest-language-server` via their preferred method (pip, uv, cargo, homebrew), and the extension just finds it in PATH.

This is simpler to maintain and matches how most LSP extensions work (e.g., rust-analyzer, pyright, etc.).

## Future Improvements

### Nice-to-Have Features

1. **Better Error Messages**
   - Detect if user is in a virtual environment without pytest-language-server
   - Suggest installation command based on detected package manager
   - Link to installation docs

2. **Configuration Options**
   - Allow users to specify custom command-line arguments
   - Support for `RUST_LOG` environment variable configuration
   - Workspace-specific settings

3. **Status Indicators**
   - Show when the language server is starting/ready
   - Display version information
   - Health check diagnostics

### Won't Implement

~~**Automatic Binary Downloads**~~
- Adds complexity for minimal benefit
- Users already have pip/cargo/brew installed
- Virtual environment integration works better with pip install
- Standalone binaries would need to be built and maintained

## Contributing

If you want to add features, please:
1. Keep it simple
2. Test with different installation methods (pip, cargo, brew)
3. Test in virtual environments
4. Consider cross-platform compatibility
