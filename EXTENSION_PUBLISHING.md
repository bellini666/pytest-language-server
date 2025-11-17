# Extension Publishing Guide

This document outlines how to set up and publish the pytest-language-server extensions to various marketplaces.

## Overview

The pytest-language-server has extensions for three IDEs:
- **VSCode**: Published to Visual Studio Marketplace
- **IntelliJ/PyCharm**: Published to JetBrains Marketplace
- **Zed**: Published as bundled extension (manual process)

All extensions bundle platform-specific binaries and fall back to system PATH if available.

## Prerequisites

You'll need accounts and API tokens for each marketplace:

1. **VSCode Marketplace**
   - Microsoft Azure DevOps account
   - Personal Access Token (PAT) with Marketplace permissions

2. **JetBrains Marketplace**
   - JetBrains account
   - Plugin Repository token

3. **GitHub**
   - GitHub account with repo access
   - Permissions to create releases

## Setup Instructions

### 1. VSCode Marketplace Setup

**Create Publisher Account:**
1. Go to https://marketplace.visualstudio.com/manage
2. Sign in with Microsoft account
3. Click "Create Publisher"
4. Publisher ID: Use `bellini666` (or your chosen ID)
5. Update `extensions/vscode-extension/package.json` with your publisher ID

**Generate PAT Token:**
1. Go to https://dev.azure.com
2. User Settings → Personal Access Tokens
3. Create new token with scopes:
   - **Marketplace**: Acquire, Manage
4. Copy token and save as GitHub secret: `VSCE_TOKEN`

**Test Locally:**
```bash
cd extensions/vscode-extension
npm install
npm install -g @vscode/vsce

# Login
vsce login bellini666

# Package
vsce package

# Publish (manual test)
vsce publish
```

### 2. JetBrains Marketplace Setup

**Create Plugin:**
1. Go to https://plugins.jetbrains.com/author/me
2. Click "Upload plugin"
3. Upload initial version of `extensions/intellij-plugin/build/distributions/*.zip`
4. Fill in plugin details:
   - Name: pytest Language Server
   - Category: Code editing
   - Tags: python, pytest, lsp

**Generate Token:**
1. Go to https://plugins.jetbrains.com/author/me/tokens
2. Create new token with "Upload plugin" permission
3. Copy token and save as GitHub secret: `JETBRAINS_TOKEN`

**Test Locally:**
```bash
cd extensions/intellij-plugin
./gradlew buildPlugin

# The plugin will be in build/distributions/
# Test by installing manually in IDE:
# Settings → Plugins → Install from disk
```

### 3. Zed Extension Setup

**Note**: Zed currently uses a manual extension publishing process or requires submitting to their repository.

**For Manual Distribution:**
The CI automatically packages the Zed extension as `pytest-language-server-zed-extension.tar.gz` and uploads it to GitHub releases.

**For Official Zed Extension Directory:**
1. Fork https://github.com/zed-industries/extensions
2. Add extension to `extensions/` directory
3. Submit PR to Zed extensions repo

The extension will still be bundled with binaries in your GitHub releases for users to install manually.

### 4. GitHub Secrets Configuration

Add these secrets to your GitHub repository (Settings → Secrets and variables → Actions):

```
VSCE_TOKEN=<your-vscode-marketplace-token>
JETBRAINS_TOKEN=<your-jetbrains-marketplace-token>
CARGO_REGISTRY_TOKEN=<your-crates-io-token>
```

**Optional IntelliJ Plugin Signing (for paid plugins):**
```
CERTIFICATE_CHAIN=<your-certificate-chain>
PRIVATE_KEY=<your-private-key>
PRIVATE_KEY_PASSWORD=<your-key-password>
```

## Release Process

### Automated Release (Recommended)

1. **Bump Version:**
   ```bash
   ./bump-version.sh 0.6.0
   git add -A
   git commit -m "chore: bump version to 0.6.0"
   ```

2. **Create and Push Tag:**
   ```bash
   git tag v0.6.0
   git push origin master
   git push origin v0.6.0
   ```

3. **CI Automatically:**
   - Builds wheels for all platforms (PyPI)
   - Builds standalone binaries for extensions
   - Packages VSCode extension with binaries
   - Publishes to VSCode Marketplace
   - Builds and publishes IntelliJ plugin
   - Packages Zed extension
   - Creates GitHub release with all artifacts
   - Publishes to PyPI
   - Publishes to crates.io

