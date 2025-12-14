//! Configuration file support for pytest-language-server.
//!
//! Reads settings from `[tool.pytest-language-server]` section in `pyproject.toml`.

use glob::Pattern;
use serde::Deserialize;
use std::path::Path;
use tracing::{debug, warn};

/// Configuration for pytest-language-server.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Glob patterns for directories/files to exclude from scanning.
    pub exclude: Vec<Pattern>,

    /// Diagnostic codes to disable (e.g., "undeclared-fixture", "scope-mismatch").
    pub disabled_diagnostics: Vec<String>,

    /// Additional directories to scan for fixtures (beyond conftest.py hierarchy).
    #[allow(dead_code)] // Planned feature
    pub fixture_paths: Vec<String>,

    /// Third-party plugins to skip when scanning virtual environment.
    #[allow(dead_code)] // Used in tests, venv scanning integration planned
    pub skip_plugins: Vec<String>,
}

/// Raw configuration as parsed from TOML (before validation).
#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    exclude: Vec<String>,

    #[serde(default)]
    disabled_diagnostics: Vec<String>,

    #[serde(default)]
    fixture_paths: Vec<String>,

    #[serde(default)]
    skip_plugins: Vec<String>,
}

/// Wrapper for the pyproject.toml structure.
#[derive(Debug, Deserialize)]
struct PyProjectToml {
    tool: Option<Tool>,
}

#[derive(Debug, Deserialize)]
struct Tool {
    #[serde(rename = "pytest-language-server")]
    pytest_language_server: Option<RawConfig>,
}

impl Config {
    /// Load configuration from pyproject.toml in the given workspace root.
    /// Returns default configuration if file doesn't exist or has errors.
    pub fn load(workspace_root: &Path) -> Self {
        let pyproject_path = workspace_root.join("pyproject.toml");

        if !pyproject_path.exists() {
            debug!(
                "No pyproject.toml found at {:?}, using defaults",
                pyproject_path
            );
            return Self::default();
        }

        match std::fs::read_to_string(&pyproject_path) {
            Ok(content) => Self::parse(&content, &pyproject_path),
            Err(e) => {
                warn!("Failed to read pyproject.toml: {}", e);
                Self::default()
            }
        }
    }

    /// Parse configuration from TOML content.
    fn parse(content: &str, path: &Path) -> Self {
        let parsed: PyProjectToml = match toml::from_str(content) {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to parse pyproject.toml at {:?}: {}", path, e);
                return Self::default();
            }
        };

        let raw = parsed
            .tool
            .and_then(|t| t.pytest_language_server)
            .unwrap_or_default();

