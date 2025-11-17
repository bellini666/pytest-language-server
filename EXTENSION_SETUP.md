# Quick Extension Setup Guide

This is a condensed guide for setting up extension publishing. See `EXTENSION_PUBLISHING.md` for detailed information.

## Prerequisites

### GitHub Secrets Required

Add these to your repo (Settings → Secrets → Actions):

- `VSCE_TOKEN` - Visual Studio Code Marketplace token
- `JETBRAINS_TOKEN` - JetBrains Marketplace token
- `CARGO_REGISTRY_TOKEN` - crates.io token (already set)

## VSCode Marketplace

1. Go to https://marketplace.visualstudio.com/manage
2. Create publisher account (use ID `bellini666` or your choice)
3. Generate PAT: https://dev.azure.com → User Settings → Personal Access Tokens
   - Scopes: Marketplace (Acquire, Manage)
4. Add token as `VSCE_TOKEN` secret in GitHub
5. Update publisher ID in `extensions/vscode-extension/package.json` if needed

## JetBrains Marketplace

1. Go to https://plugins.jetbrains.com/author/me
2. Upload initial plugin build manually (first time only)
3. Generate token: https://plugins.jetbrains.com/author/me/tokens
   - Permission: Upload plugin
4. Add token as `JETBRAINS_TOKEN` secret in GitHub

## Zed Extension

Zed extensions are currently distributed via GitHub releases (automatic).

For official Zed extension directory submission:
1. Fork https://github.com/zed-industries/extensions
2. Add extension to their repo
3. Submit PR

## Release Process

```bash
# 1. Bump version
./bump-version.sh 0.6.0

# 2. Commit and tag
git add -A
git commit -m "chore: bump version to 0.6.0"
git tag v0.6.0

# 3. Push
git push origin master
git push origin v0.6.0

# CI automatically:
# - Builds all binaries
# - Publishes to PyPI
# - Publishes to crates.io
# - Publishes to VSCode Marketplace
# - Publishes to JetBrains Marketplace
# - Creates GitHub release with Zed extension
```

## Binary Resolution Priority

All extensions use this priority:
1. User-configured path (if set)
2. System PATH (if `pytest-language-server` installed)
3. Bundled binary (platform-specific)

## Testing Locally

**VSCode:**
```bash
cd extensions/vscode-extension
npm install
npm run compile
# Press F5 in VSCode to launch Extension Development Host
```

**IntelliJ:**
```bash
cd extensions/intellij-plugin
./gradlew runIde
# Launches IntelliJ with plugin installed
```

**Zed:**
```bash
cd extensions/zed-extension
cargo build --target wasm32-wasip1
# Install manually in Zed
```

## Troubleshooting

**Version mismatch?** Run `./bump-version.sh X.Y.Z` - it updates all files.

**Token expired?** Regenerate and update GitHub secret.

**Binary not found?** Ensure binaries are in `bin/` subdirectories of each extension.

See `EXTENSION_PUBLISHING.md` for detailed troubleshooting.
