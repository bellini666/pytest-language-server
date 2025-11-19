# pytest Language Server for IntelliJ/PyCharm

A blazingly fast Language Server Protocol implementation for pytest fixtures, written in Rust.

## Features

- **Go to Definition**: Jump to fixture definitions from usage
- **Code Completion**: Smart auto-completion for pytest fixtures with context-aware suggestions
- **Find References**: Find all usages of a fixture
- **Hover Documentation**: View fixture signatures and docstrings
- **Diagnostics**: Warnings for undeclared fixtures used in function bodies
- **Code Actions**: Quick fixes to add missing fixture parameters
- **Fixture Priority**: Correctly handles pytest's fixture shadowing rules

## Architecture

This plugin uses [LSP4IJ](https://github.com/redhat-developer/lsp4ij), the standard Language Server Protocol client for IntelliJ Platform. LSP4IJ provides:

- Automatic LSP protocol handling
- Built-in LSP console for debugging
- Server configuration UI
- Trace/debug level controls
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

### LSP4IJ Features

The plugin automatically provides:

- **LSP Console**: View → Tool Windows → LSP Console
  - Monitor LSP requests/responses
  - Debug server communication
  - Configure trace levels
- **Language Server Settings**: Settings → Languages & Frameworks → Language Servers
  - View server status
  - Configure debug options
  - Manage server lifecycle

## Requirements

### For End Users

None! The plugin includes pre-built binaries for:
- macOS (Intel and Apple Silicon)
- Linux (x86_64 and ARM64)
- Windows (x86_64)

The plugin works out of the box with no additional setup required.

### For Developers

- Java 17 or later
- Gradle 8.10+ (wrapper included)
- pytest-language-server binary (for local testing)

## Usage

The language server automatically activates for Python test files:
- `test_*.py`
- `*_test.py`
- `conftest.py`

No additional configuration is needed. The language server starts on-demand when you open a matching file.

## Development

### Prerequisites

```bash
# Install Java 17+ (macOS with Homebrew)
brew install openjdk@17
export PATH="/opt/homebrew/opt/openjdk@17/bin:$PATH"

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
4. View LSP communication: **View → Tool Windows → LSP Console**
5. Check server logs in the IDE log: **Help → Show Log in Finder/Explorer**

### Code Structure

```
src/main/java/com/github/bellini666/pytestlsp/
├── PytestLanguageServerFactory.kt           # LSP4IJ factory
├── PytestLanguageServerConnectionProvider.kt # Server process management
├── PytestLanguageServerService.kt           # Binary location resolution
└── PytestLanguageServerListener.kt          # Lifecycle logging

src/main/resources/
├── META-INF/
│   ├── plugin.xml              # Plugin descriptor with LSP4IJ extensions
│   └── python-support.xml      # Python-specific configuration
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

**LSP4IJ Integration:**
- `plugin.xml` declares language server via `com.redhat.devtools.lsp4ij.server` extension
- Language mapping connects Python files to pytest language server
- `PytestLanguageServerFactory` creates connection provider and client
- `ProcessStreamConnectionProvider` handles stdio communication

**Forward Compatibility:**
- `sinceBuild="242"` (PyCharm 2024.2+)
- `untilBuild=""` (all future versions)
- LSP4IJ provides stable API across IntelliJ versions

## Troubleshooting

### Language Server Not Starting

1. **Check binary exists:**
   ```bash
   # For system PATH mode
   which pytest-language-server

   # For bundled binary (release builds)
   # Check plugin installation directory
   ```

2. **Check LSP Console:**
   - View → Tool Windows → LSP Console
   - Look for error messages or server startup issues

3. **Check IDE logs:**
   - Help → Show Log in Finder/Explorer
   - Search for "pytest-language-server" or "LSP4IJ"

4. **Verify VM options:**
   - Help → Edit Custom VM Options
   - Ensure `-Dpytest.lsp.useSystemPath=true` or custom path is set

### Build Issues

- **Gradle version:** Ensure Gradle 8.10+ (check with `./gradlew --version`)
- **Java version:** Ensure Java 17+ (check with `java -version`)
- **LSP4IJ dependency:** Uses version 0.18.0 for PyCharm 2024.2+ compatibility

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
