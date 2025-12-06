use dashmap::DashMap;
use rayon::prelude::*;
use rustpython_parser::ast::{ArgWithDefault, Arguments, Expr, Ranged, Stmt};
use rustpython_parser::{parse, Mode};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureDefinition {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize, // Character position where the fixture name starts (on the line)
    pub end_char: usize,   // Character position where the fixture name ends (on the line)
    pub docstring: Option<String>,
    pub return_type: Option<String>, // The return type annotation (for generators, the yielded type)
}

#[derive(Debug, Clone)]
pub struct FixtureUsage {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize, // Character position where this usage starts (on the line)
    pub end_char: usize,   // Character position where this usage ends (on the line)
}

#[derive(Debug, Clone)]
pub struct UndeclaredFixture {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub function_name: String, // Name of the test/fixture function where this is used
    pub function_line: usize,  // Line where the function is defined
}

/// Represents a location where a fixture can be renamed
#[derive(Debug, Clone, PartialEq)]
pub struct RenameLocation {
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize,
    pub end_char: usize,
}

/// Result of collecting rename locations for a fixture
#[derive(Debug, Clone)]
pub struct RenameInfo {
    /// The fixture definition being renamed
    pub definition: FixtureDefinition,
    /// All locations where the fixture name appears (definition + usages)
    pub locations: Vec<RenameLocation>,
}

/// Error type for rename operations
#[derive(Debug, Clone, PartialEq)]
pub enum RenameError {
    /// No fixture found at the specified position
    NoFixtureAtPosition,
    /// Cannot rename built-in fixtures (request, tmp_path, etc.)
    CannotRenameBuiltin(String),
    /// Cannot rename third-party fixtures (from site-packages)
    CannotRenameThirdParty(String),
    /// Invalid new name (not a valid Python identifier)
    InvalidName(String),
}

/// Context for code completion
#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    /// Inside a function signature (parameter list) - suggest fixtures as parameters
    FunctionSignature {
        function_name: String,
        function_line: usize,
        is_fixture: bool,
        declared_params: Vec<String>,
    },
    /// Inside a function body - suggest fixtures with auto-add to parameters
    FunctionBody {
        function_name: String,
        function_line: usize,
        is_fixture: bool,
        declared_params: Vec<String>,
    },
    /// Inside @pytest.mark.usefixtures("...") decorator - suggest fixture names as strings
    UsefixuturesDecorator,
    /// Inside @pytest.mark.parametrize(..., indirect=...) - suggest fixture names as strings
    ParametrizeIndirect,
}

/// Information about where to insert a new parameter in a function signature
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInsertionInfo {
    /// Line number (1-indexed) where the function signature is
    pub line: usize,
    /// Character position where the new parameter should be inserted
    pub char_pos: usize,
    /// Whether a comma needs to be added before the new parameter
    pub needs_comma: bool,
}

#[derive(Debug)]
pub struct FixtureDatabase {
    // Map from fixture name to all its definitions (can be in multiple conftest.py files)
    pub definitions: Arc<DashMap<String, Vec<FixtureDefinition>>>,
    // Map from file path to fixtures used in that file
    pub usages: Arc<DashMap<PathBuf, Vec<FixtureUsage>>>,
    // Cache of file contents for analyzed files (uses Arc for efficient sharing)
    pub file_cache: Arc<DashMap<PathBuf, Arc<String>>>,
    // Map from file path to undeclared fixtures used in function bodies
    pub undeclared_fixtures: Arc<DashMap<PathBuf, Vec<UndeclaredFixture>>>,
    // Map from file path to imported names in that file
    pub imports: Arc<DashMap<PathBuf, std::collections::HashSet<String>>>,
    // Cache of canonical paths to avoid repeated filesystem calls
    pub canonical_path_cache: Arc<DashMap<PathBuf, PathBuf>>,
}

impl Default for FixtureDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl FixtureDatabase {
    pub fn new() -> Self {
        Self {
            definitions: Arc::new(DashMap::new()),
            usages: Arc::new(DashMap::new()),
            file_cache: Arc::new(DashMap::new()),
            undeclared_fixtures: Arc::new(DashMap::new()),
            imports: Arc::new(DashMap::new()),
            canonical_path_cache: Arc::new(DashMap::new()),
        }
    }

    /// Get canonical path with caching to avoid repeated filesystem calls
    /// Falls back to original path if canonicalization fails
    fn get_canonical_path(&self, path: PathBuf) -> PathBuf {
        // Check cache first
        if let Some(cached) = self.canonical_path_cache.get(&path) {
            return cached.value().clone();
        }

        // Attempt canonicalization
        let canonical = path.canonicalize().unwrap_or_else(|_| {
            debug!("Could not canonicalize path {:?}, using as-is", path);
            path.clone()
        });

        // Store in cache for future lookups
        self.canonical_path_cache.insert(path, canonical.clone());
        canonical
    }

    /// Get file content from cache or read from filesystem
    /// Returns None if file cannot be read
    fn get_file_content(&self, file_path: &Path) -> Option<Arc<String>> {
        if let Some(cached) = self.file_cache.get(file_path) {
            Some(Arc::clone(cached.value()))
        } else {
            std::fs::read_to_string(file_path).ok().map(Arc::new)
        }
    }

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

