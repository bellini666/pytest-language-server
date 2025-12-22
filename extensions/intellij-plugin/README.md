# pytest Language Server for IntelliJ/PyCharm

A blazingly fast Language Server Protocol implementation for pytest fixtures, written in Rust.

## Requirements

- **PyCharm Professional 2023.2** or later
- **IntelliJ IDEA Ultimate 2023.2** or later (with Python plugin)
- **Unified PyCharm 2025.1** or later (free tier)

> **Note:** This plugin uses IntelliJ's native LSP support which provides better performance than third-party LSP clients. Native LSP is available in commercial JetBrains IDEs since 2023.2, and in the unified PyCharm distribution since 2025.1.

## Features

- **Go to Definition**: Jump to fixture definitions from usage
- **Go to Implementation**: Navigate to yield statements in generator fixtures
- **Call Hierarchy**: Explore fixture dependencies (incoming/outgoing calls)
- **Code Completion**: Smart auto-completion for pytest fixtures with context-aware suggestions
- **Find References**: Find all usages of a fixture
- **Hover Documentation**: View fixture signatures and docstrings
- **Document Symbols**: Navigate fixtures within a file using the outline view
- **Workspace Symbols**: Search for fixtures across your entire workspace
- **Code Lens**: See fixture usage counts directly above definitions
- **Inlay Hints**: See fixture return types inline next to parameters
- **Diagnostics**: Warnings for undeclared fixtures, scope mismatches, and circular dependencies
- **Code Actions**: Quick fixes to add missing fixture parameters
- **Fixture Priority**: Correctly handles pytest's fixture shadowing rules

## Architecture

This plugin uses IntelliJ's **native LSP API** (available since 2023.2, significantly improved in 2025.2+). This provides:

- Best-in-class performance with native IDE integration
- No external plugin dependencies
- Built-in Language Services status bar widget
- Automatic server lifecycle management
- Seamless integration with IntelliJ's code intelligence features

## Configuration

The plugin uses the bundled pytest-language-server binary by default. No configuration is needed.

### Optional: Use Custom Binary

If you want to use your own installation instead of the bundled binary, you can configure it via JVM properties in your IDE's VM options (Help → Edit Custom VM Options):

**Option 1: Use system PATH**
```
-Dpytest.lsp.useSystemPath=true
```

**Option 2: Specify exact path**
```
-Dpytest.lsp.executable=/path/to/pytest-language-server
```

## Installation

### For End Users

