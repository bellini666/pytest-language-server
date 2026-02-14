//! Workspace and virtual environment scanning for fixture definitions.

use super::FixtureDatabase;
use glob::Pattern;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

/// A pytest11 entry point from a dist-info package.
#[derive(Debug, Clone)]
pub(crate) struct Pytest11EntryPoint {
    /// The plugin name (left side of =)
    pub(crate) name: String,
    /// The Python module path (right side of =)
    pub(crate) module_path: String,
}

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

        // Store workspace root for editable install third-party detection
        *self.workspace_root.lock().unwrap() = Some(
            root_path
                .canonicalize()
                .unwrap_or_else(|_| root_path.to_path_buf()),
        );

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
        let permission_denied_count = AtomicUsize::new(0);

        files_to_process.par_iter().for_each(|path| {
            debug!("Found test/conftest file: {:?}", path);
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    self.analyze_file_fresh(path.clone(), &content);
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::PermissionDenied {
                        debug!("Permission denied reading file: {:?}", path);
                        permission_denied_count.fetch_add(1, Ordering::Relaxed);
                    } else {
                        error!("Failed to read file {:?}: {}", path, err);
                        error_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        let errors = error_count.load(Ordering::Relaxed);
        let permission_errors = permission_denied_count.load(Ordering::Relaxed);

        if errors > 0 {
            warn!("Workspace scan completed with {} read errors", errors);
        }
        if permission_errors > 0 {
            warn!(
                "Workspace scan: skipped {} files due to permission denied",
                permission_errors
            );
        }

        info!(
            "Workspace scan complete. Processed {} files ({} permission denied, {} errors)",
            total_files, permission_errors, errors
        );

        // Phase 3: Scan virtual environment for pytest plugins first
        // (must happen before import scanning so venv plugin files are in file_cache)
        self.scan_venv_fixtures(root_path);

        // Phase 4: Scan modules imported by conftest.py and venv plugin files
        // This ensures fixtures defined in separate modules (imported via star import
        // or pytest_plugins variable) are discovered
        self.scan_imported_fixture_modules(root_path);

        info!("Total fixtures defined: {}", self.definitions.len());
        info!("Total files with fixture usages: {}", self.usages.len());
    }

    /// Scan Python modules that are imported by conftest.py files.
    /// This discovers fixtures defined in separate modules that are re-exported via star imports.
    /// Handles transitive imports (A imports B, B imports C) by iteratively scanning until no new modules are found.
    fn scan_imported_fixture_modules(&self, _root_path: &Path) {
        use std::collections::HashSet;

        info!("Scanning for imported fixture modules");

        // Track all files we've already processed to find imports from
        let mut processed_files: HashSet<std::path::PathBuf> = HashSet::new();

        // Start with conftest.py, test files, and venv plugin files
        // (pytest_plugins can appear in any of these)
        let site_packages_paths = self.site_packages_paths.lock().unwrap().clone();
        let editable_roots: Vec<PathBuf> = self
            .editable_install_roots
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.source_root.clone())
            .collect();
        let mut files_to_check: Vec<std::path::PathBuf> = self
            .file_cache
            .iter()
            .filter(|entry| {
                let key = entry.key();
                let is_conftest_or_test = key
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| {
                        n == "conftest.py"
                            || (n.starts_with("test_") && n.ends_with(".py"))
                            || n.ends_with("_test.py")
                    })
                    .unwrap_or(false);
                let is_venv_plugin = site_packages_paths.iter().any(|sp| key.starts_with(sp));
                let is_editable_plugin = editable_roots.iter().any(|er| key.starts_with(er));
                is_conftest_or_test || is_venv_plugin || is_editable_plugin
            })
            .map(|entry| entry.key().clone())
            .collect();

        if files_to_check.is_empty() {
            debug!("No conftest/test/plugin files found, skipping import scan");
            return;
        }

        info!(
            "Starting import scan with {} conftest/test/plugin files",
            files_to_check.len()
        );

        // Iteratively process files until no new modules are discovered
        let mut iteration = 0;
        while !files_to_check.is_empty() {
            iteration += 1;
            debug!(
                "Import scan iteration {}: checking {} files",
                iteration,
                files_to_check.len()
            );

            let mut new_modules: HashSet<std::path::PathBuf> = HashSet::new();

            for file_path in &files_to_check {
                if processed_files.contains(file_path) {
                    continue;
                }
                processed_files.insert(file_path.clone());

                // Get the file content
                let Some(content) = self.get_file_content(file_path) else {
                    continue;
                };

                // Parse the AST
                let Some(parsed) = self.get_parsed_ast(file_path, &content) else {
                    continue;
                };

                let line_index = self.get_line_index(file_path, &content);

                // Extract imports and pytest_plugins
                if let rustpython_parser::ast::Mod::Module(module) = parsed.as_ref() {
                    let imports =
                        self.extract_fixture_imports(&module.body, file_path, &line_index);

                    for import in imports {
                        if let Some(resolved_path) =
                            self.resolve_module_to_file(&import.module_path, file_path)
                        {
                            let canonical = self.get_canonical_path(resolved_path);
                            if !processed_files.contains(&canonical)
                                && !self.file_cache.contains_key(&canonical)
                            {
                                new_modules.insert(canonical);
                            }
                        }
                    }

                    // Also extract pytest_plugins variable declarations
                    let plugin_modules = self.extract_pytest_plugins(&module.body);
                    for module_path in plugin_modules {
                        if let Some(resolved_path) =
                            self.resolve_module_to_file(&module_path, file_path)
                        {
                            let canonical = self.get_canonical_path(resolved_path);
                            if !processed_files.contains(&canonical)
                                && !self.file_cache.contains_key(&canonical)
                            {
                                new_modules.insert(canonical);
                            }
                        }
                    }
                }
            }

            if new_modules.is_empty() {
                debug!("No new modules found in iteration {}", iteration);
                break;
            }

            info!(
                "Iteration {}: found {} new modules to analyze",
                iteration,
                new_modules.len()
            );

            // Analyze the new modules
            for module_path in &new_modules {
                if module_path.exists() {
                    debug!("Analyzing imported module: {:?}", module_path);
                    match std::fs::read_to_string(module_path) {
                        Ok(content) => {
                            self.analyze_file_fresh(module_path.clone(), &content);
                        }
                        Err(err) => {
                            debug!("Failed to read imported module {:?}: {}", module_path, err);
                        }
                    }
                }
            }

            // Next iteration will check the newly analyzed modules for their imports
            files_to_check = new_modules.into_iter().collect();
        }

        info!(
            "Imported fixture module scan complete after {} iterations",
            iteration
        );
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
                let venv_path = venv_path.canonicalize().unwrap_or(venv_path);
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
                            let site_packages =
                                site_packages.canonicalize().unwrap_or(site_packages);
                            info!("Found site-packages: {:?}", site_packages);
                            self.site_packages_paths
                                .lock()
                                .unwrap()
                                .push(site_packages.clone());
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
            let windows_site_packages = windows_site_packages
                .canonicalize()
                .unwrap_or(windows_site_packages);
            info!("Found site-packages (Windows): {:?}", windows_site_packages);
            self.site_packages_paths
                .lock()
                .unwrap()
                .push(windows_site_packages.clone());
            self.scan_pytest_plugins(&windows_site_packages);
            return;
        }

        warn!("Could not find site-packages in venv: {:?}", venv_path);
    }

    /// Parse `entry_points.txt` content and extract pytest11 entries.
    ///
    /// Returns all successfully parsed entries from the `[pytest11]` section.
    /// Returns an empty vec if there is no `[pytest11]` section or no valid
    /// `name = value` lines within that section. Malformed lines are ignored.
    fn parse_pytest11_entry_points(content: &str) -> Vec<Pytest11EntryPoint> {
        let mut results = Vec::new();
        let mut in_pytest11_section = false;

        for line in content.lines() {
            let line = line.trim();

            // Check for section headers
            if line.starts_with('[') && line.ends_with(']') {
                in_pytest11_section = line == "[pytest11]";
                continue;
            }

            // Parse entries within pytest11 section
            if in_pytest11_section && !line.is_empty() && !line.starts_with('#') {
                if let Some((name, module_path)) = line.split_once('=') {
                    results.push(Pytest11EntryPoint {
                        name: name.trim().to_string(),
                        module_path: module_path.trim().to_string(),
                    });
                }
            }
        }
        results
    }

    /// Resolve a Python module path to a file system path within site-packages.
    ///
    /// Examples:
    /// - "pytest_mock" → site_packages/pytest_mock/__init__.py or site_packages/pytest_mock.py
    /// - "pytest_asyncio.plugin" → site_packages/pytest_asyncio/plugin.py
    ///
    /// Returns the path to a `.py` file (may be `__init__.py` for packages).
    fn resolve_entry_point_module_to_path(
        site_packages: &Path,
        module_path: &str,
    ) -> Option<PathBuf> {
        // Strip any :attr suffix (e.g., "module:function" -> "module")
        let module_path = module_path.split(':').next().unwrap_or(module_path);

        // Split into components
        let parts: Vec<&str> = module_path.split('.').collect();

        if parts.is_empty() {
            return None;
        }

        // Reject path traversal and null bytes in module path components
        if parts
            .iter()
            .any(|p| p.contains("..") || p.contains('\0') || p.is_empty())
        {
            return None;
        }

        // Build the path from module components
        let mut path = site_packages.to_path_buf();
        for part in &parts {
            path.push(part);
        }

        // Ensure resolved path stays within the base directory
        let check_bounded = |candidate: &Path| -> Option<PathBuf> {
            let canonical = candidate.canonicalize().ok()?;
            let base_canonical = site_packages.canonicalize().ok()?;
            if canonical.starts_with(&base_canonical) {
                Some(canonical)
            } else {
                None
            }
        };

        // Check if it's a module file (add .py extension)
        let py_file = path.with_extension("py");
        if py_file.exists() {
            return check_bounded(&py_file);
        }

        // Check if it's a package directory (has __init__.py)
        if path.is_dir() {
            let init_file = path.join("__init__.py");
            if init_file.exists() {
                return check_bounded(&init_file);
            }
        }

        None
    }

    /// Scan a single Python file for fixture definitions.
    fn scan_single_plugin_file(&self, file_path: &Path) {
        if file_path.extension().and_then(|s| s.to_str()) != Some("py") {
            return;
        }

        debug!("Scanning plugin file: {:?}", file_path);

        if let Ok(content) = std::fs::read_to_string(file_path) {
            self.analyze_file(file_path.to_path_buf(), &content);
        }
    }

    /// Load pytest plugins from a single dist-info directory's entry points.
    ///
    /// Reads entry_points.txt, parses [pytest11] section, resolves modules,
    /// and scans discovered plugin files for fixtures.
    ///
    /// Returns the number of plugin modules scanned.
    fn load_plugin_from_entry_point(&self, dist_info_path: &Path, site_packages: &Path) -> usize {
        let entry_points_file = dist_info_path.join("entry_points.txt");

        let content = match std::fs::read_to_string(&entry_points_file) {
            Ok(c) => c,
            Err(_) => return 0, // No entry_points.txt or unreadable
        };

        let entries = Self::parse_pytest11_entry_points(&content);

        if entries.is_empty() {
            return 0; // No pytest11 plugins in this package
        }

        let mut scanned_count = 0;

        for entry in entries {
            debug!(
                "Found pytest11 entry: {} = {}",
                entry.name, entry.module_path
            );

            let resolved =
                Self::resolve_entry_point_module_to_path(site_packages, &entry.module_path)
                    .or_else(|| self.resolve_entry_point_in_editable_installs(&entry.module_path));

            if let Some(path) = resolved {
                let scanned = if path.file_name().and_then(|n| n.to_str()) == Some("__init__.py") {
                    let package_dir = path.parent().expect("__init__.py must have parent");
                    info!(
                        "Scanning pytest plugin package directory for {}: {:?}",
                        entry.name, package_dir
                    );
                    self.scan_plugin_directory(package_dir);
                    true
                } else if path.is_file() {
                    info!("Scanning pytest plugin: {} -> {:?}", entry.name, path);
                    self.scan_single_plugin_file(&path);
                    true
                } else {
                    debug!(
                        "Resolved module path for plugin {} is not a file: {:?}",
                        entry.name, path
                    );
                    false
                };

                if scanned {
                    scanned_count += 1;
                }
            } else {
                debug!(
                    "Could not resolve module path: {} for plugin {}",
                    entry.module_path, entry.name
                );
            }
        }

        scanned_count
    }

    /// Scan pytest's internal _pytest package for built-in fixtures.
    /// This handles fixtures like tmp_path, capsys, monkeypatch, etc.
    fn scan_pytest_internal_fixtures(&self, site_packages: &Path) {
        let pytest_internal = site_packages.join("_pytest");

        if !pytest_internal.exists() || !pytest_internal.is_dir() {
            debug!("_pytest directory not found in site-packages");
            return;
        }

        info!(
            "Scanning pytest internal fixtures in: {:?}",
            pytest_internal
        );
        self.scan_plugin_directory(&pytest_internal);
    }

    /// Extract the raw and normalized package name from a `.dist-info` directory name.
    /// Returns `(raw_name, normalized_name)`.
    /// e.g., `my-package-1.0.0.dist-info` → `("my-package", "my_package")`
    fn extract_package_name_from_dist_info(dir_name: &str) -> Option<(String, String)> {
        // Strip the .dist-info or .egg-info suffix
        let name_version = dir_name
            .strip_suffix(".dist-info")
            .or_else(|| dir_name.strip_suffix(".egg-info"))?;

        // The format is `name-version`. Split on '-' and take the first segment.
        // Package names can contain hyphens, but the version always starts with a digit,
        // so find the first '-' followed by a digit.
        let name = if let Some(idx) = name_version.char_indices().position(|(i, c)| {
            c == '-' && name_version[i + 1..].starts_with(|c: char| c.is_ascii_digit())
        }) {
            &name_version[..idx]
        } else {
            name_version
        };

        let raw = name.to_string();
        // Normalize: PEP 503 says dashes, dots, underscores are interchangeable
        let normalized = name.replace(['-', '.'], "_").to_lowercase();
        Some((raw, normalized))
    }

    /// Discover editable installs by scanning `.dist-info` directories for `direct_url.json`.
    fn discover_editable_installs(&self, site_packages: &Path) {
        info!("Scanning for editable installs in: {:?}", site_packages);

        // Validate the site-packages path is a real directory before reading from it
        if !site_packages.is_dir() {
            warn!(
                "site-packages path is not a directory, skipping editable install scan: {:?}",
                site_packages
            );
            return;
        }

        // Clear previous editable installs to avoid duplicates on re-scan
        self.editable_install_roots.lock().unwrap().clear();

        // Index all .pth files once (stem → full path) to avoid re-reading site-packages per package
        let pth_index = Self::build_pth_index(site_packages);

        let entries = match std::fs::read_dir(site_packages) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let filename = path.file_name().unwrap_or_default().to_string_lossy();

            if !filename.ends_with(".dist-info") {
                continue;
            }

            let direct_url_path = path.join("direct_url.json");
            let content = match std::fs::read_to_string(&direct_url_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Parse direct_url.json to check for editable installs
            let json: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Check if dir_info.editable is true
            let is_editable = json
                .get("dir_info")
                .and_then(|d| d.get("editable"))
                .and_then(|e| e.as_bool())
                .unwrap_or(false);

            if !is_editable {
                continue;
            }

            let Some((raw_name, normalized_name)) =
                Self::extract_package_name_from_dist_info(&filename)
            else {
                continue;
            };

            // Find the .pth file that points to the source root
            let source_root = Self::find_editable_pth_source_root(
                &pth_index,
                &raw_name,
                &normalized_name,
                site_packages,
            );
            let Some(source_root) = source_root else {
                debug!(
                    "No .pth file found for editable install: {}",
                    normalized_name
                );
                continue;
            };

            info!(
                "Discovered editable install: {} -> {:?}",
                normalized_name, source_root
            );
            self.editable_install_roots
                .lock()
                .unwrap()
                .push(super::EditableInstall {
                    package_name: normalized_name,
                    raw_package_name: raw_name,
                    source_root,
                    site_packages: site_packages.to_path_buf(),
                });
        }

        let count = self.editable_install_roots.lock().unwrap().len();
        info!("Discovered {} editable install(s)", count);
    }

    /// Build an index of `.pth` file stems to their full paths.
    /// Read site-packages once and store `stem → path` for O(1) lookup.
    fn build_pth_index(site_packages: &Path) -> std::collections::HashMap<String, PathBuf> {
        let mut index = std::collections::HashMap::new();
        if !site_packages.is_dir() {
            return index;
        }
        let entries = match std::fs::read_dir(site_packages) {
            Ok(e) => e,
            Err(_) => return index,
        };
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy();
            if fname_str.ends_with(".pth") {
                let stem = fname_str.strip_suffix(".pth").unwrap_or(&fname_str);
                index.insert(stem.to_string(), entry.path());
            }
        }
        index
    }

    /// Find the source root from a `.pth` file for an editable install.
    /// Uses both raw and normalized package names to handle pip's varying naming conventions.
    /// Looks for both old-style (`_<pkg>.pth`) and new-style (`__editable__.<pkg>.pth`) naming.
    fn find_editable_pth_source_root(
        pth_index: &std::collections::HashMap<String, PathBuf>,
        raw_name: &str,
        normalized_name: &str,
        site_packages: &Path,
    ) -> Option<PathBuf> {
        // Build candidates from both raw and normalized names.
        // Raw name preserves original dashes/dots (e.g., "my-package"),
        // normalized uses underscores (e.g., "my_package").
        let mut candidates: Vec<String> = vec![
            format!("__editable__.{}", normalized_name),
            format!("_{}", normalized_name),
            normalized_name.to_string(),
        ];
        if raw_name != normalized_name {
            candidates.push(format!("__editable__.{}", raw_name));
            candidates.push(format!("_{}", raw_name));
            candidates.push(raw_name.to_string());
        }

        // Search the pre-built index for matching .pth stems
        for (stem, pth_path) in pth_index {
            let matches = candidates.iter().any(|c| {
                stem == c
                    || stem.strip_prefix(c).is_some_and(|rest| {
                        rest.starts_with('-')
                            && rest[1..].starts_with(|ch: char| ch.is_ascii_digit())
                    })
            });
            if !matches {
                continue;
            }

            // Parse the .pth file: first non-comment, non-import line is the path
            let content = match std::fs::read_to_string(pth_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with("import ") {
                    continue;
                }
                // Validate: reject lines with null bytes, control characters,
                // or path traversal sequences
                if line.contains('\0')
                    || line.bytes().any(|b| b < 0x20 && b != b'\t')
                    || line.contains("..")
                {
                    debug!("Skipping .pth line with invalid characters: {:?}", line);
                    continue;
                }
                let candidate = PathBuf::from(line);
                let resolved = if candidate.is_absolute() {
                    candidate
                } else {
                    site_packages.join(&candidate)
                };
                // Canonicalize to resolve symlinks and validate existence,
                // then verify it's an actual directory
                match resolved.canonicalize() {
                    Ok(canonical) if canonical.is_dir() => return Some(canonical),
                    Ok(canonical) => {
                        debug!(".pth path is not a directory: {:?}", canonical);
                        continue;
                    }
                    Err(_) => {
                        debug!("Could not canonicalize .pth path: {:?}", resolved);
                        continue;
                    }
                }
            }
        }

        None
    }

    /// Try to resolve an entry point module path through editable install source roots.
    fn resolve_entry_point_in_editable_installs(&self, module_path: &str) -> Option<PathBuf> {
        let installs = self.editable_install_roots.lock().unwrap();
        for install in installs.iter() {
            if let Some(path) =
                Self::resolve_entry_point_module_to_path(&install.source_root, module_path)
            {
                return Some(path);
            }
        }
        None
    }

    fn scan_pytest_plugins(&self, site_packages: &Path) {
        info!(
            "Scanning for pytest plugins via entry points in: {:?}",
            site_packages
        );

        // Discover editable installs before scanning entry points
        self.discover_editable_installs(site_packages);

        let mut plugin_count = 0;

        // First, scan pytest's internal fixtures (special case)
        self.scan_pytest_internal_fixtures(site_packages);

        // Iterate over ALL dist-info directories and check for pytest11 entry points
        for entry in std::fs::read_dir(site_packages).into_iter().flatten() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let filename = path.file_name().unwrap_or_default().to_string_lossy();

            // Only process dist metadata directories
            if !filename.ends_with(".dist-info") && !filename.ends_with(".egg-info") {
                continue;
            }

            // Try to load plugins from this package's entry points
            let scanned = self.load_plugin_from_entry_point(&path, site_packages);
            if scanned > 0 {
                plugin_count += scanned;
                debug!("Loaded {} plugin module(s) from {}", scanned, filename);
            }
        }

        info!(
            "Discovered fixtures from {} pytest plugin modules",
            plugin_count
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parse_pytest11_entry_points_basic() {
        let content = r#"
[console_scripts]
my-cli = my_package:main

[pytest11]
my_plugin = my_package.plugin
another = another_pkg

[other_section]
foo = bar
"#;

        let entries = FixtureDatabase::parse_pytest11_entry_points(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "my_plugin");
        assert_eq!(entries[0].module_path, "my_package.plugin");
        assert_eq!(entries[1].name, "another");
        assert_eq!(entries[1].module_path, "another_pkg");
    }

    #[test]
    fn test_parse_pytest11_entry_points_empty_file() {
        let entries = FixtureDatabase::parse_pytest11_entry_points("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_pytest11_entry_points_no_pytest11_section() {
        let content = r#"
[console_scripts]
my-cli = my_package:main
"#;
        let entries = FixtureDatabase::parse_pytest11_entry_points(content);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_pytest11_entry_points_with_comments() {
        let content = r#"
[pytest11]
# This is a comment
my_plugin = my_package.plugin
# Another comment
"#;
        let entries = FixtureDatabase::parse_pytest11_entry_points(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "my_plugin");
    }

    #[test]
    fn test_parse_pytest11_entry_points_with_whitespace() {
        let content = r#"
[pytest11]
   my_plugin   =   my_package.plugin
another=another_pkg
"#;
        let entries = FixtureDatabase::parse_pytest11_entry_points(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "my_plugin");
        assert_eq!(entries[0].module_path, "my_package.plugin");
        assert_eq!(entries[1].name, "another");
        assert_eq!(entries[1].module_path, "another_pkg");
    }

    #[test]
    fn test_parse_pytest11_entry_points_with_attr() {
        // Some entry points have :attr suffix (e.g., module:function)
        let content = r#"
[pytest11]
my_plugin = my_package.module:plugin_entry
"#;
        let entries = FixtureDatabase::parse_pytest11_entry_points(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].module_path, "my_package.module:plugin_entry");
    }

    #[test]
    fn test_parse_pytest11_entry_points_multiple_sections_before_pytest11() {
        let content = r#"
[console_scripts]
cli = pkg:main

[gui_scripts]
gui = pkg:gui_main

[pytest11]
my_plugin = my_package.plugin

[other]
extra = something
"#;
        let entries = FixtureDatabase::parse_pytest11_entry_points(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "my_plugin");
    }

    #[test]
    fn test_resolve_entry_point_module_to_path_package() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package with __init__.py
        let pkg_dir = site_packages.join("my_plugin");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "# plugin code").unwrap();

        // Should resolve to __init__.py (canonicalized)
        let result =
            FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "my_plugin");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            pkg_dir.join("__init__.py").canonicalize().unwrap()
        );
    }

    #[test]
    fn test_resolve_entry_point_module_to_path_submodule() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package with a submodule
        let pkg_dir = site_packages.join("my_plugin");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();
        fs::write(pkg_dir.join("plugin.py"), "# plugin code").unwrap();

        // Should resolve to plugin.py (canonicalized)
        let result =
            FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "my_plugin.plugin");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            pkg_dir.join("plugin.py").canonicalize().unwrap()
        );
    }

    #[test]
    fn test_resolve_entry_point_module_to_path_single_file() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a single-file module
        fs::write(site_packages.join("my_plugin.py"), "# plugin code").unwrap();

        // Should resolve to my_plugin.py (canonicalized)
        let result =
            FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "my_plugin");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            site_packages.join("my_plugin.py").canonicalize().unwrap()
        );
    }

    #[test]
    fn test_resolve_entry_point_module_to_path_not_found() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Nothing exists
        let result = FixtureDatabase::resolve_entry_point_module_to_path(
            site_packages,
            "nonexistent_plugin",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_entry_point_module_strips_attr() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package with a submodule
        let pkg_dir = site_packages.join("my_plugin");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();
        fs::write(pkg_dir.join("module.py"), "# plugin code").unwrap();

        // Should resolve even with :attr suffix (canonicalized)
        let result = FixtureDatabase::resolve_entry_point_module_to_path(
            site_packages,
            "my_plugin.module:entry_function",
        );
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            pkg_dir.join("module.py").canonicalize().unwrap()
        );
    }

    #[test]
    fn test_resolve_entry_point_rejects_path_traversal() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a valid module so the path would resolve if not for validation
        fs::write(site_packages.join("valid.py"), "# code").unwrap();

        // ".." in module path
        let result =
            FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "..%2Fetc%2Fpasswd");
        assert!(result.is_none(), "should reject path with ..");

        let result =
            FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "valid...secret");
        assert!(result.is_none(), "should reject dotdot segments");
    }

    #[test]
    fn test_resolve_entry_point_rejects_null_bytes() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        let result =
            FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "module\0name");
        assert!(result.is_none(), "should reject null bytes");
    }

    #[test]
    fn test_resolve_entry_point_rejects_empty_segments() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // "foo..bar" splits on '.' to ["foo", "", "bar"]
        let result = FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "foo..bar");
        assert!(result.is_none(), "should reject empty path segments");
    }

    #[test]
    fn test_resolve_entry_point_rejects_symlink_escape() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create an outside directory with a .py file
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("evil.py"), "# malicious").unwrap();

        // Create a symlink inside site-packages pointing outside
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), site_packages.join("escaped")).unwrap();

            let result =
                FixtureDatabase::resolve_entry_point_module_to_path(site_packages, "escaped.evil");
            assert!(
                result.is_none(),
                "should reject paths that escape site-packages via symlink"
            );
        }
    }

    #[test]
    fn test_entry_point_plugin_discovery_integration() {
        // Create mock site-packages structure
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a mock plugin package
        let plugin_dir = site_packages.join("my_pytest_plugin");
        fs::create_dir_all(&plugin_dir).unwrap();

        let plugin_content = r#"
import pytest

@pytest.fixture
def my_dynamic_fixture():
    """A fixture discovered via entry points."""
    return "discovered via entry point"

@pytest.fixture
def another_dynamic_fixture():
    return 42
"#;
        fs::write(plugin_dir.join("__init__.py"), plugin_content).unwrap();

        // Create dist-info with entry points
        let dist_info = site_packages.join("my_pytest_plugin-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();

        let entry_points = "[pytest11]\nmy_plugin = my_pytest_plugin\n";
        fs::write(dist_info.join("entry_points.txt"), entry_points).unwrap();

        // Scan and verify
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("my_dynamic_fixture"),
            "my_dynamic_fixture should be discovered"
        );
        assert!(
            db.definitions.contains_key("another_dynamic_fixture"),
            "another_dynamic_fixture should be discovered"
        );
    }

    #[test]
    fn test_entry_point_discovery_submodule() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create package with plugin in submodule (like pytest_asyncio.plugin)
        let plugin_dir = site_packages.join("my_pytest_plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("__init__.py"), "# main init").unwrap();

        let plugin_content = r#"
import pytest

@pytest.fixture
def submodule_fixture():
    return "from submodule"
"#;
        fs::write(plugin_dir.join("plugin.py"), plugin_content).unwrap();

        // Create dist-info with entry points pointing to submodule
        let dist_info = site_packages.join("my_pytest_plugin-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();

        let entry_points = "[pytest11]\nmy_plugin = my_pytest_plugin.plugin\n";
        fs::write(dist_info.join("entry_points.txt"), entry_points).unwrap();

        // Scan and verify
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("submodule_fixture"),
            "submodule_fixture should be discovered"
        );
    }

    #[test]
    fn test_entry_point_discovery_package_scans_submodules() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create package with fixtures in a submodule
        let plugin_dir = site_packages.join("my_pytest_plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("__init__.py"), "# package init").unwrap();

        let plugin_content = r#"
import pytest

@pytest.fixture
def package_submodule_fixture():
    return "from package submodule"
"#;
        fs::write(plugin_dir.join("fixtures.py"), plugin_content).unwrap();

        // Create dist-info with entry points pointing to package
        let dist_info = site_packages.join("my_pytest_plugin-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();

        let entry_points = "[pytest11]\nmy_plugin = my_pytest_plugin\n";
        fs::write(dist_info.join("entry_points.txt"), entry_points).unwrap();

        // Scan and verify submodule fixtures are discovered
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("package_submodule_fixture"),
            "package_submodule_fixture should be discovered"
        );
    }

    #[test]
    fn test_entry_point_discovery_no_pytest11_section() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package that's NOT a pytest plugin
        let pkg_dir = site_packages.join("some_package");
        fs::create_dir_all(&pkg_dir).unwrap();

        let pkg_content = r#"
