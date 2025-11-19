#!/bin/bash
# Version bump script for pytest-language-server
# Usage: ./bump-version.sh <new-version>
# Example: ./bump-version.sh 0.3.1

set -e

if [ -z "$1" ]; then
    echo "Usage: $0 <new-version>"
    echo "Example: $0 0.3.1"
    exit 1
fi

NEW_VERSION="$1"

# Validate version format (basic semver check)
if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "Error: Version must be in format X.Y.Z (e.g., 0.3.1)"
    exit 1
fi

echo "Bumping version to $NEW_VERSION..."

# Update Cargo.toml
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml && rm Cargo.toml.bak

# Update pyproject.toml
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" pyproject.toml && rm pyproject.toml.bak

# Update extensions/zed-extension/Cargo.toml
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" extensions/zed-extension/Cargo.toml && rm extensions/zed-extension/Cargo.toml.bak

# Update extensions/zed-extension/extension.toml
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" extensions/zed-extension/extension.toml && rm extensions/zed-extension/extension.toml.bak

# Update VSCode extension
if [ -f "extensions/vscode-extension/package.json" ]; then
    sed -i.bak "s/\"version\": \".*\"/\"version\": \"$NEW_VERSION\"/" extensions/vscode-extension/package.json && rm extensions/vscode-extension/package.json.bak
    echo "  - extensions/vscode-extension/package.json"
fi

# Update IntelliJ plugin
if [ -f "extensions/intellij-plugin/src/main/resources/META-INF/plugin.xml" ]; then
    sed -i.bak "s/<version>.*<\/version>/<version>$NEW_VERSION<\/version>/" extensions/intellij-plugin/src/main/resources/META-INF/plugin.xml && rm extensions/intellij-plugin/src/main/resources/META-INF/plugin.xml.bak
    echo "  - extensions/intellij-plugin/src/main/resources/META-INF/plugin.xml"
fi

if [ -f "extensions/intellij-plugin/build.gradle.kts" ]; then
    sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" extensions/intellij-plugin/build.gradle.kts && rm extensions/intellij-plugin/build.gradle.kts.bak
    echo "  - extensions/intellij-plugin/build.gradle.kts"
fi

# Update Cargo.lock
cargo update -p pytest-language-server

echo "✓ Version bumped to $NEW_VERSION in:"
echo "  - Cargo.toml"
echo "  - pyproject.toml"
echo "  - extensions/zed-extension/Cargo.toml"
echo "  - extensions/zed-extension/extension.toml"
echo "  - Cargo.lock"
echo ""
echo "Run 'git add -A && git commit -m \"chore: bump version to $NEW_VERSION\"' to commit"