    /// Check if a directory should be skipped during scanning
    fn should_skip_directory(dir_name: &str) -> bool {
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

    /// Scan a workspace directory for test files and conftest.py files
    pub fn scan_workspace(&self, root_path: &Path) {
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
        let mut files_to_process: Vec<PathBuf> = Vec::new();
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
        use std::sync::atomic::{AtomicUsize, Ordering};
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

    /// Scan virtual environment for pytest plugin fixtures
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
            let venv_path = PathBuf::from(venv);
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

    /// Analyze a single Python file for fixtures using AST parsing
    pub fn analyze_file(&self, file_path: PathBuf, content: &str) {
        self.analyze_file_internal(file_path, content, true);
    }

    /// Analyze a file without cleaning up previous definitions.
    /// Used during initial workspace scan when we know the database is empty.
    fn analyze_file_fresh(&self, file_path: PathBuf, content: &str) {
        self.analyze_file_internal(file_path, content, false);
    }

    /// Internal file analysis with optional cleanup of previous definitions
    fn analyze_file_internal(&self, file_path: PathBuf, content: &str, cleanup_previous: bool) {
        // Use cached canonical path to avoid repeated filesystem calls
        let file_path = self.get_canonical_path(file_path);

        debug!("Analyzing file: {:?}", file_path);

        // Cache the file content for later use (e.g., in find_fixture_definition)
        // Use Arc for efficient sharing without cloning
        self.file_cache
            .insert(file_path.clone(), Arc::new(content.to_string()));

        // Parse the Python code
        let parsed = match parse(content, Mode::Module, "") {
            Ok(ast) => ast,
            Err(e) => {
                error!("Failed to parse Python file {:?}: {}", file_path, e);
                return;
            }
        };

        // Clear previous usages for this file
        self.usages.remove(&file_path);

        // Clear previous undeclared fixtures for this file
        self.undeclared_fixtures.remove(&file_path);

        // Clear previous imports for this file
        self.imports.remove(&file_path);

        // Clear previous fixture definitions from this file (only when re-analyzing)
        // Skip this during initial workspace scan for performance
        if cleanup_previous {
            // We need to remove definitions that were in this file
            // IMPORTANT: Collect keys first to avoid deadlock. The issue is that
            // iter() holds read locks on the DashMap, and if we try to call .get() or
            // .insert() on the same map while iterating, we'll deadlock due to lock
            // contention. Collecting keys first releases the iterator locks before
            // we start mutating the map.
            let keys: Vec<String> = {
                let mut k = Vec::new();
                for entry in self.definitions.iter() {
                    k.push(entry.key().clone());
                }
                k
            }; // Iterator dropped here, all locks released

            // Now process each key individually
            for key in keys {
                // Get current definitions for this key
                let current_defs = match self.definitions.get(&key) {
                    Some(defs) => defs.clone(),
                    None => continue,
                };

                // Filter out definitions from this file
                let filtered: Vec<FixtureDefinition> = current_defs
                    .iter()
                    .filter(|def| def.file_path != file_path)
                    .cloned()
                    .collect();

                // Update or remove
                if filtered.is_empty() {
                    self.definitions.remove(&key);
                } else if filtered.len() != current_defs.len() {
                    // Only update if something changed
                    self.definitions.insert(key, filtered);
                }
            }
        }

        // Check if this is a conftest.py
        let is_conftest = file_path
            .file_name()
            .map(|n| n == "conftest.py")
            .unwrap_or(false);
        debug!("is_conftest: {}", is_conftest);

        // Build line index for O(1) line lookups
        let line_index = Self::build_line_index(content);

        // Process each statement in the module
        if let rustpython_parser::ast::Mod::Module(module) = parsed {
            debug!("Module has {} statements", module.body.len());

            // First pass: collect all module-level names (imports, assignments, function/class defs)
            let mut module_level_names = std::collections::HashSet::new();
            for stmt in &module.body {
                self.collect_module_level_names(stmt, &mut module_level_names);
            }
            self.imports.insert(file_path.clone(), module_level_names);

            // Second pass: analyze fixtures and tests
            for stmt in &module.body {
                self.visit_stmt(stmt, &file_path, is_conftest, content, &line_index);
            }
        }

        debug!("Analysis complete for {:?}", file_path);
    }

    fn visit_stmt(
        &self,
        stmt: &Stmt,
        file_path: &PathBuf,
        _is_conftest: bool,
        content: &str,
        line_index: &[usize],
    ) {
        // First check for assignment-style fixtures: fixture_name = pytest.fixture()(func)
        if let Stmt::Assign(assign) = stmt {
            self.visit_assignment_fixture(assign, file_path, content, line_index);
        }

        // Handle class definitions - recurse into class body to find test methods
        if let Stmt::ClassDef(class_def) = stmt {
            // Check for @pytest.mark.usefixtures decorator on the class
            for decorator in &class_def.decorator_list {
                let usefixtures = Self::extract_usefixtures_names(decorator);
                for (fixture_name, range) in usefixtures {
                    let usage_line =
                        self.get_line_from_offset(range.start().to_usize(), line_index);
                    let start_char =
                        self.get_char_position_from_offset(range.start().to_usize(), line_index);
                    // Add 1 to start_char and subtract 1 from end for the quotes around the string
                    let end_char =
                        self.get_char_position_from_offset(range.end().to_usize(), line_index);

                    info!(
                        "Found usefixtures usage on class: {} at {:?}:{}:{}",
                        fixture_name, file_path, usage_line, start_char
                    );

                    let usage = FixtureUsage {
                        name: fixture_name,
                        file_path: file_path.clone(),
                        line: usage_line,
                        start_char: start_char + 1, // Skip opening quote
                        end_char: end_char - 1,     // Skip closing quote
                    };

                    self.usages
                        .entry(file_path.clone())
                        .or_default()
                        .push(usage);
                }
            }

            for class_stmt in &class_def.body {
                self.visit_stmt(class_stmt, file_path, _is_conftest, content, line_index);
            }
            return;
        }

        // Handle both regular and async function definitions
        let (func_name, decorator_list, args, range, body, returns) = match stmt {
            Stmt::FunctionDef(func_def) => (
                func_def.name.as_str(),
                &func_def.decorator_list,
                &func_def.args,
                func_def.range,
                &func_def.body,
                &func_def.returns,
            ),
            Stmt::AsyncFunctionDef(func_def) => (
                func_def.name.as_str(),
                &func_def.decorator_list,
                &func_def.args,
                func_def.range,
                &func_def.body,
                &func_def.returns,
            ),
            _ => return,
        };

        debug!("Found function: {}", func_name);

        // Check for @pytest.mark.usefixtures decorator on the function
        for decorator in decorator_list {
            let usefixtures = Self::extract_usefixtures_names(decorator);
            for (fixture_name, range) in usefixtures {
                let usage_line = self.get_line_from_offset(range.start().to_usize(), line_index);
                let start_char =
                    self.get_char_position_from_offset(range.start().to_usize(), line_index);
                let end_char =
                    self.get_char_position_from_offset(range.end().to_usize(), line_index);

                info!(
                    "Found usefixtures usage on function: {} at {:?}:{}:{}",
                    fixture_name, file_path, usage_line, start_char
                );

                let usage = FixtureUsage {
                    name: fixture_name,
                    file_path: file_path.clone(),
                    line: usage_line,
                    start_char: start_char + 1, // Skip opening quote
                    end_char: end_char - 1,     // Skip closing quote
                };

                self.usages
                    .entry(file_path.clone())
                    .or_default()
                    .push(usage);
            }
        }

        // Check for @pytest.mark.parametrize with indirect=True on the function
        for decorator in decorator_list {
            let indirect_fixtures = Self::extract_parametrize_indirect_fixtures(decorator);
            for (fixture_name, range) in indirect_fixtures {
                let usage_line = self.get_line_from_offset(range.start().to_usize(), line_index);
                let start_char =
                    self.get_char_position_from_offset(range.start().to_usize(), line_index);
                let end_char =
                    self.get_char_position_from_offset(range.end().to_usize(), line_index);

                info!(
                    "Found parametrize indirect fixture usage: {} at {:?}:{}:{}",
                    fixture_name, file_path, usage_line, start_char
                );

                let usage = FixtureUsage {
                    name: fixture_name,
                    file_path: file_path.clone(),
                    line: usage_line,
                    start_char: start_char + 1, // Skip opening quote
                    end_char: end_char - 1,     // Skip closing quote
                };

                self.usages
                    .entry(file_path.clone())
                    .or_default()
                    .push(usage);
            }
        }

        // Check if this is a fixture definition
        debug!(
            "Function {} has {} decorators",
            func_name,
            decorator_list.len()
        );
        // Find the fixture decorator and check for renamed fixtures (name= parameter)
        let fixture_decorator = decorator_list
            .iter()
            .find(|dec| Self::is_fixture_decorator(dec));

        if let Some(decorator) = fixture_decorator {
            debug!("  Decorator matched as fixture!");

            // Check if the fixture has a custom name (e.g., @pytest.fixture(name="custom_name"))
            let fixture_name = Self::extract_fixture_name_from_decorator(decorator)
                .unwrap_or_else(|| func_name.to_string());

            // Calculate line number from the range start
            let line = self.get_line_from_offset(range.start().to_usize(), line_index);

            // Extract docstring if present
            let docstring = self.extract_docstring(body);

            // Extract return type annotation
            let return_type = self.extract_return_type(returns, body, content);

            info!(
                "Found fixture definition: {} (function: {}) at {:?}:{}",
                fixture_name, func_name, file_path, line
            );
            if let Some(ref doc) = docstring {
                debug!("  Docstring: {}", doc);
            }
            if let Some(ref ret_type) = return_type {
                debug!("  Return type: {}", ret_type);
            }

            // Calculate character positions for the fixture name
            // For renamed fixtures, use the function name position (since that's what gets renamed)
            // The fixture_name might differ from func_name if name= parameter was used
            let (start_char, end_char) = self.find_function_name_position(content, line, func_name);

            let definition = FixtureDefinition {
                name: fixture_name.clone(),
                file_path: file_path.clone(),
                line,
                start_char,
                end_char,
                docstring,
                return_type,
            };

            self.definitions
                .entry(fixture_name)
                .or_default()
                .push(definition);

            // Fixtures can depend on other fixtures - record these as usages too
            let mut declared_params: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            declared_params.insert("self".to_string());
            declared_params.insert("request".to_string());
            declared_params.insert(func_name.to_string()); // Exclude function name itself

            // Iterate over all argument types: positional-only, regular, and keyword-only
            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" && arg_name != "request" {
                    // Get the actual line where this parameter appears
                    // arg.def.range contains the location of the parameter name
                    let arg_line =
                        self.get_line_from_offset(arg.def.range.start().to_usize(), line_index);
                    let start_char = self.get_char_position_from_offset(
                        arg.def.range.start().to_usize(),
                        line_index,
                    );
                    let end_char = self
                        .get_char_position_from_offset(arg.def.range.end().to_usize(), line_index);

                    info!(
                        "Found fixture dependency: {} at {:?}:{}:{}",
                        arg_name, file_path, arg_line, start_char
                    );

                    let usage = FixtureUsage {
                        name: arg_name.to_string(),
                        file_path: file_path.clone(),
                        line: arg_line, // Use actual parameter line
                        start_char,
                        end_char,
                    };

                    self.usages
                        .entry(file_path.clone())
                        .or_default()
                        .push(usage);
                }
            }

            // Scan fixture body for undeclared fixture usages
            let function_line = self.get_line_from_offset(range.start().to_usize(), line_index);
            self.scan_function_body_for_undeclared_fixtures(
                body,
                file_path,
                content,
                line_index,
                &declared_params,
                func_name,
                function_line,
            );
        }

        // Check if this is a test function
        let is_test = func_name.starts_with("test_");

        if is_test {
            debug!("Found test function: {}", func_name);

            // Collect declared parameters
            let mut declared_params: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            declared_params.insert("self".to_string());
            declared_params.insert("request".to_string()); // pytest built-in

            // Extract fixture usages from function parameters
            // Iterate over all argument types: positional-only, regular, and keyword-only
            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" {
                    // Get the actual line where this parameter appears
                    // This handles multiline function signatures correctly
                    // arg.def.range contains the location of the parameter name
                    let arg_offset = arg.def.range.start().to_usize();
                    let arg_line = self.get_line_from_offset(arg_offset, line_index);
                    let start_char = self.get_char_position_from_offset(arg_offset, line_index);
                    let end_char = self
                        .get_char_position_from_offset(arg.def.range.end().to_usize(), line_index);

                    debug!(
                        "Parameter {} at offset {}, calculated line {}, char {}",
                        arg_name, arg_offset, arg_line, start_char
                    );
                    info!(
                        "Found fixture usage: {} at {:?}:{}:{}",
                        arg_name, file_path, arg_line, start_char
                    );

                    let usage = FixtureUsage {
                        name: arg_name.to_string(),
                        file_path: file_path.clone(),
                        line: arg_line, // Use actual parameter line
                        start_char,
                        end_char,
                    };

                    // Append to existing usages for this file
                    self.usages
                        .entry(file_path.clone())
                        .or_default()
                        .push(usage);
                }
            }

