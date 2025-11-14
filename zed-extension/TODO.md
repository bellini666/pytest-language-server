# TODO for Zed Extension

## Future Improvements

### Standalone Binary Releases

Currently, the GitHub releases only include Python wheels. To enable automatic installation in the Zed extension, we should add standalone binary releases to the release workflow.

**Required changes to `.github/workflows/release.yml`:**

1. Add a new job to build standalone binaries for each platform:
   - Linux (x86_64, aarch64, armv7) - with musl for better compatibility
   - macOS (x86_64, aarch64)
   - Windows (x86_64, x86)

2. Package each binary as a tar.gz (Linux/macOS) or zip (Windows) archive

3. Upload these as release assets alongside the Python wheels

**Binary naming convention:**
```
pytest-language-server-v{version}-{arch}-{os}.{extension}
```

Examples:
- `pytest-language-server-v0.3.0-x86_64-apple-darwin.tar.gz`
- `pytest-language-server-v0.3.0-aarch64-unknown-linux-musl.tar.gz`
- `pytest-language-server-v0.3.0-x86_64-pc-windows-msvc.zip`

Once this is implemented, the Zed extension will automatically download and manage the language server binary without requiring manual installation.

### Reference Implementation

See other Rust-based LSP extensions for examples:
- [just-lsp Zed extension](https://github.com/sectore/zed-just-ls)
- [Ruff Zed extension](https://github.com/zed-industries/extensions)

The extension code is already prepared to handle automatic downloads - it just needs the release assets to be available.