import pytest

@pytest.fixture
def should_not_be_found():
    return "this package is not a pytest plugin"
"#;
        fs::write(pkg_dir.join("__init__.py"), pkg_content).unwrap();

        // Create dist-info WITHOUT pytest11 section
        let dist_info = site_packages.join("some_package-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();

        let entry_points = "[console_scripts]\nsome_cli = some_package:main\n";
        fs::write(dist_info.join("entry_points.txt"), entry_points).unwrap();

        // Scan and verify
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            !db.definitions.contains_key("should_not_be_found"),
            "should_not_be_found should NOT be discovered (not a pytest plugin)"
        );
    }

    #[test]
    fn test_entry_point_discovery_missing_entry_points_txt() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package
        let pkg_dir = site_packages.join("some_package");
        fs::create_dir_all(&pkg_dir).unwrap();

        let pkg_content = r#"
import pytest

@pytest.fixture
def should_not_be_found():
    return "no entry_points.txt"
"#;
        fs::write(pkg_dir.join("__init__.py"), pkg_content).unwrap();

        // Create dist-info WITHOUT entry_points.txt file
        let dist_info = site_packages.join("some_package-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();
        // Don't create entry_points.txt

        // Scan and verify
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            !db.definitions.contains_key("should_not_be_found"),
            "should_not_be_found should NOT be discovered (no entry_points.txt)"
        );
    }

    #[test]
    fn test_entry_point_discovery_egg_info() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package
        let pkg_dir = site_packages.join("legacy_plugin");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("__init__.py"),
            r#"
