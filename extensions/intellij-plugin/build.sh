#!/bin/bash
# Build script for IntelliJ plugin using Gradle
set -e

echo "Building pytest-language-server IntelliJ plugin..."

# Build the plugin using Gradle
./gradlew buildPlugin

# The plugin ZIP will be in build/distributions/
echo "✓ Plugin built successfully!"
ls -lh build/distributions/*.zip
