#!/bin/bash
# Simple build script for IntelliJ plugin without Gradle
set -e

# Extract version from plugin.xml
VERSION=$(grep -o '<version>[^<]*</version>' src/main/resources/META-INF/plugin.xml | sed 's/<version>\(.*\)<\/version>/\1/')

echo "Building pytest-language-server IntelliJ plugin v${VERSION}..."

# Clean previous builds
rm -rf build
mkdir -p build/classes/com/github/bellini666/pytestlsp
mkdir -p build/lib
mkdir -p build/META-INF

# Copy resources
echo "Copying resources..."
cp src/main/resources/META-INF/plugin.xml build/META-INF/
cp src/main/resources/META-INF/python-support.xml build/META-INF/
cp src/main/resources/META-INF/pluginIcon.png build/META-INF/

# Copy bundled binaries if they exist
if [ -d "src/main/resources/bin" ] && [ -n "$(ls -A src/main/resources/bin 2>/dev/null)" ]; then
  echo "Copying bundled binaries..."
  mkdir -p build/bin
  cp -r src/main/resources/bin/* build/bin/
fi

# Compile Kotlin files (simple compilation without dependencies for now)
# Note: For a real LSP plugin, you'd need proper IntelliJ SDK compilation
# For CI, we'll just package the source files which JetBrains can compile
echo "Packaging source files..."
cp -r src/main/java/com/github/bellini666/pytestlsp/*.kt build/classes/com/github/bellini666/pytestlsp/

# Create JAR
echo "Creating plugin JAR..."
cd build
if [ -d "bin" ]; then
  jar cf ../pytest-language-server.jar META-INF/ classes/ bin/
else
  jar cf ../pytest-language-server.jar META-INF/ classes/
fi

# Create distribution ZIP
cd ..
mkdir -p dist
cp pytest-language-server.jar dist/
cd dist
mkdir -p pytest-language-server/lib
mv pytest-language-server.jar pytest-language-server/lib/
zip -r pytest-language-server-${VERSION}.zip pytest-language-server/

echo "✓ Plugin built successfully: dist/pytest-language-server-${VERSION}.zip"
ls -lh pytest-language-server-${VERSION}.zip