import pytest

@pytest.fixture
def legacy_plugin_fixture():
    return "from egg-info"
"#,
        )
        .unwrap();

        // Create egg-info with entry points
        let egg_info = site_packages.join("legacy_plugin-1.0.0.egg-info");
        fs::create_dir_all(&egg_info).unwrap();
        let entry_points = "[pytest11]\nlegacy_plugin = legacy_plugin\n";
        fs::write(egg_info.join("entry_points.txt"), entry_points).unwrap();

        // Scan and verify
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("legacy_plugin_fixture"),
            "legacy_plugin_fixture should be discovered"
        );
    }

    #[test]
    fn test_entry_point_discovery_multiple_plugins() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create first plugin
        let plugin1_dir = site_packages.join("plugin_one");
        fs::create_dir_all(&plugin1_dir).unwrap();
        fs::write(
            plugin1_dir.join("__init__.py"),
            r#"
import pytest

@pytest.fixture
def fixture_from_plugin_one():
    return 1
"#,
        )
        .unwrap();

        let dist_info1 = site_packages.join("plugin_one-1.0.0.dist-info");
        fs::create_dir_all(&dist_info1).unwrap();
        fs::write(
            dist_info1.join("entry_points.txt"),
            "[pytest11]\nplugin_one = plugin_one\n",
        )
        .unwrap();

        // Create second plugin
        let plugin2_dir = site_packages.join("plugin_two");
        fs::create_dir_all(&plugin2_dir).unwrap();
        fs::write(
            plugin2_dir.join("__init__.py"),
            r#"
import pytest

@pytest.fixture
def fixture_from_plugin_two():
    return 2
"#,
        )
        .unwrap();

        let dist_info2 = site_packages.join("plugin_two-2.0.0.dist-info");
        fs::create_dir_all(&dist_info2).unwrap();
        fs::write(
            dist_info2.join("entry_points.txt"),
            "[pytest11]\nplugin_two = plugin_two\n",
        )
        .unwrap();

        // Scan and verify both are discovered
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("fixture_from_plugin_one"),
            "fixture_from_plugin_one should be discovered"
        );
        assert!(
            db.definitions.contains_key("fixture_from_plugin_two"),
            "fixture_from_plugin_two should be discovered"
        );
    }

    #[test]
    fn test_entry_point_discovery_multiple_entries_in_one_package() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a package with multiple plugin modules
        let plugin_dir = site_packages.join("multi_plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("__init__.py"), "").unwrap();

        fs::write(
            plugin_dir.join("fixtures_a.py"),
            r#"
import pytest

@pytest.fixture
def fixture_a():
    return "A"
"#,
        )
        .unwrap();

        fs::write(
            plugin_dir.join("fixtures_b.py"),
            r#"
import pytest

@pytest.fixture
def fixture_b():
    return "B"
"#,
        )
        .unwrap();

        // Create dist-info with multiple pytest11 entries
        let dist_info = site_packages.join("multi_plugin-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();
        fs::write(
            dist_info.join("entry_points.txt"),
            r#"[pytest11]
fixtures_a = multi_plugin.fixtures_a
fixtures_b = multi_plugin.fixtures_b
"#,
        )
        .unwrap();

        // Scan and verify both modules are scanned
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("fixture_a"),
            "fixture_a should be discovered"
        );
        assert!(
            db.definitions.contains_key("fixture_b"),
            "fixture_b should be discovered"
        );
    }

    #[test]
    fn test_pytest_internal_fixtures_scanned() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create mock _pytest directory (pytest's internal package)
        let pytest_internal = site_packages.join("_pytest");
        fs::create_dir_all(&pytest_internal).unwrap();

        let internal_fixtures = r#"
import pytest

@pytest.fixture
def tmp_path():
    """Pytest's built-in tmp_path fixture."""
    pass

@pytest.fixture
def capsys():
    """Pytest's built-in capsys fixture."""
    pass
"#;
        fs::write(pytest_internal.join("fixtures.py"), internal_fixtures).unwrap();

        // Scan and verify internal fixtures are discovered
        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        // Note: We're checking that _pytest is scanned as a special case
        // even without entry points
        assert!(
            db.definitions.contains_key("tmp_path"),
            "tmp_path should be discovered from _pytest"
        );
        assert!(
            db.definitions.contains_key("capsys"),
            "capsys should be discovered from _pytest"
        );
    }

    #[test]
    fn test_extract_package_name_from_dist_info() {
        assert_eq!(
            FixtureDatabase::extract_package_name_from_dist_info("mypackage-1.0.0.dist-info"),
            Some(("mypackage".to_string(), "mypackage".to_string()))
        );
        assert_eq!(
            FixtureDatabase::extract_package_name_from_dist_info("my-package-1.0.0.dist-info"),
            Some(("my-package".to_string(), "my_package".to_string()))
        );
        assert_eq!(
            FixtureDatabase::extract_package_name_from_dist_info("My.Package-2.3.4.dist-info"),
            Some(("My.Package".to_string(), "my_package".to_string()))
        );
        assert_eq!(
            FixtureDatabase::extract_package_name_from_dist_info("pytest_mock-3.12.0.dist-info"),
            Some(("pytest_mock".to_string(), "pytest_mock".to_string()))
        );
        assert_eq!(
            FixtureDatabase::extract_package_name_from_dist_info("mypackage-0.1.0.egg-info"),
            Some(("mypackage".to_string(), "mypackage".to_string()))
        );
        // Edge case: no version
        assert_eq!(
            FixtureDatabase::extract_package_name_from_dist_info("mypackage.dist-info"),
            Some(("mypackage".to_string(), "mypackage".to_string()))
        );
    }

    #[test]
    fn test_discover_editable_installs() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a source root for the editable package
        let source_root = tempdir().unwrap();
        let pkg_dir = source_root.path().join("mypackage");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();

        // Create dist-info with direct_url.json indicating editable
        let dist_info = site_packages.join("mypackage-0.1.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();

        let direct_url = serde_json::json!({
            "url": format!("file://{}", source_root.path().display()),
            "dir_info": {
                "editable": true
            }
        });
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_string(&direct_url).unwrap(),
        )
        .unwrap();

        // Create a .pth file pointing to the source root
        let pth_content = format!("{}\n", source_root.path().display());
        fs::write(
            site_packages.join("__editable__.mypackage-0.1.0.pth"),
            &pth_content,
        )
        .unwrap();

        let db = FixtureDatabase::new();
        db.discover_editable_installs(site_packages);

        let installs = db.editable_install_roots.lock().unwrap();
        assert_eq!(installs.len(), 1, "Should discover one editable install");
        assert_eq!(installs[0].package_name, "mypackage");
        assert_eq!(
            installs[0].source_root,
            source_root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_discover_editable_installs_pth_with_dashes() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a source root
        let source_root = tempdir().unwrap();
        let pkg_dir = source_root.path().join("my_package");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();

        // dist-info uses dashes (PEP 427): my-package-0.1.0.dist-info
        let dist_info = site_packages.join("my-package-0.1.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();
        let direct_url = serde_json::json!({
            "url": format!("file://{}", source_root.path().display()),
            "dir_info": { "editable": true }
        });
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_string(&direct_url).unwrap(),
        )
        .unwrap();

        // .pth file keeps dashes (matches pip's actual behavior)
        let pth_content = format!("{}\n", source_root.path().display());
        fs::write(
            site_packages.join("__editable__.my-package-0.1.0.pth"),
            &pth_content,
        )
        .unwrap();

        let db = FixtureDatabase::new();
        db.discover_editable_installs(site_packages);

        let installs = db.editable_install_roots.lock().unwrap();
        assert_eq!(
            installs.len(),
            1,
            "Should discover editable install from .pth with dashes"
        );
        assert_eq!(installs[0].package_name, "my_package");
        assert_eq!(
            installs[0].source_root,
            source_root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_discover_editable_installs_pth_with_dots() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a source root
        let source_root = tempdir().unwrap();
        fs::create_dir_all(source_root.path().join("my_package")).unwrap();
        fs::write(source_root.path().join("my_package/__init__.py"), "").unwrap();

        // dist-info uses dots: My.Package-1.0.0.dist-info
        let dist_info = site_packages.join("My.Package-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();
        let direct_url = serde_json::json!({
            "url": format!("file://{}", source_root.path().display()),
            "dir_info": { "editable": true }
        });
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_string(&direct_url).unwrap(),
        )
        .unwrap();

        // .pth file keeps dots
        let pth_content = format!("{}\n", source_root.path().display());
        fs::write(
            site_packages.join("__editable__.My.Package-1.0.0.pth"),
            &pth_content,
        )
        .unwrap();

        let db = FixtureDatabase::new();
        db.discover_editable_installs(site_packages);

        let installs = db.editable_install_roots.lock().unwrap();
        assert_eq!(
            installs.len(),
            1,
            "Should discover editable install from .pth with dots"
        );
        assert_eq!(installs[0].package_name, "my_package");
        assert_eq!(
            installs[0].source_root,
            source_root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_discover_editable_installs_dedup_on_rescan() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        let source_root = tempdir().unwrap();
        fs::create_dir_all(source_root.path().join("pkg")).unwrap();
        fs::write(source_root.path().join("pkg/__init__.py"), "").unwrap();

        let dist_info = site_packages.join("pkg-0.1.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();
        let direct_url = serde_json::json!({
            "url": format!("file://{}", source_root.path().display()),
            "dir_info": { "editable": true }
        });
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_string(&direct_url).unwrap(),
        )
        .unwrap();

        let pth_content = format!("{}\n", source_root.path().display());
        fs::write(site_packages.join("pkg.pth"), &pth_content).unwrap();

        let db = FixtureDatabase::new();

        // Scan twice
        db.discover_editable_installs(site_packages);
        db.discover_editable_installs(site_packages);

        let installs = db.editable_install_roots.lock().unwrap();
        assert_eq!(
            installs.len(),
            1,
            "Re-scanning should not produce duplicates"
        );
    }

    #[test]
    fn test_editable_install_entry_point_resolution() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        // Create a source root with a plugin module
        let source_root = tempdir().unwrap();
        let pkg_dir = source_root.path().join("mypackage");
        fs::create_dir_all(&pkg_dir).unwrap();

        let plugin_content = r#"
import pytest

@pytest.fixture
def editable_fixture():
    return "from editable install"
"#;
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();
        fs::write(pkg_dir.join("plugin.py"), plugin_content).unwrap();

        // Create dist-info with direct_url.json and entry_points.txt
        let dist_info = site_packages.join("mypackage-0.1.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();

        let direct_url = serde_json::json!({
            "url": format!("file://{}", source_root.path().display()),
            "dir_info": { "editable": true }
        });
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_string(&direct_url).unwrap(),
        )
        .unwrap();

        let entry_points = "[pytest11]\nmypackage = mypackage.plugin\n";
        fs::write(dist_info.join("entry_points.txt"), entry_points).unwrap();

        // Create .pth file
        let pth_content = format!("{}\n", source_root.path().display());
        fs::write(
            site_packages.join("__editable__.mypackage-0.1.0.pth"),
            &pth_content,
        )
        .unwrap();

        let db = FixtureDatabase::new();
        db.scan_pytest_plugins(site_packages);

        assert!(
            db.definitions.contains_key("editable_fixture"),
            "editable_fixture should be discovered via entry point fallback"
        );
    }

    #[test]
    fn test_discover_editable_installs_namespace_package() {
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        let source_root = tempdir().unwrap();
        let pkg_dir = source_root.path().join("namespace").join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();

        let dist_info = site_packages.join("namespace.pkg-1.0.0.dist-info");
        fs::create_dir_all(&dist_info).unwrap();
        let direct_url = serde_json::json!({
            "url": format!("file://{}", source_root.path().display()),
            "dir_info": { "editable": true }
        });
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_string(&direct_url).unwrap(),
        )
        .unwrap();

        let pth_content = format!("{}\n", source_root.path().display());
        fs::write(
            site_packages.join("__editable__.namespace.pkg-1.0.0.pth"),
            &pth_content,
        )
        .unwrap();

        let db = FixtureDatabase::new();
        db.discover_editable_installs(site_packages);

        let installs = db.editable_install_roots.lock().unwrap();
        assert_eq!(
            installs.len(),
            1,
            "Should discover namespace editable install"
        );
        assert_eq!(installs[0].package_name, "namespace_pkg");
        assert_eq!(installs[0].raw_package_name, "namespace.pkg");
        assert_eq!(
            installs[0].source_root,
            source_root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_pth_prefix_matching_no_false_positive() {
        // "foo" candidate should NOT match "foo-bar.pth" (different package)
        let temp = tempdir().unwrap();
        let site_packages = temp.path();

        let source_root_foo = tempdir().unwrap();
        fs::create_dir_all(source_root_foo.path()).unwrap();

        let source_root_foobar = tempdir().unwrap();
        fs::create_dir_all(source_root_foobar.path()).unwrap();

        // Create foo-bar.pth pointing to foobar source
        fs::write(
            site_packages.join("foo-bar.pth"),
            format!("{}\n", source_root_foobar.path().display()),
        )
        .unwrap();

        let pth_index = FixtureDatabase::build_pth_index(site_packages);

        // "foo" should NOT match "foo-bar" (different package, not a version suffix)
        let result =
            FixtureDatabase::find_editable_pth_source_root(&pth_index, "foo", "foo", site_packages);
        assert!(
            result.is_none(),
            "foo should not match foo-bar.pth (different package)"
        );

        // "foo-bar" exact match should work
        let result = FixtureDatabase::find_editable_pth_source_root(
            &pth_index,
            "foo-bar",
            "foo_bar",
            site_packages,
        );
        assert!(result.is_some(), "foo-bar should match foo-bar.pth exactly");
    }
}