            // Now scan the function body for undeclared fixture usages
            let function_line = self.get_line_from_offset(range.start().to_usize(), line_index);
            self.scan_function_body_for_undeclared_fixtures(
                body,
                file_path,
                content,
                line_index,
                &declared_params,
                func_name,
                function_line,
            );
        }
    }

    fn visit_assignment_fixture(
        &self,
        assign: &rustpython_parser::ast::StmtAssign,
        file_path: &PathBuf,
        _content: &str,
        line_index: &[usize],
    ) {
        // Check for pattern: fixture_name = pytest.fixture()(func)
        // The value should be a Call expression where the func is a Call to pytest.fixture()

        if let Expr::Call(outer_call) = &*assign.value {
            // Check if outer_call.func is pytest.fixture() or fixture()
            if let Expr::Call(inner_call) = &*outer_call.func {
                if Self::is_fixture_decorator(&inner_call.func) {
                    // This is pytest.fixture()(something)
                    // Get the fixture name from the assignment target
                    for target in &assign.targets {
                        if let Expr::Name(name) = target {
                            let fixture_name = name.id.as_str();
                            let line = self
                                .get_line_from_offset(assign.range.start().to_usize(), line_index);

                            // Get character positions from the name expression's range
                            let start_char = self.get_char_position_from_offset(
                                name.range.start().to_usize(),
                                line_index,
                            );
                            let end_char = self.get_char_position_from_offset(
                                name.range.end().to_usize(),
                                line_index,
                            );

                            info!(
                                "Found fixture assignment: {} at {:?}:{}:{}-{}",
                                fixture_name, file_path, line, start_char, end_char
                            );

                            // We don't have a docstring or return type for assignment-style fixtures
                            let definition = FixtureDefinition {
                                name: fixture_name.to_string(),
                                file_path: file_path.clone(),
                                line,
                                start_char,
                                end_char,
                                docstring: None,
                                return_type: None,
                            };

                            self.definitions
                                .entry(fixture_name.to_string())
                                .or_default()
                                .push(definition);
                        }
                    }
                }
            }
        }
    }

    /// Returns an iterator over all function arguments including positional-only,
    /// regular positional, and keyword-only arguments.
    /// This is needed because pytest fixtures can be declared as any of these types.
    fn all_args(args: &Arguments) -> impl Iterator<Item = &ArgWithDefault> {
        args.posonlyargs
            .iter()
            .chain(args.args.iter())
            .chain(args.kwonlyargs.iter())
    }

    fn is_fixture_decorator(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "fixture",
            Expr::Attribute(attr) => {
                // Check for pytest.fixture
                if let Expr::Name(value) = &*attr.value {
                    value.id.as_str() == "pytest" && attr.attr.as_str() == "fixture"
                } else {
                    false
                }
            }
            Expr::Call(call) => {
                // Handle @pytest.fixture() or @fixture() with parentheses
                Self::is_fixture_decorator(&call.func)
            }
            _ => false,
        }
    }

    /// Extracts the fixture name from a decorator's `name=` argument if present.
    fn extract_fixture_name_from_decorator(expr: &Expr) -> Option<String> {
        let Expr::Call(call) = expr else { return None };
        if !Self::is_fixture_decorator(&call.func) {
            return None;
        }

        call.keywords
            .iter()
            .filter(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "name"))
            .find_map(|kw| match &kw.value {
                Expr::Constant(c) => match &c.value {
                    rustpython_parser::ast::Constant::Str(s) => Some(s.to_string()),
                    _ => None,
                },
                _ => None,
            })
    }

    /// Checks if an expression is a pytest.mark.usefixtures decorator.
    /// Handles both @pytest.mark.usefixtures("fix") and @mark.usefixtures("fix")
    fn is_usefixtures_decorator(expr: &Expr) -> bool {
        match expr {
            Expr::Call(call) => Self::is_usefixtures_decorator(&call.func),
            Expr::Attribute(attr) => {
                // Check for pytest.mark.usefixtures or mark.usefixtures
                if attr.attr.as_str() != "usefixtures" {
                    return false;
                }
                match &*attr.value {
                    // pytest.mark.usefixtures
                    Expr::Attribute(inner_attr) => {
                        if inner_attr.attr.as_str() != "mark" {
                            return false;
                        }
                        matches!(&*inner_attr.value, Expr::Name(name) if name.id.as_str() == "pytest")
                    }
                    // mark.usefixtures (when imported as from pytest import mark)
                    Expr::Name(name) => name.id.as_str() == "mark",
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Extracts fixture names from @pytest.mark.usefixtures("fix1", "fix2", ...) decorator.
    /// Returns a vector of (fixture_name, range) tuples.
    fn extract_usefixtures_names(
        expr: &Expr,
    ) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
        let Expr::Call(call) = expr else {
            return vec![];
        };
        if !Self::is_usefixtures_decorator(&call.func) {
            return vec![];
        }

        call.args
            .iter()
            .filter_map(|arg| {
                if let Expr::Constant(c) = arg {
                    if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                        return Some((s.to_string(), c.range));
                    }
                }
                None
            })
            .collect()
    }

    /// Checks if an expression is a pytest.mark.parametrize decorator.
    fn is_parametrize_decorator(expr: &Expr) -> bool {
        match expr {
            Expr::Call(call) => Self::is_parametrize_decorator(&call.func),
            Expr::Attribute(attr) => {
                if attr.attr.as_str() != "parametrize" {
                    return false;
                }
                match &*attr.value {
                    // pytest.mark.parametrize
                    Expr::Attribute(inner_attr) => {
                        if inner_attr.attr.as_str() != "mark" {
                            return false;
                        }
                        matches!(&*inner_attr.value, Expr::Name(name) if name.id.as_str() == "pytest")
                    }
                    // mark.parametrize (when imported as from pytest import mark)
                    Expr::Name(name) => name.id.as_str() == "mark",
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Extracts fixture names from @pytest.mark.parametrize when indirect=True.
    /// Returns a vector of (fixture_name, range) tuples.
    ///
    /// Handles:
    /// - @pytest.mark.parametrize("fixture_name", [...], indirect=True)
    /// - @pytest.mark.parametrize("fix1,fix2", [...], indirect=True)
    /// - @pytest.mark.parametrize("fix1,fix2", [...], indirect=["fix1"])
    fn extract_parametrize_indirect_fixtures(
        expr: &Expr,
    ) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
        let Expr::Call(call) = expr else {
            return vec![];
        };
        if !Self::is_parametrize_decorator(&call.func) {
            return vec![];
        }

        // Check for indirect keyword argument
        let indirect_value = call.keywords.iter().find_map(|kw| {
            if kw.arg.as_ref().is_some_and(|a| a.as_str() == "indirect") {
                Some(&kw.value)
            } else {
                None
            }
        });

        let Some(indirect) = indirect_value else {
            return vec![];
        };

        // Get the first positional argument (parameter names)
        let Some(first_arg) = call.args.first() else {
            return vec![];
        };

        let Expr::Constant(param_const) = first_arg else {
            return vec![];
        };

        let rustpython_parser::ast::Constant::Str(param_str) = &param_const.value else {
            return vec![];
        };

        // Parse parameter names (can be comma-separated)
        let param_names: Vec<&str> = param_str.split(',').map(|s| s.trim()).collect();

        match indirect {
            // indirect=True means all parameters are fixtures
            Expr::Constant(c) => {
                if matches!(c.value, rustpython_parser::ast::Constant::Bool(true)) {
                    return param_names
                        .into_iter()
                        .map(|name| (name.to_string(), param_const.range))
                        .collect();
                }
            }
            // indirect=["fix1", "fix2"] means only listed parameters are fixtures
            Expr::List(list) => {
                return list
                    .elts
                    .iter()
                    .filter_map(|elt| {
                        if let Expr::Constant(c) = elt {
                            if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                                if param_names.contains(&s.as_str()) {
                                    return Some((s.to_string(), c.range));
                                }
                            }
                        }
                        None
                    })
                    .collect();
            }
            _ => {}
        }

        vec![]
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_function_body_for_undeclared_fixtures(
        &self,
        body: &[Stmt],
        file_path: &PathBuf,
        content: &str,
        line_index: &[usize],
        declared_params: &std::collections::HashSet<String>,
        function_name: &str,
        function_line: usize,
    ) {
        // First, collect all local variable names with their definition line numbers
        let mut local_vars = std::collections::HashMap::new();
        self.collect_local_variables(body, content, line_index, &mut local_vars);

        // Also add imported names to local_vars (they shouldn't be flagged as undeclared fixtures)
        // We set their line to 0 so they're treated as always in scope (line 0 < any actual usage line)
        if let Some(imports) = self.imports.get(file_path) {
            for import in imports.iter() {
                local_vars.insert(import.clone(), 0);
            }
        }

        // Walk through the function body and find all Name references
        for stmt in body {
            self.visit_stmt_for_names(
                stmt,
                file_path,
                content,
                line_index,
                declared_params,
                &local_vars,
                function_name,
                function_line,
            );
        }
    }

    fn collect_module_level_names(
        &self,
        stmt: &Stmt,
        names: &mut std::collections::HashSet<String>,
    ) {
        match stmt {
            // Imports
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    // If there's an "as" alias, use that; otherwise use the original name
                    let name = alias.asname.as_ref().unwrap_or(&alias.name);
                    names.insert(name.to_string());
                }
            }
            Stmt::ImportFrom(import_from) => {
                for alias in &import_from.names {
                    // If there's an "as" alias, use that; otherwise use the original name
                    let name = alias.asname.as_ref().unwrap_or(&alias.name);
                    names.insert(name.to_string());
                }
            }
            // Regular function definitions (not fixtures)
            Stmt::FunctionDef(func_def) => {
                // Check if this is NOT a fixture
                let is_fixture = func_def
                    .decorator_list
                    .iter()
                    .any(Self::is_fixture_decorator);
                if !is_fixture {
                    names.insert(func_def.name.to_string());
                }
            }
            // Async function definitions (not fixtures)
            Stmt::AsyncFunctionDef(func_def) => {
                let is_fixture = func_def
                    .decorator_list
                    .iter()
                    .any(Self::is_fixture_decorator);
                if !is_fixture {
                    names.insert(func_def.name.to_string());
                }
            }
            // Class definitions
            Stmt::ClassDef(class_def) => {
                names.insert(class_def.name.to_string());
            }
            // Module-level assignments
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    self.collect_names_from_expr(target, names);
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                self.collect_names_from_expr(&ann_assign.target, names);
            }
            _ => {}
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_local_variables(
        &self,
        body: &[Stmt],
        content: &str,
        line_index: &[usize],
        local_vars: &mut std::collections::HashMap<String, usize>,
    ) {
        for stmt in body {
            match stmt {
                Stmt::Assign(assign) => {
                    // Collect variable names from left-hand side with their line numbers
                    let line =
                        self.get_line_from_offset(assign.range.start().to_usize(), line_index);
                    let mut temp_names = std::collections::HashSet::new();
                    for target in &assign.targets {
                        self.collect_names_from_expr(target, &mut temp_names);
                    }
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    // Collect annotated assignment targets with their line numbers
                    let line =
                        self.get_line_from_offset(ann_assign.range.start().to_usize(), line_index);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&ann_assign.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::AugAssign(aug_assign) => {
                    // Collect augmented assignment targets (+=, -=, etc.)
                    let line =
                        self.get_line_from_offset(aug_assign.range.start().to_usize(), line_index);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&aug_assign.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::For(for_stmt) => {
                    // Collect loop variable with its line number
                    let line =
                        self.get_line_from_offset(for_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&for_stmt.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                    // Recursively collect from body
                    self.collect_local_variables(&for_stmt.body, content, line_index, local_vars);
                }
                Stmt::AsyncFor(for_stmt) => {
                    let line =
                        self.get_line_from_offset(for_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&for_stmt.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                    self.collect_local_variables(&for_stmt.body, content, line_index, local_vars);
                }
                Stmt::While(while_stmt) => {
                    self.collect_local_variables(&while_stmt.body, content, line_index, local_vars);
                }
                Stmt::If(if_stmt) => {
                    self.collect_local_variables(&if_stmt.body, content, line_index, local_vars);
                    self.collect_local_variables(&if_stmt.orelse, content, line_index, local_vars);
                }
                Stmt::With(with_stmt) => {
                    // Collect context manager variables with their line numbers
                    let line =
                        self.get_line_from_offset(with_stmt.range.start().to_usize(), line_index);
                    for item in &with_stmt.items {
                        if let Some(ref optional_vars) = item.optional_vars {
                            let mut temp_names = std::collections::HashSet::new();
                            self.collect_names_from_expr(optional_vars, &mut temp_names);
                            for name in temp_names {
                                local_vars.insert(name, line);
                            }
                        }
                    }
                    self.collect_local_variables(&with_stmt.body, content, line_index, local_vars);
                }
                Stmt::AsyncWith(with_stmt) => {
                    let line =
                        self.get_line_from_offset(with_stmt.range.start().to_usize(), line_index);
                    for item in &with_stmt.items {
                        if let Some(ref optional_vars) = item.optional_vars {
                            let mut temp_names = std::collections::HashSet::new();
                            self.collect_names_from_expr(optional_vars, &mut temp_names);
                            for name in temp_names {
                                local_vars.insert(name, line);
                            }
                        }
                    }
                    self.collect_local_variables(&with_stmt.body, content, line_index, local_vars);
                }
                Stmt::Try(try_stmt) => {
                    self.collect_local_variables(&try_stmt.body, content, line_index, local_vars);
                    // TODO: ExceptHandler struct doesn't expose exception variable name or
                    // body in rustpython-parser 0.4.0. This means we can't collect local
                    // variables from except blocks. Should be revisited if parser is upgraded.
                    self.collect_local_variables(&try_stmt.orelse, content, line_index, local_vars);
                    self.collect_local_variables(
                        &try_stmt.finalbody,
                        content,
                        line_index,
                        local_vars,
                    );
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_names_from_expr(&self, expr: &Expr, names: &mut std::collections::HashSet<String>) {
        match expr {
            Expr::Name(name) => {
                names.insert(name.id.to_string());
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.collect_names_from_expr(elt, names);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.collect_names_from_expr(elt, names);
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_stmt_for_names(
        &self,
        stmt: &Stmt,
        file_path: &PathBuf,
        content: &str,
        line_index: &[usize],
        declared_params: &std::collections::HashSet<String>,
        local_vars: &std::collections::HashMap<String, usize>,
        function_name: &str,
        function_line: usize,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                self.visit_expr_for_names(
                    &expr_stmt.value,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Stmt::Assign(assign) => {
                self.visit_expr_for_names(
                    &assign.value,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Stmt::AugAssign(aug_assign) => {
                self.visit_expr_for_names(
                    &aug_assign.value,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Stmt::Return(ret) => {
                if let Some(ref value) = ret.value {
                    self.visit_expr_for_names(
                        value,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::If(if_stmt) => {
                self.visit_expr_for_names(
                    &if_stmt.test,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for stmt in &if_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
                for stmt in &if_stmt.orelse {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::While(while_stmt) => {
                self.visit_expr_for_names(
                    &while_stmt.test,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for stmt in &while_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::For(for_stmt) => {
                self.visit_expr_for_names(
                    &for_stmt.iter,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for stmt in &for_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    self.visit_expr_for_names(
                        &item.context_expr,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
                for stmt in &with_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::AsyncFor(for_stmt) => {
                self.visit_expr_for_names(
                    &for_stmt.iter,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for stmt in &for_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::AsyncWith(with_stmt) => {
                for item in &with_stmt.items {
                    self.visit_expr_for_names(
                        &item.context_expr,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
                for stmt in &with_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::Assert(assert_stmt) => {
                self.visit_expr_for_names(
                    &assert_stmt.test,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                if let Some(ref msg) = assert_stmt.msg {
                    self.visit_expr_for_names(
                        msg,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            _ => {} // Other statement types
        }
    }

    #[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
    fn visit_expr_for_names(
        &self,
        expr: &Expr,
        file_path: &PathBuf,
        content: &str,
        line_index: &[usize],
        declared_params: &std::collections::HashSet<String>,
        local_vars: &std::collections::HashMap<String, usize>,
        function_name: &str,
        function_line: usize,
    ) {
        match expr {
            Expr::Name(name) => {
                let name_str = name.id.as_str();
                let line = self.get_line_from_offset(name.range.start().to_usize(), line_index);

                // Check if this name is a known fixture and not a declared parameter
                // For local variables, only exclude them if they're defined BEFORE the current line
                // (Python variables are only in scope after they're assigned)
                let is_local_var_in_scope = local_vars
                    .get(name_str)
                    .map(|def_line| *def_line < line)
                    .unwrap_or(false);

                if !declared_params.contains(name_str)
                    && !is_local_var_in_scope
                    && self.is_available_fixture(file_path, name_str)
                {
                    let start_char = self
                        .get_char_position_from_offset(name.range.start().to_usize(), line_index);
                    let end_char =
                        self.get_char_position_from_offset(name.range.end().to_usize(), line_index);

                    info!(
                        "Found undeclared fixture usage: {} at {:?}:{}:{} in function {}",
                        name_str, file_path, line, start_char, function_name
                    );

                    let undeclared = UndeclaredFixture {
                        name: name_str.to_string(),
                        file_path: file_path.clone(),
                        line,
                        start_char,
                        end_char,
                        function_name: function_name.to_string(),
                        function_line,
                    };

                    self.undeclared_fixtures
                        .entry(file_path.clone())
                        .or_default()
                        .push(undeclared);
                }
            }
            Expr::Call(call) => {
                self.visit_expr_for_names(
                    &call.func,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for arg in &call.args {
                    self.visit_expr_for_names(
                        arg,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Expr::Attribute(attr) => {
                self.visit_expr_for_names(
                    &attr.value,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Expr::BinOp(binop) => {
                self.visit_expr_for_names(
                    &binop.left,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                self.visit_expr_for_names(
                    &binop.right,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Expr::UnaryOp(unaryop) => {
                self.visit_expr_for_names(
                    &unaryop.operand,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Expr::Compare(compare) => {
                self.visit_expr_for_names(
                    &compare.left,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for comparator in &compare.comparators {
                    self.visit_expr_for_names(
                        comparator,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Expr::Subscript(subscript) => {
                self.visit_expr_for_names(
                    &subscript.value,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                self.visit_expr_for_names(
                    &subscript.slice,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.visit_expr_for_names(
                        elt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.visit_expr_for_names(
                        elt,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Expr::Dict(dict) => {
                for k in dict.keys.iter().flatten() {
                    self.visit_expr_for_names(
                        k,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
                for value in &dict.values {
                    self.visit_expr_for_names(
                        value,
                        file_path,
                        content,
                        line_index,
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Expr::Await(await_expr) => {
                // Handle await expressions (async functions)
                self.visit_expr_for_names(
                    &await_expr.value,
                    file_path,
                    content,
                    line_index,
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
            }
            _ => {} // Other expression types
        }
    }

    fn is_available_fixture(&self, file_path: &Path, fixture_name: &str) -> bool {
        // Check if this fixture exists and is available at this file location
        if let Some(definitions) = self.definitions.get(fixture_name) {
            // Check if any definition is available from this file location
            for def in definitions.iter() {
                // Fixture is available if it's in the same file or in a conftest.py in a parent directory
                if def.file_path == file_path {
                    return true;
                }

                // Check if it's in a conftest.py in a parent directory
                if def.file_path.file_name().and_then(|n| n.to_str()) == Some("conftest.py")
                    && file_path.starts_with(def.file_path.parent().unwrap_or(Path::new("")))
                {
                    return true;
                }

                // Check if it's in a virtual environment (third-party fixture)
                if def.file_path.to_string_lossy().contains("site-packages") {
                    return true;
                }
            }
        }
        false
    }

    fn extract_docstring(&self, body: &[Stmt]) -> Option<String> {
        // Python docstrings are the first statement in a function if it's an Expr containing a Constant string
        if let Some(Stmt::Expr(expr_stmt)) = body.first() {
            if let Expr::Constant(constant) = &*expr_stmt.value {
                // Check if the constant is a string
                if let rustpython_parser::ast::Constant::Str(s) = &constant.value {
                    return Some(self.format_docstring(s.to_string()));
                }
            }
        }
        None
    }

    fn format_docstring(&self, docstring: String) -> String {
        // Process docstring similar to Python's inspect.cleandoc()
        // 1. Split into lines
        let lines: Vec<&str> = docstring.lines().collect();

        if lines.is_empty() {
            return String::new();
        }

        // 2. Strip leading and trailing empty lines
        let mut start = 0;
        let mut end = lines.len();

        while start < lines.len() && lines[start].trim().is_empty() {
            start += 1;
        }

        while end > start && lines[end - 1].trim().is_empty() {
            end -= 1;
        }

        if start >= end {
            return String::new();
        }

        let lines = &lines[start..end];

        // 3. Find minimum indentation (ignoring first line if it's not empty)
        let mut min_indent = usize::MAX;
        for (i, line) in lines.iter().enumerate() {
            if i == 0 && !line.trim().is_empty() {
                // First line might not be indented, skip it
                continue;
            }

            if !line.trim().is_empty() {
                let indent = line.len() - line.trim_start().len();
                min_indent = min_indent.min(indent);
            }
        }

        if min_indent == usize::MAX {
            min_indent = 0;
        }

        // 4. Remove the common indentation from all lines (except possibly first)
        let mut result = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                // First line: just trim it
                result.push(line.trim().to_string());
            } else if line.trim().is_empty() {
                // Empty line: keep it empty
                result.push(String::new());
            } else {
                // Remove common indentation
                let dedented = if line.len() > min_indent {
                    &line[min_indent..]
                } else {
                    line.trim_start()
                };
                result.push(dedented.to_string());
            }
        }

        // 5. Join lines back together
        result.join("\n")
    }

    fn extract_return_type(
        &self,
        returns: &Option<Box<rustpython_parser::ast::Expr>>,
        body: &[Stmt],
        content: &str,
    ) -> Option<String> {
        if let Some(return_expr) = returns {
            // Check if the function body contains yield statements
            let has_yield = self.contains_yield(body);

            if has_yield {
                // For generators, extract the yielded type from Generator[YieldType, ...]
                // or Iterator[YieldType] or similar
                return self.extract_yielded_type(return_expr, content);
            } else {
                // For regular functions, just return the type annotation as-is
                return Some(self.expr_to_string(return_expr, content));
            }
        }
        None
    }

    #[allow(clippy::only_used_in_recursion)]
    fn contains_yield(&self, body: &[Stmt]) -> bool {
        for stmt in body {
            match stmt {
                Stmt::Expr(expr_stmt) => {
                    if let Expr::Yield(_) | Expr::YieldFrom(_) = &*expr_stmt.value {
                        return true;
                    }
                }
                Stmt::If(if_stmt) => {
                    if self.contains_yield(&if_stmt.body) || self.contains_yield(&if_stmt.orelse) {
                        return true;
                    }
                }
                Stmt::For(for_stmt) => {
                    if self.contains_yield(&for_stmt.body) || self.contains_yield(&for_stmt.orelse)
                    {
                        return true;
                    }
                }
                Stmt::While(while_stmt) => {
                    if self.contains_yield(&while_stmt.body)
                        || self.contains_yield(&while_stmt.orelse)
                    {
                        return true;
                    }
                }
                Stmt::With(with_stmt) => {
                    if self.contains_yield(&with_stmt.body) {
                        return true;
                    }
                }
                Stmt::Try(try_stmt) => {
                    if self.contains_yield(&try_stmt.body)
                        || self.contains_yield(&try_stmt.orelse)
                        || self.contains_yield(&try_stmt.finalbody)
                    {
                        return true;
                    }
                    // TODO: ExceptHandler struct doesn't expose body in rustpython-parser 0.4.0.
                    // Should be revisited if parser is upgraded.
                }
                _ => {}
            }
        }
        false
    }

    fn extract_yielded_type(
        &self,
        expr: &rustpython_parser::ast::Expr,
        content: &str,
    ) -> Option<String> {
        // Handle Generator[YieldType, SendType, ReturnType] -> extract YieldType
        // Handle Iterator[YieldType] -> extract YieldType
        // Handle Iterable[YieldType] -> extract YieldType
        if let Expr::Subscript(subscript) = expr {
            // Get the base type name (Generator, Iterator, etc.)
            let _base_name = self.expr_to_string(&subscript.value, content);

            // Extract the first type argument (the yield type)
            if let Expr::Tuple(tuple) = &*subscript.slice {
                if let Some(first_elem) = tuple.elts.first() {
                    return Some(self.expr_to_string(first_elem, content));
                }
            } else {
                // Single type argument (like Iterator[str])
                return Some(self.expr_to_string(&subscript.slice, content));
            }
        }

        // If we can't extract the yielded type, return the whole annotation
        Some(self.expr_to_string(expr, content))
    }

    #[allow(clippy::only_used_in_recursion)]
    fn expr_to_string(&self, expr: &rustpython_parser::ast::Expr, content: &str) -> String {
        match expr {
            Expr::Name(name) => name.id.to_string(),
            Expr::Attribute(attr) => {
                format!(
                    "{}.{}",
                    self.expr_to_string(&attr.value, content),
                    attr.attr
                )
            }
            Expr::Subscript(subscript) => {
                let base = self.expr_to_string(&subscript.value, content);
                let slice = self.expr_to_string(&subscript.slice, content);
                format!("{}[{}]", base, slice)
            }
            Expr::Tuple(tuple) => {
                let elements: Vec<String> = tuple
                    .elts
                    .iter()
                    .map(|e| self.expr_to_string(e, content))
                    .collect();
                elements.join(", ")
            }
            Expr::Constant(constant) => {
                format!("{:?}", constant.value)
            }
            Expr::BinOp(binop) if matches!(binop.op, rustpython_parser::ast::Operator::BitOr) => {
                // Handle union types like str | int
                format!(
                    "{} | {}",
                    self.expr_to_string(&binop.left, content),
                    self.expr_to_string(&binop.right, content)
                )
            }
            _ => {
                // Fallback for complex types we don't handle yet
                "Any".to_string()
            }
        }
    }

    fn build_line_index(content: &str) -> Vec<usize> {
        let mut line_index = Vec::with_capacity(content.len() / 30);
        line_index.push(0);
        for (i, c) in content.char_indices() {
            if c == '\n' {
                line_index.push(i + 1);
            }
        }
        line_index
    }

    fn get_line_from_offset(&self, offset: usize, line_index: &[usize]) -> usize {
        match line_index.binary_search(&offset) {
            Ok(line) => line + 1,
            Err(line) => line,
        }
    }

    fn get_char_position_from_offset(&self, offset: usize, line_index: &[usize]) -> usize {
        let line = self.get_line_from_offset(offset, line_index);
        let line_start = line_index[line - 1];
        offset.saturating_sub(line_start)
    }

    /// Find the character position of a function name in a line
    /// Handles both "def func_name(" and "async def func_name(" patterns
    /// Returns (start_char, end_char) tuple
    fn find_function_name_position(
        &self,
        content: &str,
        line: usize,
        func_name: &str,
    ) -> (usize, usize) {
        // Get the line content
        if let Some(line_content) = content.lines().nth(line.saturating_sub(1)) {
            // Look for the function name after "def " or "async def "
            // The pattern should be: (async )?def func_name(
            if let Some(def_pos) = line_content.find("def ") {
                let search_start = def_pos + 4; // Skip "def "
                if let Some(name_start) = line_content[search_start..].find(func_name) {
                    let start_char = search_start + name_start;
                    let end_char = start_char + func_name.len();
                    return (start_char, end_char);
                }
            }
        }

        // Fallback: return 0, func_name.len() if we can't find it
        (0, func_name.len())
    }

    /// Find fixture definition for a given position in a file
    pub fn find_fixture_definition(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<FixtureDefinition> {
        debug!(
            "find_fixture_definition: file={:?}, line={}, char={}",
            file_path, line, character
        );

        let target_line = (line + 1) as usize; // Convert from 0-based to 1-based

        // Read the file content - try cache first, then file system
        // Use Arc to avoid cloning large strings - just increments ref count
        let content = self.get_file_content(file_path)?;

        // Avoid allocating Vec - access line directly via iterator
        let line_content = content.lines().nth(target_line.saturating_sub(1))?;
        debug!("Line content: {}", line_content);

        // Extract the word at the character position
        let word_at_cursor = self.extract_word_at_position(line_content, character as usize)?;
        debug!("Word at cursor: {:?}", word_at_cursor);

        // Check if we're inside a fixture definition with the same name (self-referencing)
        // In that case, we should skip the current definition and find the parent
        let current_fixture_def = self.get_fixture_definition_at_line(file_path, target_line);

        // First, check if this word matches any fixture usage on this line
        // AND that the cursor is within the character range of that usage
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line && usage.name == word_at_cursor {
                    // Check if cursor is within the character range of this usage
                    let cursor_pos = character as usize;
                    if cursor_pos >= usage.start_char && cursor_pos < usage.end_char {
                        debug!(
                            "Cursor at {} is within usage range {}-{}: {}",
                            cursor_pos, usage.start_char, usage.end_char, usage.name
                        );
                        info!("Found fixture usage at cursor position: {}", usage.name);

                        // If we're in a fixture definition with the same name, skip it when searching
                        if let Some(ref current_def) = current_fixture_def {
                            if current_def.name == word_at_cursor {
                                info!(
                                    "Self-referencing fixture detected, finding parent definition"
                                );
                                return self.find_closest_definition_excluding(
                                    file_path,
                                    &usage.name,
                                    Some(current_def),
                                );
                            }
                        }

                        // Find the closest definition for this fixture
                        return self.find_closest_definition(file_path, &usage.name);
                    }
                }
            }
        }

        debug!("Word at cursor '{}' is not a fixture usage", word_at_cursor);
        None
    }

    /// Get the fixture definition at a specific line (if the line is a fixture definition)
    fn get_fixture_definition_at_line(
        &self,
        file_path: &Path,
        line: usize,
    ) -> Option<FixtureDefinition> {
        for entry in self.definitions.iter() {
            for def in entry.value().iter() {
                if def.file_path == file_path && def.line == line {
                    return Some(def.clone());
                }
            }
        }
        None
    }

    /// Public method to get the fixture definition at a specific line and name
    /// Used when cursor is on a fixture definition line (not a usage)
    pub fn get_definition_at_line(
        &self,
        file_path: &Path,
        line: usize,
        fixture_name: &str,
    ) -> Option<FixtureDefinition> {
        if let Some(definitions) = self.definitions.get(fixture_name) {
            for def in definitions.iter() {
                if def.file_path == file_path && def.line == line {
                    return Some(def.clone());
                }
            }
        }
        None
    }

    fn find_closest_definition(
        &self,
        file_path: &Path,
        fixture_name: &str,
    ) -> Option<FixtureDefinition> {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Check if fixture is defined in the same file (highest priority)
        // If multiple definitions exist in the same file, return the last one (pytest semantics)
        debug!(
            "Checking for fixture {} in same file: {:?}",
            fixture_name, file_path
        );

        // Use iterator directly without collecting to Vec - more efficient
        if let Some(last_def) = definitions
            .iter()
            .filter(|def| def.file_path == file_path)
            .max_by_key(|def| def.line)
        {
            info!(
                "Found fixture {} in same file at line {} (using last definition)",
                fixture_name, last_def.line
            );
            return Some(last_def.clone());
        }

        // Priority 2: Search upward through conftest.py files in parent directories
        // Start from the current file's directory and search upward
        let mut current_dir = file_path.parent()?;

        debug!(
            "Searching for fixture {} in conftest.py files starting from {:?}",
            fixture_name, current_dir
        );
        loop {
            // Check for conftest.py in current directory
            let conftest_path = current_dir.join("conftest.py");
            debug!("  Checking conftest.py at: {:?}", conftest_path);

            for def in definitions.iter() {
                if def.file_path == conftest_path {
                    info!(
                        "Found fixture {} in conftest.py: {:?}",
                        fixture_name, conftest_path
                    );
                    return Some(def.clone());
                }
            }

            // Move up one directory
            match current_dir.parent() {
                Some(parent) => current_dir = parent,
                None => break,
            }
        }

        // Priority 3: Check for third-party fixtures (from virtual environment)
        // These are fixtures from pytest plugins in site-packages
        debug!(
            "No fixture {} found in conftest hierarchy, checking for third-party fixtures",
            fixture_name
        );
        for def in definitions.iter() {
            if def.file_path.to_string_lossy().contains("site-packages") {
                info!(
                    "Found third-party fixture {} in site-packages: {:?}",
                    fixture_name, def.file_path
                );
                return Some(def.clone());
            }
        }

        // No fixture found in scope - this is intentional, not a fallback.
        // A fixture must be in: same file, conftest.py hierarchy, or site-packages
        // to be accessible from the requesting file.
        debug!(
            "No fixture {} found in scope for {:?} (same file, conftest hierarchy, or third-party)",
            fixture_name, file_path
        );
        None
    }

    /// Find the closest definition for a fixture, excluding a specific definition
    /// This is useful for self-referencing fixtures where we need to find the parent definition
    fn find_closest_definition_excluding(
        &self,
        file_path: &Path,
        fixture_name: &str,
        exclude: Option<&FixtureDefinition>,
    ) -> Option<FixtureDefinition> {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Check if fixture is defined in the same file (highest priority)
        // but skip the excluded definition
        // If multiple definitions exist, use the last one (pytest semantics)
        debug!(
            "Checking for fixture {} in same file: {:?} (excluding: {:?})",
            fixture_name, file_path, exclude
        );

        // Use iterator directly without collecting to Vec - more efficient
        if let Some(last_def) = definitions
            .iter()
            .filter(|def| {
                if def.file_path != file_path {
                    return false;
                }
                // Skip the excluded definition
                if let Some(excluded) = exclude {
                    if def == &excluded {
                        debug!("Skipping excluded definition at line {}", def.line);
                        return false;
                    }
                }
                true
            })
            .max_by_key(|def| def.line)
        {
            info!(
                "Found fixture {} in same file at line {} (using last definition, excluding specified)",
                fixture_name, last_def.line
            );
            return Some(last_def.clone());
        }

        // Priority 2: Search upward through conftest.py files in parent directories
        let mut current_dir = file_path.parent()?;

        debug!(
            "Searching for fixture {} in conftest.py files starting from {:?}",
            fixture_name, current_dir
        );
        loop {
            let conftest_path = current_dir.join("conftest.py");
            debug!("  Checking conftest.py at: {:?}", conftest_path);

            for def in definitions.iter() {
                if def.file_path == conftest_path {
                    // Skip the excluded definition (though it's unlikely to be in a different file)
                    if let Some(excluded) = exclude {
                        if def == excluded {
                            debug!("Skipping excluded definition at line {}", def.line);
                            continue;
                        }
                    }
                    info!(
                        "Found fixture {} in conftest.py: {:?}",
                        fixture_name, conftest_path
                    );
                    return Some(def.clone());
                }
            }

            // Move up one directory
            match current_dir.parent() {
                Some(parent) => current_dir = parent,
                None => break,
            }
        }

        // Priority 3: Check for third-party fixtures (from virtual environment)
        debug!(
            "No fixture {} found in conftest hierarchy (excluding specified), checking for third-party fixtures",
            fixture_name
        );
        for def in definitions.iter() {
            // Skip excluded definition
            if let Some(excluded) = exclude {
                if def == excluded {
                    continue;
                }
            }
            if def.file_path.to_string_lossy().contains("site-packages") {
                info!(
                    "Found third-party fixture {} in site-packages: {:?}",
                    fixture_name, def.file_path
                );
                return Some(def.clone());
            }
        }

        // No fixture found in scope - this is intentional, not a fallback.
        // A fixture must be in: same file, conftest.py hierarchy, or site-packages
        // to be accessible from the requesting file.
        debug!(
            "No fixture {} found in scope for {:?} (excluding specified definition)",
            fixture_name, file_path
        );
        None
    }

    /// Find the fixture name at a given position (either definition or usage)
    pub fn find_fixture_at_position(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<String> {
        let target_line = (line + 1) as usize; // Convert from 0-based to 1-based

        debug!(
            "find_fixture_at_position: file={:?}, line={}, char={}",
            file_path, target_line, character
        );

        // Read the file content - try cache first, then file system
        // Use Arc to avoid cloning large strings - just increments ref count
        let content = self.get_file_content(file_path)?;

        // Avoid allocating Vec - access line directly via iterator
        let line_content = content.lines().nth(target_line.saturating_sub(1))?;
        debug!("Line content: {}", line_content);

        // Extract the word at the character position
        let word_at_cursor = self.extract_word_at_position(line_content, character as usize);
        debug!("Word at cursor: {:?}", word_at_cursor);

        // Check if this word matches any fixture usage on this line
        // AND that the cursor is within the character range of that usage
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line {
                    // Check if cursor is within the character range of this usage
                    let cursor_pos = character as usize;
                    if cursor_pos >= usage.start_char && cursor_pos < usage.end_char {
                        debug!(
                            "Cursor at {} is within usage range {}-{}: {}",
                            cursor_pos, usage.start_char, usage.end_char, usage.name
                        );
                        info!("Found fixture usage at cursor position: {}", usage.name);
                        return Some(usage.name.clone());
                    }
                }
            }
        }

        // If no usage matched, check if we're on a fixture definition line
        // (but only if the cursor is NOT on a parameter name)
        for entry in self.definitions.iter() {
            for def in entry.value().iter() {
                if def.file_path == file_path && def.line == target_line {
                    // Check if the cursor is on the function name itself, not a parameter
                    if let Some(ref word) = word_at_cursor {
                        if word == &def.name {
                            info!(
                                "Found fixture definition name at cursor position: {}",
                                def.name
                            );
                            return Some(def.name.clone());
                        }
                    }
                    // If cursor is elsewhere on the definition line, don't return the fixture name
                    // unless it matches a parameter (which would be a usage)
                }
            }
        }

        debug!("No fixture found at cursor position");
        None
    }

    pub fn extract_word_at_position(&self, line: &str, character: usize) -> Option<String> {
        // Use char_indices to avoid Vec allocation - more efficient for hot path
        let char_indices: Vec<(usize, char)> = line.char_indices().collect();

        // If cursor is beyond the line, return None
        if character >= char_indices.len() {
            return None;
        }

        // Get the character at the cursor position
        let (_byte_pos, c) = char_indices[character];

        // Check if cursor is ON an identifier character
        if c.is_alphanumeric() || c == '_' {
            // Find start of word (scan backwards)
            let mut start_idx = character;
            while start_idx > 0 {
                let (_, prev_c) = char_indices[start_idx - 1];
                if !prev_c.is_alphanumeric() && prev_c != '_' {
                    break;
                }
                start_idx -= 1;
            }

            // Find end of word (scan forwards)
            let mut end_idx = character + 1;
            while end_idx < char_indices.len() {
                let (_, curr_c) = char_indices[end_idx];
                if !curr_c.is_alphanumeric() && curr_c != '_' {
                    break;
                }
                end_idx += 1;
            }

            // Extract substring using byte positions
            let start_byte = char_indices[start_idx].0;
            let end_byte = if end_idx < char_indices.len() {
                char_indices[end_idx].0
            } else {
                line.len()
            };

            return Some(line[start_byte..end_byte].to_string());
        }

        None
    }

    /// Find all references (usages) of a fixture by name
    pub fn find_fixture_references(&self, fixture_name: &str) -> Vec<FixtureUsage> {
        info!("Finding all references for fixture: {}", fixture_name);

        let mut all_references = Vec::new();

        // Iterate through all files that have usages
        for entry in self.usages.iter() {
            let file_path = entry.key();
            let usages = entry.value();

            // Find all usages of this fixture in this file
            for usage in usages.iter() {
                if usage.name == fixture_name {
                    debug!(
                        "Found reference to {} in {:?} at line {}",
                        fixture_name, file_path, usage.line
                    );
                    all_references.push(usage.clone());
                }
            }
        }

        info!(
            "Found {} total references for fixture: {}",
            all_references.len(),
            fixture_name
        );
        all_references
    }

    /// Find all references (usages) that would resolve to a specific fixture definition
    /// This respects the priority rules: same file > closest conftest.py > parent conftest.py
    ///
    /// For fixture overriding, this handles self-referencing parameters correctly:
    /// If a fixture parameter appears on the same line as a fixture definition with the same name,
    /// we exclude that definition when resolving, so it finds the parent instead.
    pub fn find_references_for_definition(
        &self,
        definition: &FixtureDefinition,
    ) -> Vec<FixtureUsage> {
        info!(
            "Finding references for specific definition: {} at {:?}:{}",
            definition.name, definition.file_path, definition.line
        );

        let mut matching_references = Vec::new();

        // Get all usages of this fixture name
        for entry in self.usages.iter() {
            let file_path = entry.key();
            let usages = entry.value();

            for usage in usages.iter() {
                if usage.name == definition.name {
                    // Check if this usage is on the same line as a fixture definition with the same name
                    // (i.e., a self-referencing fixture parameter like "def foo(foo):")
                    let fixture_def_at_line =
                        self.get_fixture_definition_at_line(file_path, usage.line);

                    let resolved_def = if let Some(ref current_def) = fixture_def_at_line {
                        if current_def.name == usage.name {
                            // Self-referencing parameter - exclude current definition and find parent
                            debug!(
                                "Usage at {:?}:{} is self-referencing, excluding definition at line {}",
                                file_path, usage.line, current_def.line
                            );
                            self.find_closest_definition_excluding(
                                file_path,
                                &usage.name,
                                Some(current_def),
                            )
                        } else {
                            // Different fixture - use normal resolution
                            self.find_closest_definition(file_path, &usage.name)
                        }
                    } else {
                        // Not on a fixture definition line - use normal resolution
                        self.find_closest_definition(file_path, &usage.name)
                    };

                    if let Some(resolved_def) = resolved_def {
                        if resolved_def == *definition {
                            debug!(
                                "Usage at {:?}:{} resolves to our definition",
                                file_path, usage.line
                            );
                            matching_references.push(usage.clone());
                        } else {
                            debug!(
                                "Usage at {:?}:{} resolves to different definition at {:?}:{}",
                                file_path, usage.line, resolved_def.file_path, resolved_def.line
                            );
                        }
                    }
                }
            }
        }

        info!(
            "Found {} references that resolve to this specific definition",
            matching_references.len()
        );
        matching_references
    }

    /// List of built-in pytest fixtures that cannot be renamed
    const BUILTIN_FIXTURES: &'static [&'static str] = &[
        "request",
        "tmp_path",
        "tmp_path_factory",
        "tmpdir",
        "tmpdir_factory",
        "cache",
        "capsys",
        "capsysbinary",
        "capfd",
        "capfdbinary",
        "caplog",
        "doctest_namespace",
        "pytestconfig",
        "record_property",
        "record_testsuite_property",
        "record_xml_attribute",
        "recwarn",
        "monkeypatch",
        "pytester",
        "testdir",
    ];

    /// Check if a fixture name is a built-in pytest fixture
    pub fn is_builtin_fixture(name: &str) -> bool {
        Self::BUILTIN_FIXTURES.contains(&name)
    }

    /// Check if a fixture definition is from a third-party package (site-packages)
    pub fn is_third_party_fixture(definition: &FixtureDefinition) -> bool {
        definition
            .file_path
            .to_string_lossy()
            .contains("site-packages")
    }

    /// Validate that a name is a valid Python identifier
    pub fn is_valid_python_identifier(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }

        // Python identifier rules:
        // - First character must be a letter or underscore
        // - Remaining characters can be letters, digits, or underscores
        // - Cannot be a Python keyword
        let mut chars = name.chars();
        let first = chars.next().unwrap();
        if !first.is_alphabetic() && first != '_' {
            return false;
        }
        for c in chars {
            if !c.is_alphanumeric() && c != '_' {
                return false;
            }
        }

        // Check against Python keywords
        const PYTHON_KEYWORDS: &[&str] = &[
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield",
        ];
        !PYTHON_KEYWORDS.contains(&name)
    }

    /// Collect all locations where a fixture needs to be renamed
    /// This includes the definition and all usages that resolve to it
    ///
    /// Returns an error if:
    /// - No fixture is found at the cursor position
    /// - The fixture is a built-in fixture (request, tmp_path, etc.)
    /// - The fixture is from a third-party package (site-packages)
    pub fn collect_rename_locations(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<RenameInfo, RenameError> {
        info!(
            "collect_rename_locations: file={:?}, line={}, char={}",
            file_path, line, character
        );

        let target_line = (line + 1) as usize; // Convert from 0-based to 1-based

        // First, find the fixture at the cursor position
        let fixture_name = self
            .find_fixture_at_position(file_path, line, character)
            .ok_or(RenameError::NoFixtureAtPosition)?;

        info!("Found fixture at cursor: {}", fixture_name);

        // Check if it's a built-in fixture
        if Self::is_builtin_fixture(&fixture_name) {
            return Err(RenameError::CannotRenameBuiltin(fixture_name));
        }

        // Find the definition for this fixture
        let definition = self
            .find_fixture_definition(file_path, line, character)
            .or_else(|| {
                // If cursor is on a definition line itself, get that definition
                self.get_definition_at_line(file_path, target_line, &fixture_name)
            })
            .ok_or(RenameError::NoFixtureAtPosition)?;

        info!(
            "Found definition: {} at {:?}:{}",
            definition.name, definition.file_path, definition.line
        );

        // Check if it's a third-party fixture
        if Self::is_third_party_fixture(&definition) {
            return Err(RenameError::CannotRenameThirdParty(fixture_name));
        }

        // Collect all locations
        let mut locations = Vec::new();

        // Add the definition location
        locations.push(RenameLocation {
            file_path: definition.file_path.clone(),
            line: definition.line,
            start_char: definition.start_char,
            end_char: definition.end_char,
        });

        // Add all usage locations that resolve to this definition
        let references = self.find_references_for_definition(&definition);
        for usage in references {
            // Skip usages on the same line as the definition (already added above)
            if usage.file_path == definition.file_path && usage.line == definition.line {
                continue;
            }
            locations.push(RenameLocation {
                file_path: usage.file_path.clone(),
                line: usage.line,
                start_char: usage.start_char,
                end_char: usage.end_char,
            });
        }

        info!(
            "Collected {} rename locations for fixture {}",
            locations.len(),
            fixture_name
        );

        Ok(RenameInfo {
            definition,
            locations,
        })
    }

    /// Get the current fixture name and its range at a cursor position
    /// Used for prepare_rename to show the current name and range to the user
    pub fn get_fixture_range_at_position(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<(String, usize, usize, usize)> {
        let target_line = (line + 1) as usize; // Convert from 0-based to 1-based

        // Check if cursor is on a fixture usage
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line {
                    let cursor_pos = character as usize;
                    if cursor_pos >= usage.start_char && cursor_pos < usage.end_char {
                        return Some((
                            usage.name.clone(),
                            usage.line,
                            usage.start_char,
                            usage.end_char,
                        ));
                    }
                }
            }
        }

        // Check if cursor is on a fixture definition
        for entry in self.definitions.iter() {
            for def in entry.value().iter() {
                if def.file_path == file_path && def.line == target_line {
                    let cursor_pos = character as usize;
                    if cursor_pos >= def.start_char && cursor_pos < def.end_char {
                        return Some((def.name.clone(), def.line, def.start_char, def.end_char));
                    }
                }
            }
        }

        None
    }

    /// Get all undeclared fixture usages for a file
    pub fn get_undeclared_fixtures(&self, file_path: &Path) -> Vec<UndeclaredFixture> {
        self.undeclared_fixtures
            .get(file_path)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }

    /// Get all available fixtures for a given file, respecting pytest's fixture hierarchy
    /// Returns a list of fixture definitions sorted by name
    pub fn get_available_fixtures(&self, file_path: &Path) -> Vec<FixtureDefinition> {
        let mut available_fixtures = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        // Priority 1: Fixtures in the same file
        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            for def in entry.value().iter() {
                if def.file_path == file_path && !seen_names.contains(fixture_name.as_str()) {
                    available_fixtures.push(def.clone());
                    seen_names.insert(fixture_name.clone());
                }
            }
        }

        // Priority 2: Fixtures in conftest.py files (walking up the directory tree)
        if let Some(mut current_dir) = file_path.parent() {
            loop {
                let conftest_path = current_dir.join("conftest.py");

                for entry in self.definitions.iter() {
                    let fixture_name = entry.key();
                    for def in entry.value().iter() {
                        if def.file_path == conftest_path
                            && !seen_names.contains(fixture_name.as_str())
                        {
                            available_fixtures.push(def.clone());
                            seen_names.insert(fixture_name.clone());
                        }
                    }
                }

                // Move up one directory
                match current_dir.parent() {
                    Some(parent) => current_dir = parent,
                    None => break,
                }
            }
        }

        // Priority 3: Third-party fixtures from site-packages
        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            for def in entry.value().iter() {
                if def.file_path.to_string_lossy().contains("site-packages")
                    && !seen_names.contains(fixture_name.as_str())
                {
                    available_fixtures.push(def.clone());
                    seen_names.insert(fixture_name.clone());
                }
            }
        }

        // Sort by name for consistent ordering
        available_fixtures.sort_by(|a, b| a.name.cmp(&b.name));
        available_fixtures
    }

    /// Check if a position is inside a test or fixture function (parameter or body)
    /// Returns Some((function_name, is_fixture, declared_params)) if inside a function
    pub fn is_inside_function(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<(String, bool, Vec<String>)> {
        // Try cache first, then file system
        let content = self.get_file_content(file_path)?;

        let target_line = (line + 1) as usize; // Convert to 1-based

        // Parse the file
        let parsed = parse(&content, Mode::Module, "").ok()?;

        if let rustpython_parser::ast::Mod::Module(module) = parsed {
            return self.find_enclosing_function(
                &module.body,
                &content,
                target_line,
                character as usize,
            );
        }

        None
    }

    fn find_enclosing_function(
        &self,
        stmts: &[Stmt],
        content: &str,
        target_line: usize,
        _target_char: usize,
    ) -> Option<(String, bool, Vec<String>)> {
        for stmt in stmts {
            match stmt {
                Stmt::FunctionDef(func_def) => {
                    let func_start_line = content[..func_def.range.start().to_usize()]
                        .matches('\n')
                        .count()
                        + 1;
                    let func_end_line = content[..func_def.range.end().to_usize()]
                        .matches('\n')
                        .count()
                        + 1;

                    // Check if target is within this function's range
                    if target_line >= func_start_line && target_line <= func_end_line {
                        let is_fixture = func_def
                            .decorator_list
                            .iter()
                            .any(Self::is_fixture_decorator);
                        let is_test = func_def.name.starts_with("test_");

                        // Only return if it's a test or fixture
                        if is_test || is_fixture {
                            let params: Vec<String> = func_def
                                .args
                                .args
                                .iter()
                                .map(|arg| arg.def.arg.to_string())
                                .collect();

                            return Some((func_def.name.to_string(), is_fixture, params));
                        }
                    }
                }
                Stmt::AsyncFunctionDef(func_def) => {
                    let func_start_line = content[..func_def.range.start().to_usize()]
                        .matches('\n')
                        .count()
                        + 1;
                    let func_end_line = content[..func_def.range.end().to_usize()]
                        .matches('\n')
                        .count()
                        + 1;

                    if target_line >= func_start_line && target_line <= func_end_line {
                        let is_fixture = func_def
                            .decorator_list
                            .iter()
                            .any(Self::is_fixture_decorator);
                        let is_test = func_def.name.starts_with("test_");

                        if is_test || is_fixture {
                            let params: Vec<String> = func_def
                                .args
                                .args
                                .iter()
                                .map(|arg| arg.def.arg.to_string())
                                .collect();

                            return Some((func_def.name.to_string(), is_fixture, params));
                        }
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Get the completion context for a given position
    /// This determines what kind of completions should be offered
    pub fn get_completion_context(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<CompletionContext> {
        let content = self.get_file_content(file_path)?;
        let target_line = (line + 1) as usize; // Convert to 1-based

        // Parse the file
        let parsed = parse(&content, Mode::Module, "").ok()?;

        if let rustpython_parser::ast::Mod::Module(module) = parsed {
            // First check if we're inside a decorator (usefixtures or parametrize)
            if let Some(ctx) = Self::check_decorator_context(&module.body, &content, target_line) {
                return Some(ctx);
            }

            // Then check for function context
            return self.get_function_completion_context(
                &module.body,
                &content,
                target_line,
                character as usize,
            );
        }

        None
    }

    /// Check if the cursor is inside a decorator that needs fixture completions
    fn check_decorator_context(
        stmts: &[Stmt],
        content: &str,
        target_line: usize,
    ) -> Option<CompletionContext> {
        for stmt in stmts {
            let decorator_list = match stmt {
                Stmt::FunctionDef(f) => &f.decorator_list,
                Stmt::AsyncFunctionDef(f) => &f.decorator_list,
                Stmt::ClassDef(c) => &c.decorator_list,
                _ => continue,
            };

            for decorator in decorator_list {
                let dec_start_line = content[..decorator.range().start().to_usize()]
                    .matches('\n')
                    .count()
                    + 1;
                let dec_end_line = content[..decorator.range().end().to_usize()]
                    .matches('\n')
                    .count()
                    + 1;

                if target_line >= dec_start_line && target_line <= dec_end_line {
                    // Check if it's a usefixtures decorator
                    if Self::is_usefixtures_decorator(decorator) {
                        return Some(CompletionContext::UsefixuturesDecorator);
                    }
                    // Check if it's a parametrize decorator with indirect
                    if Self::is_parametrize_decorator(decorator) {
                        return Some(CompletionContext::ParametrizeIndirect);
                    }
                }
            }

            // Recursively check class bodies
            if let Stmt::ClassDef(class_def) = stmt {
                if let Some(ctx) =
                    Self::check_decorator_context(&class_def.body, content, target_line)
                {
                    return Some(ctx);
                }
            }
        }

        None
    }

    /// Get completion context when cursor is inside a function
    fn get_function_completion_context(
        &self,
        stmts: &[Stmt],
        content: &str,
        target_line: usize,
        target_char: usize,
    ) -> Option<CompletionContext> {
        for stmt in stmts {
            match stmt {
                Stmt::FunctionDef(func_def) => {
                    if let Some(ctx) = self.get_func_context(
                        &func_def.name,
                        &func_def.decorator_list,
                        &func_def.args,
                        func_def.range,
                        content,
                        target_line,
                        target_char,
                    ) {
                        return Some(ctx);
                    }
                }
                Stmt::AsyncFunctionDef(func_def) => {
                    if let Some(ctx) = self.get_func_context(
                        &func_def.name,
                        &func_def.decorator_list,
                        &func_def.args,
                        func_def.range,
                        content,
                        target_line,
                        target_char,
                    ) {
                        return Some(ctx);
                    }
                }
                Stmt::ClassDef(class_def) => {
                    // Recursively check class body
                    if let Some(ctx) = self.get_function_completion_context(
                        &class_def.body,
                        content,
                        target_line,
                        target_char,
                    ) {
                        return Some(ctx);
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Helper to get function completion context
    #[allow(clippy::too_many_arguments)]
    fn get_func_context(
        &self,
        func_name: &rustpython_parser::ast::Identifier,
        decorator_list: &[Expr],
        args: &Arguments,
        range: rustpython_parser::text_size::TextRange,
        content: &str,
        target_line: usize,
        _target_char: usize,
    ) -> Option<CompletionContext> {
        let func_start_line = content[..range.start().to_usize()].matches('\n').count() + 1;
        let func_end_line = content[..range.end().to_usize()].matches('\n').count() + 1;

        if target_line < func_start_line || target_line > func_end_line {
            return None;
        }

        let is_fixture = decorator_list.iter().any(Self::is_fixture_decorator);
        let is_test = func_name.as_str().starts_with("test_");

        if !is_test && !is_fixture {
            return None;
        }

        // Collect all parameters including positional-only and keyword-only
        let params: Vec<String> = Self::all_args(args)
            .map(|arg| arg.def.arg.to_string())
            .collect();

        // Find the line where the function signature ends (closing parenthesis + colon)
        // We need to find "def func_name(...):""
        let lines: Vec<&str> = content.lines().collect();

        // Find the signature end line by looking for "):" pattern
        let mut sig_end_line = func_start_line;
        for (i, line) in lines
            .iter()
            .enumerate()
            .skip(func_start_line.saturating_sub(1))
        {
            if line.contains("):") {
                sig_end_line = i + 1; // 1-indexed
                break;
            }
            // Stop if we've gone too far
            if i + 1 > func_start_line + 10 {
                break;
            }
        }

        // Determine if cursor is in signature or body
        let in_signature = target_line <= sig_end_line;

        let context = if in_signature {
            CompletionContext::FunctionSignature {
                function_name: func_name.to_string(),
                function_line: func_start_line,
                is_fixture,
                declared_params: params,
            }
        } else {
            CompletionContext::FunctionBody {
                function_name: func_name.to_string(),
                function_line: func_start_line,
                is_fixture,
                declared_params: params,
            }
        };

        Some(context)
    }

    /// Get information about where to insert a new parameter in a function signature
    pub fn get_function_param_insertion_info(
        &self,
        file_path: &Path,
        function_line: usize,
    ) -> Option<ParamInsertionInfo> {
        let content = self.get_file_content(file_path)?;
        let lines: Vec<&str> = content.lines().collect();

        // Find the closing parenthesis of the function signature
        // Start from function_line (1-indexed) and look for "):"
        for i in (function_line.saturating_sub(1))..lines.len().min(function_line + 10) {
            let line = lines[i];
            if let Some(paren_pos) = line.find("):") {
                // Check if there are existing parameters
                // Look for the opening parenthesis
                let open_paren = if let Some(pos) = line.find('(') {
                    pos
                } else {
                    // Opening paren might be on an earlier line for multiline signatures
                    // For simplicity, we'll look at the function_line
                    let func_line = lines.get(function_line.saturating_sub(1))?;
                    func_line.find('(')?
                };

                // Check if there's content between ( and )
                let params_section = &line[open_paren + 1..paren_pos];
                let has_params = !params_section.trim().is_empty();

                return Some(ParamInsertionInfo {
                    line: i + 1, // Convert to 1-indexed
                    char_pos: paren_pos,
                    needs_comma: has_params,
                });
            }
        }

        None
    }

    /// Compute usage counts for all fixture definitions efficiently.
    /// This iterates all usages once and resolves each to its definition,
    /// rather than iterating all usages for each definition.
    fn compute_definition_usage_counts(&self) -> HashMap<(PathBuf, String), usize> {
        let mut counts: HashMap<(PathBuf, String), usize> = HashMap::new();

        // Initialize all definitions with 0 count
        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            for def in entry.value().iter() {
                counts.insert((def.file_path.clone(), fixture_name.clone()), 0);
            }
        }

        // Cache for resolved definitions: (usage_file, fixture_name) -> resolved_def_file
        // This avoids re-resolving the same fixture for multiple usages in the same file.
        // Note: Self-referencing fixtures bypass this cache since they need special handling.
        let mut resolution_cache: HashMap<(PathBuf, String), Option<PathBuf>> = HashMap::new();

        // Pre-compute fixture definition lines per file for fast lookup
        // This avoids calling get_fixture_definition_at_line for each usage
        let mut fixture_def_lines: HashMap<PathBuf, HashMap<usize, FixtureDefinition>> =
            HashMap::new();
        for entry in self.definitions.iter() {
            for def in entry.value().iter() {
                fixture_def_lines
                    .entry(def.file_path.clone())
                    .or_default()
                    .insert(def.line, def.clone());
            }
        }

        // Iterate all usages once and resolve each to its definition
        for entry in self.usages.iter() {
            let file_path = entry.key();
            let usages = entry.value();
            let file_def_lines = fixture_def_lines.get(file_path);

            for usage in usages.iter() {
                // Fast lookup: check if this usage is on a fixture definition line
                let fixture_def_at_line = file_def_lines
                    .and_then(|lines| lines.get(&usage.line))
                    .cloned();

                let is_self_referencing = fixture_def_at_line
                    .as_ref()
                    .is_some_and(|def| def.name == usage.name);

                let resolved_def = if is_self_referencing {
                    // Self-referencing parameter - must resolve without cache
                    self.find_closest_definition_excluding(
                        file_path,
                        &usage.name,
                        fixture_def_at_line.as_ref(),
                    )
                } else {
                    // Use cache for normal resolution
                    let cache_key = (file_path.clone(), usage.name.clone());
                    if let Some(cached) = resolution_cache.get(&cache_key) {
                        // Return cached result
                        cached.as_ref().and_then(|def_path| {
                            // Look up the full definition from the cached path
                            self.definitions.get(&usage.name).and_then(|defs| {
                                defs.iter().find(|d| &d.file_path == def_path).cloned()
                            })
                        })
                    } else {
                        // Resolve and cache
                        let def = self.find_closest_definition(file_path, &usage.name);
                        resolution_cache
                            .insert(cache_key, def.as_ref().map(|d| d.file_path.clone()));
                        def
                    }
                };

                if let Some(def) = resolved_def {
                    let key = (def.file_path.clone(), usage.name.clone());
                    *counts.entry(key).or_insert(0) += 1;
                }
            }
        }

        counts
    }

    /// Print fixtures as a tree structure
    /// Shows directory hierarchy with fixtures defined in each file
    pub fn print_fixtures_tree(&self, root_path: &Path, skip_unused: bool, only_unused: bool) {
        // Collect all files that define fixtures
        let mut file_fixtures: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();

        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            let definitions = entry.value();

            for def in definitions {
                file_fixtures
                    .entry(def.file_path.clone())
                    .or_default()
                    .insert(fixture_name.clone());
            }
        }

        // Count fixture usages using scoped reference counting (Issue #23 fix)
        // Each definition's usage count is based on references that actually resolve to it,
        // not just any usage of the same fixture name globally.
        // Key: (file_path, fixture_name) -> usage_count
        //
        // OPTIMIZATION: Instead of calling find_references_for_definition for each definition
        // (which iterates all usages each time), we iterate usages once and resolve each to
        // its definition, then count per-definition.
        let definition_usage_counts = self.compute_definition_usage_counts();

        // Build a tree structure from paths
        let mut tree: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        let mut all_paths: BTreeSet<PathBuf> = BTreeSet::new();

        for file_path in file_fixtures.keys() {
            all_paths.insert(file_path.clone());

            // Add all parent directories
            let mut current = file_path.as_path();
            while let Some(parent) = current.parent() {
                if parent == root_path || parent.as_os_str().is_empty() {
                    break;
                }
                all_paths.insert(parent.to_path_buf());
                current = parent;
            }
        }

        // Build parent-child relationships
        for path in &all_paths {
            if let Some(parent) = path.parent() {
                if parent != root_path && !parent.as_os_str().is_empty() {
                    tree.entry(parent.to_path_buf())
                        .or_default()
                        .push(path.clone());
                }
            }
        }

        // Sort children in each directory
        for children in tree.values_mut() {
            children.sort();
        }

        // Print the tree
        println!("Fixtures tree for: {}", root_path.display());
        println!();

        if file_fixtures.is_empty() {
            println!("No fixtures found in this directory.");
            return;
        }

        // Find top-level items (direct children of root)
        let mut top_level: Vec<PathBuf> = all_paths
            .iter()
            .filter(|p| {
                if let Some(parent) = p.parent() {
                    parent == root_path
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        top_level.sort();

        for (i, path) in top_level.iter().enumerate() {
            let is_last = i == top_level.len() - 1;
            self.print_tree_node(
                path,
                &file_fixtures,
                &tree,
                "",
                is_last,
                true, // is_root_level
                &definition_usage_counts,
                skip_unused,
                only_unused,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::only_used_in_recursion)]
    fn print_tree_node(
        &self,
        path: &Path,
        file_fixtures: &BTreeMap<PathBuf, BTreeSet<String>>,
        tree: &BTreeMap<PathBuf, Vec<PathBuf>>,
        prefix: &str,
        is_last: bool,
        is_root_level: bool,
        definition_usage_counts: &HashMap<(PathBuf, String), usize>,
        skip_unused: bool,
        only_unused: bool,
    ) {
        use colored::Colorize;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        // Print current node
        let connector = if is_root_level {
            "" // No connector for root level
        } else if is_last {
            "└── "
        } else {
            "├── "
        };

        if path.is_file() {
            // Print file with fixtures
            if let Some(fixtures) = file_fixtures.get(path) {
                // Filter fixtures based on flags
                let fixture_vec: Vec<_> = fixtures
                    .iter()
                    .filter(|fixture_name| {
                        // Look up usage count using (file_path, fixture_name) key
                        let usage_count = definition_usage_counts
                            .get(&(path.to_path_buf(), (*fixture_name).clone()))
                            .copied()
                            .unwrap_or(0);
                        if only_unused {
                            usage_count == 0
                        } else if skip_unused {
                            usage_count > 0
                        } else {
                            true
                        }
                    })
                    .collect();

                // Skip this file if no fixtures match the filter
                if fixture_vec.is_empty() {
                    return;
                }

                let file_display = name.to_string().cyan().bold();
                println!(
                    "{}{}{} ({} fixtures)",
                    prefix,
                    connector,
                    file_display,
                    fixture_vec.len()
                );

                // Print fixtures in this file
                let new_prefix = if is_root_level {
                    "".to_string()
                } else {
                    format!("{}{}", prefix, if is_last { "    " } else { "│   " })
                };

                for (j, fixture_name) in fixture_vec.iter().enumerate() {
                    let is_last_fixture = j == fixture_vec.len() - 1;
                    let fixture_connector = if is_last_fixture {
                        "└── "
                    } else {
                        "├── "
                    };

                    // Get usage count for this specific definition (file_path, fixture_name)
                    let usage_count = definition_usage_counts
                        .get(&(path.to_path_buf(), (*fixture_name).clone()))
                        .copied()
                        .unwrap_or(0);

                    // Format the fixture name with color based on usage
                    let fixture_display = if usage_count == 0 {
                        // Unused fixture - show in dim/gray
                        fixture_name.to_string().dimmed()
                    } else {
                        // Used fixture - show in green
                        fixture_name.to_string().green()
                    };

                    // Format usage count
                    let usage_info = if usage_count == 0 {
                        "unused".dimmed().to_string()
                    } else if usage_count == 1 {
                        format!("{}", "used 1 time".yellow())
                    } else {
                        format!("{}", format!("used {} times", usage_count).yellow())
                    };

                    println!(
                        "{}{}{} ({})",
                        new_prefix, fixture_connector, fixture_display, usage_info
                    );
                }
            } else {
                println!("{}{}{}", prefix, connector, name);
            }
        } else {
            // Print directory - but first check if it has any visible children
            if let Some(children) = tree.get(path) {
                // Check if any children will be visible
                let has_visible_children = children.iter().any(|child| {
                    Self::has_visible_fixtures(
                        child,
                        file_fixtures,
                        tree,
                        definition_usage_counts,
                        skip_unused,
                        only_unused,
                    )
                });

                if !has_visible_children {
                    return;
                }

                let dir_display = format!("{}/", name).blue().bold();
                println!("{}{}{}", prefix, connector, dir_display);

                let new_prefix = if is_root_level {
                    "".to_string()
                } else {
                    format!("{}{}", prefix, if is_last { "    " } else { "│   " })
                };

                for (j, child) in children.iter().enumerate() {
                    let is_last_child = j == children.len() - 1;
                    self.print_tree_node(
                        child,
                        file_fixtures,
                        tree,
                        &new_prefix,
                        is_last_child,
                        false, // is_root_level
                        definition_usage_counts,
                        skip_unused,
                        only_unused,
                    );
                }
            }
        }
    }

    fn has_visible_fixtures(
        path: &Path,
        file_fixtures: &BTreeMap<PathBuf, BTreeSet<String>>,
        tree: &BTreeMap<PathBuf, Vec<PathBuf>>,
        definition_usage_counts: &HashMap<(PathBuf, String), usize>,
        skip_unused: bool,
        only_unused: bool,
    ) -> bool {
        if path.is_file() {
            // Check if this file has any fixtures matching the filter
            if let Some(fixtures) = file_fixtures.get(path) {
                return fixtures.iter().any(|fixture_name| {
                    let usage_count = definition_usage_counts
                        .get(&(path.to_path_buf(), fixture_name.clone()))
                        .copied()
                        .unwrap_or(0);
                    if only_unused {
                        usage_count == 0
                    } else if skip_unused {
                        usage_count > 0
                    } else {
                        true
                    }
                });
            }
            false
        } else {
            // Check if any children have visible fixtures
            if let Some(children) = tree.get(path) {
                children.iter().any(|child| {
                    Self::has_visible_fixtures(
                        child,
                        file_fixtures,
                        tree,
                        definition_usage_counts,
                        skip_unused,
                        only_unused,
                    )
                })
            } else {
                false
            }
        }
    }
}
