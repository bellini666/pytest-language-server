# Publishing the Zed Extension

This guide explains how to publish the pytest-language-server extension to the Zed extension registry.

## Prerequisites

1. **Rust installed via rustup** (required for building the extension)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Fork the Zed extensions repository**
   - Visit https://github.com/zed-industries/extensions
   - Click "Fork" (preferably to a personal account, not an organization)
   - This allows Zed staff to push changes to your PR if needed

3. **Clone your fork**
   ```bash
   git clone https://github.com/YOUR-USERNAME/extensions
   cd extensions
   git submodule init
   git submodule update
   ```

## Publishing Steps

### 1. Add Extension as Submodule

From the `extensions` repository root:

```bash
git submodule add https://github.com/bellini666/pytest-language-server.git extensions/pytest-language-server
git add extensions/pytest-language-server
```

**Note:** The submodule path should point to the root of the pytest-language-server repository, not the `zed-extension` subdirectory.

### 2. Update extensions.toml

Add this entry to the `extensions.toml` file in the root of the extensions repository:

```toml
[pytest-language-server]
submodule = "extensions/pytest-language-server"
path = "zed-extension"
version = "0.3.0"
```

**Important:** The `path` field is required because the extension files are in a subdirectory (`zed-extension`) within the repository.

### 3. Sort Extensions

Run the sorting script to ensure proper ordering:

```bash
pnpm sort-extensions
```

### 4. Create Pull Request

```bash
git add extensions.toml .gitmodules
git commit -m "Add pytest Language Server extension"
git push origin main
```

Then:
1. Go to https://github.com/zed-industries/extensions
2. Click "New Pull Request"
3. Select your fork and branch
4. Create the PR with a descriptive title like "Add pytest Language Server extension"
5. In the PR description, explain what the extension does and link to the main repository

### 5. Wait for Review

The Zed team will review your PR. Once merged, the extension will be automatically packaged and published to the Zed extension registry.

## Updating the Extension

To release an update:

1. Update the version in `zed-extension/extension.toml`
2. Create and push a new tag in the pytest-language-server repository
3. Create a PR to the extensions repository:
   ```bash
   cd extensions/extensions/pytest-lsp
   git pull origin main
   git checkout v0.4.0  # or whatever the new version is
   cd ../../..
   ```
4. Update the version in `extensions.toml`
5. Run `pnpm sort-extensions`
6. Commit and create a PR

## Testing Locally

Before publishing, test the extension locally:

1. Open Zed
2. Run command: "zed: install dev extension"
3. Select the `zed-extension` directory from this repository
4. Test all features in a Python project with pytest

## License Requirement

The extension code must have a valid open-source license. This repository uses the MIT license, which is one of the accepted licenses for Zed extensions.

## References

- [Zed Extension Documentation](https://zed.dev/docs/extensions/developing-extensions)
- [Zed Extensions Repository](https://github.com/zed-industries/extensions)
- [Example Language Server Extension](https://github.com/sectore/zed-just-ls)
