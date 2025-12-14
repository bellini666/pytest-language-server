//! Workspace and virtual environment scanning for fixture definitions.

use super::FixtureDatabase;
use glob::Pattern;
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

impl FixtureDatabase {
    /// Directories that should be skipped during workspace scanning.
    /// These are typically large directories that don't contain test files.
    const SKIP_DIRECTORIES: &'static [&'static str] = &[
        // Version control
        ".git",
        ".hg",
        ".svn",
        // Virtual environments (scanned separately for plugins)
        ".venv",
        "venv",
        "env",
        ".env",
        // Python caches and build artifacts
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        ".tox",
        ".nox",
        "build",
        "dist",
        ".eggs",
        // JavaScript/Node
        "node_modules",
        "bower_components",
        // Rust (for mixed projects)
        "target",
        // IDE and editor directories
        ".idea",
        ".vscode",
        // Other common large directories
        ".cache",
        ".local",
        "vendor",
        "site-packages",
    ];

    /// Check if a directory should be skipped during scanning.
    pub(crate) fn should_skip_directory(dir_name: &str) -> bool {
        // Check exact matches
        if Self::SKIP_DIRECTORIES.contains(&dir_name) {
            return true;
        }
        // Also skip directories ending with .egg-info
        if dir_name.ends_with(".egg-info") {
            return true;
        }
        false
    }

    /// Scan a workspace directory for test files and conftest.py files.
    /// Optionally accepts exclude patterns from configuration.
    pub fn scan_workspace(&self, root_path: &Path) {
        self.scan_workspace_with_excludes(root_path, &[]);
    }

    /// Scan a workspace directory with custom exclude patterns.
    pub fn scan_workspace_with_excludes(&self, root_path: &Path, exclude_patterns: &[Pattern]) {
        info!("Scanning workspace: {:?}", root_path);

        // Defensive check: ensure the root path exists
        if !root_path.exists() {
            warn!(
                "Workspace path does not exist, skipping scan: {:?}",
                root_path
            );
            return;
        }

        // Phase 1: Collect all file paths (sequential, fast)
        let mut files_to_process: Vec<std::path::PathBuf> = Vec::new();
        let mut skipped_dirs = 0;

        // Use WalkDir with filter to skip large/irrelevant directories
        let walker = WalkDir::new(root_path).into_iter().filter_entry(|entry| {
            // Allow files to pass through
            if entry.file_type().is_file() {
                return true;
            }
            // For directories, check if we should skip them
            if let Some(dir_name) = entry.file_name().to_str() {
                !Self::should_skip_directory(dir_name)
            } else {
                true
            }
        });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    // Log directory traversal errors (permission denied, etc.)
                    if err
                        .io_error()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
                    {
                        warn!(
                            "Permission denied accessing path during workspace scan: {}",
                            err
                        );
                    } else {
                        debug!("Error during workspace scan: {}", err);
                    }
                    continue;
                }
            };

            let path = entry.path();

            // Skip files in filtered directories (shouldn't happen with filter_entry, but just in case)
            if path.components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(Self::should_skip_directory)
            }) {
                skipped_dirs += 1;
                continue;
            }

            // Skip files matching user-configured exclude patterns
            // Patterns are matched against paths relative to workspace root
            if !exclude_patterns.is_empty() {
                if let Ok(relative_path) = path.strip_prefix(root_path) {
                    let relative_str = relative_path.to_string_lossy();
                    if exclude_patterns.iter().any(|p| p.matches(&relative_str)) {
                        debug!("Skipping excluded path: {:?}", path);
                        continue;
                    }
                }
            }

            // Look for conftest.py or test_*.py or *_test.py files
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename == "conftest.py"
                    || filename.starts_with("test_") && filename.ends_with(".py")
                    || filename.ends_with("_test.py")
                {
                    files_to_process.push(path.to_path_buf());
                }
            }
        }

        if skipped_dirs > 0 {
            debug!("Skipped {} entries in filtered directories", skipped_dirs);
        }

        let total_files = files_to_process.len();
        info!("Found {} test/conftest files to process", total_files);

        // Phase 2: Process files in parallel using rayon
        // Use analyze_file_fresh since this is initial scan (no previous definitions to clean)
        let error_count = AtomicUsize::new(0);

        files_to_process.par_iter().for_each(|path| {
            debug!("Found test/conftest file: {:?}", path);
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    self.analyze_file_fresh(path.clone(), &content);
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::PermissionDenied {
                        warn!("Permission denied reading file: {:?}", path);
                    } else {
                        error!("Failed to read file {:?}: {}", path, err);
                        error_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        let errors = error_count.load(Ordering::Relaxed);
        if errors > 0 {
            warn!("Workspace scan completed with {} errors", errors);
        }

        info!("Workspace scan complete. Processed {} files", total_files);

        // Also scan virtual environment for pytest plugins
        self.scan_venv_fixtures(root_path);

        info!("Total fixtures defined: {}", self.definitions.len());
        info!("Total files with fixture usages: {}", self.usages.len());
    }

    /// Scan virtual environment for pytest plugin fixtures.
    fn scan_venv_fixtures(&self, root_path: &Path) {
        info!("Scanning for pytest plugins in virtual environment");

        // Try to find virtual environment
        let venv_paths = vec![
            root_path.join(".venv"),
            root_path.join("venv"),
            root_path.join("env"),
        ];

        info!("Checking for venv in: {:?}", root_path);
        for venv_path in &venv_paths {
            debug!("Checking venv path: {:?}", venv_path);
            if venv_path.exists() {
                info!("Found virtual environment at: {:?}", venv_path);
                self.scan_venv_site_packages(venv_path);
                return;
            } else {
                debug!("  Does not exist: {:?}", venv_path);
            }
        }

        // Also check for system-wide VIRTUAL_ENV
        if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
            info!("Found VIRTUAL_ENV environment variable: {}", venv);
            let venv_path = std::path::PathBuf::from(venv);
            if venv_path.exists() {
                info!("Using VIRTUAL_ENV: {:?}", venv_path);
                self.scan_venv_site_packages(&venv_path);
                return;
            } else {
                warn!("VIRTUAL_ENV path does not exist: {:?}", venv_path);
            }
        } else {
            debug!("No VIRTUAL_ENV environment variable set");
        }

        warn!("No virtual environment found - third-party fixtures will not be available");
    }

    fn scan_venv_site_packages(&self, venv_path: &Path) {
        info!("Scanning venv site-packages in: {:?}", venv_path);

        // Find site-packages directory
        let lib_path = venv_path.join("lib");
        debug!("Checking lib path: {:?}", lib_path);

        if lib_path.exists() {
            // Look for python* directories
            if let Ok(entries) = std::fs::read_dir(&lib_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let dirname = path.file_name().unwrap_or_default().to_string_lossy();
                    debug!("Found in lib: {:?}", dirname);

                    if path.is_dir() && dirname.starts_with("python") {
                        let site_packages = path.join("site-packages");
                        debug!("Checking site-packages: {:?}", site_packages);

                        if site_packages.exists() {
                            info!("Found site-packages: {:?}", site_packages);
                            self.scan_pytest_plugins(&site_packages);
                            return;
                        }
                    }
                }
            }
        }

        // Try Windows path
        let windows_site_packages = venv_path.join("Lib/site-packages");
        debug!("Checking Windows path: {:?}", windows_site_packages);
        if windows_site_packages.exists() {
            info!("Found site-packages (Windows): {:?}", windows_site_packages);
            self.scan_pytest_plugins(&windows_site_packages);
            return;
        }

        warn!("Could not find site-packages in venv: {:?}", venv_path);
    }

    fn scan_pytest_plugins(&self, site_packages: &Path) {
        info!("Scanning pytest plugins in: {:?}", site_packages);

        // List of known pytest plugin prefixes/packages
        let pytest_packages = vec![
            // Existing plugins
            "pytest_mock",
            "pytest-mock",
            "pytest_asyncio",
            "pytest-asyncio",
            "pytest_django",
            "pytest-django",
            "pytest_cov",
            "pytest-cov",
            "pytest_xdist",
            "pytest-xdist",
            "pytest_fixtures",
            // Additional popular plugins
            "pytest_flask",
            "pytest-flask",
            "pytest_httpx",
            "pytest-httpx",
            "pytest_postgresql",
            "pytest-postgresql",
            "pytest_mongodb",
            "pytest-mongodb",
            "pytest_redis",
            "pytest-redis",
            "pytest_elasticsearch",
            "pytest-elasticsearch",
            "pytest_rabbitmq",
            "pytest-rabbitmq",
            "pytest_mysql",
            "pytest-mysql",
            "pytest_docker",
            "pytest-docker",
            "pytest_kubernetes",
            "pytest-kubernetes",
            "pytest_celery",
            "pytest-celery",
            "pytest_tornado",
            "pytest-tornado",
            "pytest_aiohttp",
            "pytest-aiohttp",
            "pytest_sanic",
            "pytest-sanic",
            "pytest_fastapi",
            "pytest-fastapi",
            "pytest_alembic",
            "pytest-alembic",
            "pytest_sqlalchemy",
            "pytest-sqlalchemy",
            "pytest_factoryboy",
            "pytest-factoryboy",
            "pytest_freezegun",
            "pytest-freezegun",
            "pytest_mimesis",
            "pytest-mimesis",
            "pytest_lazy_fixture",
            "pytest-lazy-fixture",
            "pytest_cases",
            "pytest-cases",
            "pytest_bdd",
            "pytest-bdd",
            "pytest_benchmark",
            "pytest-benchmark",
            "pytest_timeout",
            "pytest-timeout",
            "pytest_retry",
            "pytest-retry",
            "pytest_repeat",
            "pytest-repeat",
            "pytest_rerunfailures",
            "pytest-rerunfailures",
            "pytest_ordering",
            "pytest-ordering",
            "pytest_dependency",
            "pytest-dependency",
            "pytest_random_order",
            "pytest-random-order",
            "pytest_picked",
            "pytest-picked",
            "pytest_testmon",
            "pytest-testmon",
            "pytest_split",
            "pytest-split",
            "pytest_env",
            "pytest-env",
            "pytest_dotenv",
            "pytest-dotenv",
            "pytest_html",
            "pytest-html",
            "pytest_json_report",
            "pytest-json-report",
            "pytest_metadata",
            "pytest-metadata",
            "pytest_instafail",
            "pytest-instafail",
            "pytest_clarity",
            "pytest-clarity",
            "pytest_sugar",
            "pytest-sugar",
            "pytest_emoji",
            "pytest-emoji",
            "pytest_play",
            "pytest-play",
            "pytest_selenium",
            "pytest-selenium",
            "pytest_playwright",
            "pytest-playwright",
            "pytest_splinter",
            "pytest-splinter",
        ];

        let mut plugin_count = 0;

        for entry in std::fs::read_dir(site_packages).into_iter().flatten() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let filename = path.file_name().unwrap_or_default().to_string_lossy();

            // Check if this is a pytest-related package
            let is_pytest_package = pytest_packages.iter().any(|pkg| filename.contains(pkg))
                || filename.starts_with("pytest")
                || filename.contains("_pytest");

            if is_pytest_package && path.is_dir() {
                // Skip .dist-info directories - they don't contain code
                if filename.ends_with(".dist-info") || filename.ends_with(".egg-info") {
                    debug!("Skipping dist-info directory: {:?}", filename);
                    continue;
                }

                info!("Scanning pytest plugin: {:?}", path);
                plugin_count += 1;
                self.scan_plugin_directory(&path);
            } else {
                // Log packages we're skipping for debugging
                if filename.contains("mock") {
                    debug!("Found mock-related package (not scanning): {:?}", filename);
                }
            }
        }

        info!("Scanned {} pytest plugin packages", plugin_count);
    }

    fn scan_plugin_directory(&self, plugin_dir: &Path) {
        // Recursively scan for Python files with fixtures
        for entry in WalkDir::new(plugin_dir)
            .max_depth(3) // Limit depth to avoid scanning too much
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("py") {
                // Only scan files that might have fixtures (not test files)
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Skip test files and __pycache__
                    if filename.starts_with("test_") || filename.contains("__pycache__") {
                        continue;
                    }

                    debug!("Scanning plugin file: {:?}", path);
                    if let Ok(content) = std::fs::read_to_string(path) {
                        self.analyze_file(path.to_path_buf(), &content);
                    }
                }
            }
        }
    }
}