### Manual Release (Fallback)

If CI fails or you need to publish manually:

**VSCode:**
```bash
cd extensions/vscode-extension
# Ensure binaries are in bin/ directory
npm install
vsce package
vsce publish
```

**IntelliJ:**
```bash
cd extensions/intellij-plugin
# Ensure binaries are in src/main/resources/bin/
./gradlew buildPlugin
./gradlew publishPlugin
```

**Zed:**
```bash
cd extensions/zed-extension
# Ensure binaries are in bin/ directory
cargo build --release --target wasm32-wasip1
# Package manually and distribute
```

## Extension Configuration

### VSCode Configuration

Users can configure the extension via settings:
```json
{
  "pytestLanguageServer.executable": "",  // Empty = use bundled
  "pytestLanguageServer.trace.server": "off"
}
```

### IntelliJ Configuration

Users can configure via JVM properties:
```
-Dpytest.lsp.executable=/path/to/binary
```

### Zed Configuration

Zed extension automatically tries:
1. System PATH (`pytest-language-server`)
2. Bundled binary

## Binary Resolution Priority

All extensions follow this priority:

1. **User-configured path** (if provided)
2. **System PATH** (if `pytest-language-server` is installed via pip)
3. **Bundled binary** (platform-specific)

## Troubleshooting

### VSCode Publishing Fails

- Check `VSCE_TOKEN` is valid
- Verify publisher ID matches in package.json
- Ensure version in package.json doesn't already exist

### IntelliJ Publishing Fails

- Check `JETBRAINS_TOKEN` is valid
- Verify plugin.xml has correct version
- Check build/distributions/ has the .zip file

### Binary Not Found

- Verify binaries are in correct location:
  - VSCode: `extensions/vscode-extension/bin/`
  - IntelliJ: `extensions/intellij-plugin/src/main/resources/bin/`
  - Zed: `extensions/zed-extension/bin/`
- Check binary names match platform expectations
- Ensure execute permissions on Unix (chmod +x)

### Version Mismatch

If versions get out of sync:
```bash
# Use the version bump script
./bump-version.sh X.Y.Z

# It updates:
# - Cargo.toml (main project)
# - pyproject.toml
# - extensions/zed-extension/Cargo.toml
# - extensions/zed-extension/extension.toml

# Manually update:
# - extensions/vscode-extension/package.json
# - extensions/intellij-plugin/build.gradle.kts
# - extensions/intellij-plugin/src/main/resources/META-INF/plugin.xml
```

## Monitoring

**VSCode Marketplace:**
- https://marketplace.visualstudio.com/items?itemName=bellini666.pytest-language-server
- View downloads, ratings, and reviews

**JetBrains Marketplace:**
- https://plugins.jetbrains.com/plugin/<your-plugin-id>
- View installs and ratings

**GitHub Releases:**
- https://github.com/bellini666/pytest-language-server/releases
- View download counts for binaries

**PyPI:**
- https://pypi.org/project/pytest-language-server/
- View download statistics

## First-Time Setup Checklist

- [ ] Create VSCode publisher account
- [ ] Generate VSCode PAT token
- [ ] Add VSCE_TOKEN to GitHub secrets
- [ ] Create JetBrains plugin listing
- [ ] Generate JetBrains token
- [ ] Add JETBRAINS_TOKEN to GitHub secrets
- [ ] Test VSCode extension locally
- [ ] Test IntelliJ plugin locally
- [ ] Test Zed extension locally
- [ ] Verify all binaries build correctly
- [ ] Run first release and verify all publishes work
- [ ] Update README with installation instructions for all platforms

## Platform-Specific Notes

**macOS:**
- Binaries need both x86_64 and aarch64 (Intel and Apple Silicon)
- Code signing may be required for Gatekeeper (future consideration)

**Linux:**
- Support both x86_64 and aarch64
- Use gnu libc targets (not musl) for better compatibility

**Windows:**
- Only x86_64 needed currently
- .exe extension required

## Support

If users report issues with bundled binaries, they can always fall back to:
```bash
pip install pytest-language-server
```

Then configure the extension to use system binary.