        Self::from_raw(raw, path)
    }

    /// Convert raw config to validated config.
    fn from_raw(raw: RawConfig, path: &Path) -> Self {
        // Parse exclude patterns, warning on invalid ones
        let exclude: Vec<Pattern> = raw
            .exclude
            .into_iter()
            .filter_map(|pattern| match Pattern::new(&pattern) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!("Invalid exclude pattern '{}' in {:?}: {}", pattern, path, e);
                    None
                }
            })
            .collect();

        // Validate diagnostic codes
        let valid_diagnostics = [
            "undeclared-fixture",
            "scope-mismatch",
            "circular-dependency",
        ];
        let disabled_diagnostics: Vec<String> = raw
            .disabled_diagnostics
            .into_iter()
            .filter(|code| {
                if valid_diagnostics.contains(&code.as_str()) {
                    true
                } else {
                    warn!(
                        "Unknown diagnostic code '{}' in {:?}, valid codes are: {:?}",
                        code, path, valid_diagnostics
                    );
                    false
                }
            })
            .collect();

        debug!(
            "Loaded config from {:?}: {} exclude patterns, {} disabled diagnostics",
            path,
            exclude.len(),
            disabled_diagnostics.len()
        );

        Self {
            exclude,
            disabled_diagnostics,
            fixture_paths: raw.fixture_paths,
            skip_plugins: raw.skip_plugins,
        }
    }

    /// Check if a diagnostic code is disabled.
    pub fn is_diagnostic_disabled(&self, code: &str) -> bool {
        self.disabled_diagnostics.iter().any(|d| d == code)
    }

    /// Check if a path should be excluded from scanning.
    #[allow(dead_code)] // Used in tests and will be used for file-level exclusion
    pub fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        self.exclude
            .iter()
            .any(|pattern| pattern.matches(&path_str))
    }

    /// Check if a plugin should be skipped when scanning venv.
    #[allow(dead_code)] // Used in tests, venv scanning integration planned
    pub fn should_skip_plugin(&self, plugin_name: &str) -> bool {
        self.skip_plugins.iter().any(|p| p == plugin_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config() {
        let content = r#"
[project]
name = "myproject"
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert!(config.exclude.is_empty());
        assert!(config.disabled_diagnostics.is_empty());
        assert!(config.fixture_paths.is_empty());
        assert!(config.skip_plugins.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let content = r#"
[project]
name = "myproject"

[tool.pytest-language-server]
exclude = ["build", "dist/**", ".tox"]
disabled_diagnostics = ["undeclared-fixture"]
fixture_paths = ["fixtures/", "shared/fixtures/"]
skip_plugins = ["pytest-xdist"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert_eq!(config.exclude.len(), 3);
        assert_eq!(config.disabled_diagnostics, vec!["undeclared-fixture"]);
        assert_eq!(config.fixture_paths, vec!["fixtures/", "shared/fixtures/"]);
        assert_eq!(config.skip_plugins, vec!["pytest-xdist"]);
    }

    #[test]
    fn test_parse_partial_config() {
        let content = r#"
[tool.pytest-language-server]
exclude = ["build"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert_eq!(config.exclude.len(), 1);
        assert!(config.disabled_diagnostics.is_empty());
    }

    #[test]
    fn test_invalid_glob_pattern_skipped() {
        let content = r#"
[tool.pytest-language-server]
exclude = ["valid", "[invalid", "also_valid"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        // Invalid pattern "[invalid" should be skipped
        assert_eq!(config.exclude.len(), 2);
    }

    #[test]
    fn test_invalid_diagnostic_code_skipped() {
        let content = r#"
[tool.pytest-language-server]
disabled_diagnostics = ["undeclared-fixture", "invalid-code", "scope-mismatch"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        // "invalid-code" should be filtered out
        assert_eq!(config.disabled_diagnostics.len(), 2);
        assert!(config
            .disabled_diagnostics
            .contains(&"undeclared-fixture".to_string()));
        assert!(config
            .disabled_diagnostics
            .contains(&"scope-mismatch".to_string()));
    }

    #[test]
    fn test_is_diagnostic_disabled() {
        let content = r#"
[tool.pytest-language-server]
disabled_diagnostics = ["undeclared-fixture"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert!(config.is_diagnostic_disabled("undeclared-fixture"));
        assert!(!config.is_diagnostic_disabled("scope-mismatch"));
    }

    #[test]
    fn test_should_exclude() {
        let content = r#"
[tool.pytest-language-server]
exclude = ["build/**", "dist"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert!(config.should_exclude(Path::new("build/output/file.py")));
        assert!(config.should_exclude(Path::new("dist")));
        assert!(!config.should_exclude(Path::new("src/main.py")));
    }

    #[test]
    fn test_should_skip_plugin() {
        let content = r#"
[tool.pytest-language-server]
skip_plugins = ["pytest-xdist", "pytest-cov"]
"#;
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert!(config.should_skip_plugin("pytest-xdist"));
        assert!(config.should_skip_plugin("pytest-cov"));
        assert!(!config.should_skip_plugin("pytest-mock"));
    }

    #[test]
    fn test_invalid_toml_returns_default() {
        let content = "this is not valid toml [[[";
        let config = Config::parse(content, Path::new("pyproject.toml"));
        assert!(config.exclude.is_empty());
        assert!(config.disabled_diagnostics.is_empty());
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.exclude.is_empty());
        assert!(config.disabled_diagnostics.is_empty());
        assert!(config.fixture_paths.is_empty());
        assert!(config.skip_plugins.is_empty());
    }
}
