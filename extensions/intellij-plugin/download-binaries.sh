#!/bin/bash
# Download pre-built binaries from GitHub releases for IntelliJ plugin
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$SCRIPT_DIR/src/main/resources/bin"

# Get version from Cargo.toml in project root
VERSION=$(grep '^version = ' "$SCRIPT_DIR/../../Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
RELEASE_TAG="v$VERSION"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📦 Downloading pytest-language-server binaries"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Version: $VERSION"
echo "Release: $RELEASE_TAG"
echo ""

# Check if running in CI (GitHub Actions, GitLab CI, etc.)
if [ -n "$CI" ] || [ -n "$GITHUB_ACTIONS" ]; then
    echo "✓ Running in CI environment - skipping download"
    echo "  (Binaries should be provided by CI workflow)"
    echo ""
    if [ -d "$BIN_DIR" ] && [ -n "$(ls -A "$BIN_DIR" 2>/dev/null)" ]; then
        echo "Found binaries:"
        ls -lh "$BIN_DIR"
        echo ""
    fi
    exit 0
fi

# Check if binaries already exist (from previous download or manual placement)
if [ -d "$BIN_DIR" ] && [ -n "$(ls -A "$BIN_DIR" 2>/dev/null)" ]; then
    echo "✓ Binaries already present in $BIN_DIR"
    echo ""
    echo "Found binaries:"
    ls -lh "$BIN_DIR"
    echo ""
    echo "Skipping download. To force re-download, run: rm -rf $BIN_DIR"
    exit 0
fi

# Create bin directory
mkdir -p "$BIN_DIR"

# GitHub repository
REPO="bellini666/pytest-language-server"
BASE_URL="https://github.com/$REPO/releases/download/$RELEASE_TAG"

# Binary names to download
BINARIES=(
    "pytest-language-server-x86_64-unknown-linux-gnu"
    "pytest-language-server-aarch64-unknown-linux-gnu"
    "pytest-language-server-x86_64-apple-darwin"
    "pytest-language-server-aarch64-apple-darwin"
    "pytest-language-server.exe"
)

echo "Downloading binaries from GitHub release $RELEASE_TAG..."
echo ""

# Download each binary
for binary in "${BINARIES[@]}"; do
    URL="$BASE_URL/$binary"
    OUTPUT="$BIN_DIR/$binary"

    echo "⬇️  Downloading $binary..."
    if curl -L -f -o "$OUTPUT" "$URL" 2>/dev/null; then
        chmod +x "$OUTPUT" 2>/dev/null || true
        echo "   ✓ Downloaded successfully"
    else
        echo "   ✗ Failed to download $binary"
        echo "   URL: $URL"
        echo ""
        echo "❌ Error: Failed to download binary from GitHub releases"
        echo ""
        echo "Possible causes:"
        echo "  1. Release $RELEASE_TAG does not exist yet"
        echo "  2. Binary $binary is not available in the release"
        echo "  3. Network connectivity issues"
        echo ""
        echo "To resolve:"
        echo "  • Verify release exists: https://github.com/$REPO/releases/tag/$RELEASE_TAG"
        echo "  • Build binaries locally: cargo build --release --target <target>"
        echo "  • Or wait for CI to publish the release"
        exit 1
    fi
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ All binaries downloaded successfully!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Downloaded binaries:"
ls -lh "$BIN_DIR"
echo ""