Install from the [JetBrains Marketplace](https://plugins.jetbrains.com/plugin/24608-pytest-language-server):
1. Open Settings → Plugins → Marketplace
2. Search for "pytest Language Server"
3. Click Install

The plugin includes pre-built binaries for:
- macOS (Intel and Apple Silicon)
- Linux (x86_64 and ARM64)
- Windows (x86_64)

The plugin works out of the box with no additional setup required.

## Usage

The language server automatically activates for Python test files:
- `test_*.py`
- `*_test.py`
- `conftest.py`

No additional configuration is needed. The language server starts on-demand when you open a matching file.

## Development

### Prerequisites

- Java 21 or later
- Gradle 8.10+ (wrapper included)
- pytest-language-server binary (for local testing)

```bash
# Install Java 21+ (macOS with Homebrew)
brew install openjdk@21
export PATH="/opt/homebrew/opt/openjdk@21/bin:$PATH"

# Verify Java version
java -version
```

### Building

```bash
# Clean build
./gradlew clean buildPlugin

# The plugin ZIP will be in build/distributions/
ls -lh build/distributions/pytest-language-server-*.zip
```

### Testing Locally

**Option 1: Use bundled binary (release builds only)**

Bundled binaries are only included in CI/CD release builds. For local development, use Option 2.

**Option 2: Use installed binary (recommended for development)**

```bash
# 1. Install pytest-language-server from project root
cd ../..  # Navigate to project root
cargo install --path .

# 2. Configure IDE to use system PATH
# Add to Help → Edit Custom VM Options:
# -Dpytest.lsp.useSystemPath=true

# 3. Launch IDE with plugin
cd extensions/intellij-plugin
./gradlew runIde
```

**Option 3: Use custom binary path**

```bash
# Build the language server
cd ../..
cargo build --release

# Launch IDE with custom path
cd extensions/intellij-plugin
./gradlew runIde -Dpytest.lsp.executable=/path/to/pytest-language-server
```

### Debugging

1. Launch the IDE: `./gradlew runIde`
2. Open a Python test project
3. Open a test file (`test_*.py` or `conftest.py`)
4. Check the Language Services widget in the status bar
5. Check IDE logs: **Help → Show Log in Finder/Explorer**
6. Enable debug logging: **Help → Diagnostic Tools → Debug Log Settings** → add `#com.intellij.platform.lsp`

### Code Structure

```
src/main/java/com/github/bellini666/pytestlsp/
├── PytestLspServerSupportProvider.kt    # Native LSP server provider
├── PytestLanguageServerService.kt       # Binary location resolution
└── PytestLanguageServerListener.kt      # Lifecycle logging

src/main/resources/
├── META-INF/
│   ├── plugin.xml              # Plugin descriptor with native LSP extensions
│   └── python-support.xml      # Python-specific configuration (optional)
└── bin/                        # Platform-specific binaries (CI/CD only)
    ├── pytest-language-server-x86_64-apple-darwin
    ├── pytest-language-server-aarch64-apple-darwin
    ├── pytest-language-server-x86_64-unknown-linux-gnu
    ├── pytest-language-server-aarch64-unknown-linux-gnu
    └── pytest-language-server.exe
```

### Key Implementation Details

**Binary Resolution Priority:**
1. Custom path: `-Dpytest.lsp.executable=/path/to/binary`
2. System PATH: `-Dpytest.lsp.useSystemPath=true`
3. Bundled binary (default)

**Native LSP Integration:**
- `plugin.xml` declares language server via `com.intellij.platform.lsp.serverSupportProvider` extension
- `PytestLspServerSupportProvider` implements `LspServerSupportProvider`
- `PytestLspServerDescriptor` extends `ProjectWideLspServerDescriptor`
- File matching is done in `isSupportedFile()` method

**Forward Compatibility:**
- `sinceBuild="252"` (PyCharm 2025.2+)
- `untilBuild=""` (all future versions)
- Native LSP API provides stable interface across IntelliJ versions

## Troubleshooting

### Language Server Not Starting

1. **Check IDE version:**
   - Requires PyCharm 2025.2+ or IntelliJ IDEA Ultimate 2025.2+
   - PyCharm Community Edition before 2025.1 is not supported

2. **Check binary exists:**
   ```bash
   # For system PATH mode
   which pytest-language-server

   # For bundled binary (release builds)
   # Check plugin installation directory
   ```

3. **Check Language Services widget:**
   - Look at the status bar for LSP status indicators
   - Click to see connected language servers

4. **Check IDE logs:**
   - Help → Show Log in Finder/Explorer
   - Search for "pytest-language-server" or "LspServer"

5. **Enable debug logging:**
   - Help → Diagnostic Tools → Debug Log Settings
   - Add: `#com.intellij.platform.lsp`
   - Restart IDE and reproduce the issue

6. **Verify VM options:**
   - Help → Edit Custom VM Options
   - Ensure `-Dpytest.lsp.useSystemPath=true` or custom path is set

### Build Issues

- **Gradle version:** Ensure Gradle 8.10+ (check with `./gradlew --version`)
- **Java version:** Ensure Java 21+ (check with `java -version`)
- **IDE target:** Uses IntelliJ IDEA Ultimate 2025.2 for LSP module access

## Migration from Previous Versions

If you were using the previous version of this plugin (which used LSP4IJ), note:

1. **IDE requirements changed**: Now requires PyCharm Professional/IntelliJ Ultimate 2023.2+ (or unified PyCharm 2025.1+)
2. **No external dependencies**: LSP4IJ is no longer required
3. **Better performance**: Native LSP integration is faster and more responsive
4. **No configuration changes needed**: Your existing settings will continue to work

## CI/CD and Releases

The GitHub Actions workflow:
1. Builds the language server binaries for all platforms
2. Downloads binaries to `src/main/resources/bin/`
3. Builds the IntelliJ plugin with bundled binaries
4. Publishes to JetBrains Marketplace

For local development, binaries are NOT bundled - use system PATH instead.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Test locally with `./gradlew runIde`
5. Submit a pull request

## Issues

Report issues at: https://github.com/bellini666/pytest-language-server/issues

## License

MIT
