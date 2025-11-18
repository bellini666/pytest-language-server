use dashmap::DashMap;
use ruff_python_ast::{Decorator, Expr, Stmt};
use ruff_python_parser::parse_module;
use ruff_text_size::Ranged;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureDefinition {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
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
        }
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

    /// Scan a workspace directory for test files and conftest.py files
    pub fn scan_workspace(&self, root_path: &Path) {
        info!("Scanning workspace: {:?}", root_path);
        let mut file_count = 0;

        for entry in WalkDir::new(root_path).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();

            // Look for conftest.py or test_*.py or *_test.py files
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename == "conftest.py"
                    || filename.starts_with("test_") && filename.ends_with(".py")
                    || filename.ends_with("_test.py")
                {
                    debug!("Found test/conftest file: {:?}", path);
                    if let Ok(content) = std::fs::read_to_string(path) {
                        self.analyze_file(path.to_path_buf(), &content);
                        file_count += 1;
                    }
                }
            }
        }

        info!("Workspace scan complete. Processed {} files", file_count);

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
        // Canonicalize the path to handle symlinks and normalize path representation
        // This ensures consistent path comparisons later
        let file_path = file_path.canonicalize().unwrap_or_else(|_| {
            // If canonicalization fails (e.g., file doesn't exist yet, or on some filesystems),
            // fall back to the original path
            debug!(
                "Warning: Could not canonicalize path {:?}, using as-is",
                file_path
            );
            file_path
        });

        debug!("Analyzing file: {:?}", file_path);

        // Cache the file content for later use (e.g., in find_fixture_definition)
        // Use Arc for efficient sharing without cloning
        self.file_cache
            .insert(file_path.clone(), Arc::new(content.to_string()));

        // Parse the Python code
        let parsed = match parse_module(content) {
            Ok(ast) => ast,
            Err(e) => {
                warn!("Failed to parse {:?}: {:?}", file_path, e);
                return;
            }
        };

        // Clear previous usages for this file
        self.usages.remove(&file_path);

        // Clear previous undeclared fixtures for this file
        self.undeclared_fixtures.remove(&file_path);

        // Clear previous imports for this file
        self.imports.remove(&file_path);

        // Clear previous fixture definitions from this file
        // We need to remove definitions that were in this file
        for mut entry in self.definitions.iter_mut() {
            entry.value_mut().retain(|def| def.file_path != file_path);
        }
        // Remove empty entries
        self.definitions.retain(|_, defs| !defs.is_empty());

        // Check if this is a conftest.py
        let is_conftest = file_path
            .file_name()
            .map(|n| n == "conftest.py")
            .unwrap_or(false);
        debug!("is_conftest: {}", is_conftest);

        // Process each statement in the module
        let module = parsed.into_syntax();
        debug!("Module has {} statements", module.body.len());

        // First pass: collect all module-level names (imports, assignments, function/class defs)
        let mut module_level_names = std::collections::HashSet::new();
        for stmt in &module.body {
            self.collect_module_level_names(stmt, &mut module_level_names);
        }
        self.imports.insert(file_path.clone(), module_level_names);

        // Second pass: analyze fixtures and tests
        for stmt in &module.body {
            self.visit_stmt(stmt, &file_path, is_conftest, content);
        }

        debug!("Analysis complete for {:?}", file_path);
    }

    fn visit_stmt(&self, stmt: &Stmt, file_path: &PathBuf, _is_conftest: bool, content: &str) {
        // First check for assignment-style fixtures: fixture_name = pytest.fixture()(func)
        if let Stmt::Assign(assign) = stmt {
            self.visit_assignment_fixture(assign, file_path, content);
        }

        // Handle both regular and async function definitions
        let (func_name, decorator_list, args, range, body, returns) = match stmt {
            Stmt::FunctionDef(func_def) => (
                func_def.name.as_str(),
                &func_def.decorator_list,
                &func_def.parameters,
                func_def.range,
                &func_def.body,
                &func_def.returns,
            ),
            _ => return,
        };

        debug!("Found function: {}", func_name);

        // Check if this is a fixture definition
        debug!(
            "Function {} has {} decorators",
            func_name,
            decorator_list.len()
        );
        let is_fixture = decorator_list.iter().any(|dec| {
            let result = Self::is_fixture_decorator(dec);
            if result {
                debug!("  Decorator matched as fixture!");
            }
            result
        });

        if is_fixture {
            // Calculate line number from the function def, not the decorators
            // In ruff, range includes decorators, so we need to find the actual def line
            // Search for "def <func_name>" after the range start
            let range_start = range.start().to_usize();
            let search_str = format!("def {}", func_name);
            let def_offset = content[range_start..]
                .find(&search_str)
                .map(|pos| range_start + pos)
                .unwrap_or(range_start);
            let line = self.get_line_from_offset(def_offset, content);

            // Extract docstring if present
            let docstring = self.extract_docstring(body);

            // Extract return type annotation
            let return_type = self.extract_return_type(returns, body, content);

            info!(
                "Found fixture definition: {} at {:?}:{}",
                func_name, file_path, line
            );
            if let Some(ref doc) = docstring {
                debug!("  Docstring: {}", doc);
            }
            if let Some(ref ret_type) = return_type {
                debug!("  Return type: {}", ret_type);
            }

            let definition = FixtureDefinition {
                name: func_name.to_string(),
                file_path: file_path.clone(),
                line,
                docstring,
                return_type,
            };

            self.definitions
                .entry(func_name.to_string())
                .or_default()
                .push(definition);

            // Fixtures can depend on other fixtures - record these as usages too
            let mut declared_params: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            declared_params.insert("self".to_string());
            declared_params.insert("request".to_string());
            declared_params.insert(func_name.to_string()); // Exclude function name itself

            for arg in &args.args {
                let arg_name = arg.parameter.name.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" && arg_name != "request" {
                    // Get the actual line where this parameter appears
                    // arg.parameter.range contains the location of the parameter name
                    let arg_line = self
                        .get_line_from_offset(arg.parameter.range().start().to_usize(), content);
                    let start_char = self.get_char_position_from_offset(
                        arg.parameter.range().start().to_usize(),
                        content,
                    );
                    let end_char = self.get_char_position_from_offset(
                        arg.parameter.range().end().to_usize(),
                        content,
                    );

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
            let function_line = self.get_line_from_offset(range.start().to_usize(), content);
            self.scan_function_body_for_undeclared_fixtures(
                body,
                file_path,
                content,
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
            for arg in &args.args {
                let arg_name = arg.parameter.name.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" {
                    // Get the actual line where this parameter appears
                    // This handles multiline function signatures correctly
                    // arg.parameter.range contains the location of the parameter name
                    let arg_offset = arg.parameter.range().start().to_usize();
                    let arg_line = self.get_line_from_offset(arg_offset, content);
                    let start_char = self.get_char_position_from_offset(arg_offset, content);
                    let end_char = self.get_char_position_from_offset(
                        arg.parameter.range().end().to_usize(),
                        content,
                    );

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
            let function_line = self.get_line_from_offset(range.start().to_usize(), content);
            self.scan_function_body_for_undeclared_fixtures(
                body,
                file_path,
                content,
                &declared_params,
                func_name,
                function_line,
            );
        }
    }

    fn visit_assignment_fixture(
        &self,
        assign: &ruff_python_ast::StmtAssign,
        file_path: &PathBuf,
        content: &str,
    ) {
        // Check for pattern: fixture_name = pytest.fixture()(func)
        // The value should be a Call expression where the func is a Call to pytest.fixture()

        if let Expr::Call(outer_call) = &*assign.value {
            // Check if outer_call.func is pytest.fixture() or fixture()
            if let Expr::Call(inner_call) = &*outer_call.func {
                if Self::is_fixture_expr(&inner_call.func) {
                    // This is pytest.fixture()(something)
                    // Get the fixture name from the assignment target
                    for target in &assign.targets {
                        if let Expr::Name(name) = target {
                            let fixture_name = name.id.as_str();
                            let line = self
                                .get_line_from_offset(assign.range().start().to_usize(), content);

                            info!(
                                "Found fixture assignment: {} at {:?}:{}",
                                fixture_name, file_path, line
                            );

                            // We don't have a docstring or return type for assignment-style fixtures
                            let definition = FixtureDefinition {
                                name: fixture_name.to_string(),
                                file_path: file_path.clone(),
                                line,
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

    fn is_fixture_decorator(decorator: &Decorator) -> bool {
        let expr = &decorator.expression;
        Self::is_fixture_expr(expr)
    }

    fn is_fixture_expr(expr: &Expr) -> bool {
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
                // Need to check the call.func expression
                match &*call.func {
                    Expr::Name(name) => name.id.as_str() == "fixture",
                    Expr::Attribute(attr) => {
                        if let Expr::Name(value) = &*attr.value {
                            value.id.as_str() == "pytest" && attr.attr.as_str() == "fixture"
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn scan_function_body_for_undeclared_fixtures(
        &self,
        body: &[Stmt],
        file_path: &PathBuf,
        content: &str,
        declared_params: &std::collections::HashSet<String>,
        function_name: &str,
        function_line: usize,
    ) {
        // First, collect all local variable names with their definition line numbers
        let mut local_vars = std::collections::HashMap::new();
        self.collect_local_variables(body, content, &mut local_vars);

        // Also add imported names to local_vars (they shouldn't be flagged as undeclared fixtures)
        // Set their line to 0 so they're always considered "in scope"
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
            // Function definitions (both regular and async, not fixtures)
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

    fn collect_local_variables(
        &self,
        body: &[Stmt],
        content: &str,
        local_vars: &mut std::collections::HashMap<String, usize>,
    ) {
        for stmt in body {
            match stmt {
                Stmt::Assign(assign) => {
                    // Collect variable names from left-hand side with their line numbers
                    let line =
                        self.get_line_from_offset(assign.range().start().to_usize(), content);
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
                        self.get_line_from_offset(ann_assign.range().start().to_usize(), content);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&ann_assign.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::AugAssign(aug_assign) => {
                    // Collect augmented assignment targets (+=, -=, etc.)
                    let line =
                        self.get_line_from_offset(aug_assign.range().start().to_usize(), content);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&aug_assign.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::For(for_stmt) => {
                    // Collect loop variable with its line number (handles both sync and async for)
                    let line =
                        self.get_line_from_offset(for_stmt.range().start().to_usize(), content);
                    let mut temp_names = std::collections::HashSet::new();
                    self.collect_names_from_expr(&for_stmt.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                    // Recursively collect from body
                    self.collect_local_variables(&for_stmt.body, content, local_vars);
                }
                Stmt::While(while_stmt) => {
                    self.collect_local_variables(&while_stmt.body, content, local_vars);
                }
                Stmt::If(if_stmt) => {
                    self.collect_local_variables(&if_stmt.body, content, local_vars);
                    // Handle elif/else clauses
                    for clause in &if_stmt.elif_else_clauses {
                        self.collect_local_variables(&clause.body, content, local_vars);
                    }
                }
                Stmt::With(with_stmt) => {
                    // Collect context manager variables with their line numbers (handles both sync and async with)
                    let line =
                        self.get_line_from_offset(with_stmt.range().start().to_usize(), content);
                    for item in &with_stmt.items {
                        if let Some(ref optional_vars) = item.optional_vars {
                            let mut temp_names = std::collections::HashSet::new();
                            self.collect_names_from_expr(optional_vars, &mut temp_names);
                            for name in temp_names {
                                local_vars.insert(name, line);
                            }
                        }
                    }
                    self.collect_local_variables(&with_stmt.body, content, local_vars);
                }
                Stmt::Try(try_stmt) => {
                    self.collect_local_variables(&try_stmt.body, content, local_vars);
                    // Note: ExceptHandler struct doesn't expose name/body in current API
                    // This is a limitation of rustpython-parser 0.4.0
                    self.collect_local_variables(&try_stmt.orelse, content, local_vars);
                    self.collect_local_variables(&try_stmt.finalbody, content, local_vars);
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
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
                // Handle elif/else clauses
                for clause in &if_stmt.elif_else_clauses {
                    for stmt in &clause.body {
                        self.visit_stmt_for_names(
                            stmt,
                            file_path,
                            content,
                            declared_params,
                            local_vars,
                            function_name,
                            function_line,
                        );
                    }
                }
            }
            Stmt::While(while_stmt) => {
                self.visit_expr_for_names(
                    &while_stmt.test,
                    file_path,
                    content,
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
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::For(for_stmt) => {
                // Handles both sync and async for loops
                self.visit_expr_for_names(
                    &for_stmt.iter,
                    file_path,
                    content,
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
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Stmt::With(with_stmt) => {
                // Handles both sync and async with statements
                for item in &with_stmt.items {
                    self.visit_expr_for_names(
                        &item.context_expr,
                        file_path,
                        content,
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

    #[allow(clippy::too_many_arguments)]
    fn visit_expr_for_names(
        &self,
        expr: &Expr,
        file_path: &PathBuf,
        content: &str,
        declared_params: &std::collections::HashSet<String>,
        local_vars: &std::collections::HashMap<String, usize>,
        function_name: &str,
        function_line: usize,
    ) {
        match expr {
            Expr::Name(name) => {
                let name_str = name.id.as_str();
                let line = self.get_line_from_offset(name.range().start().to_usize(), content);

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
                        .get_char_position_from_offset(name.range().start().to_usize(), content);
                    let end_char =
                        self.get_char_position_from_offset(name.range().end().to_usize(), content);

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
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                for arg in &call.arguments.args {
                    self.visit_expr_for_names(
                        arg,
                        file_path,
                        content,
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
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                self.visit_expr_for_names(
                    &binop.right,
                    file_path,
                    content,
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
                    declared_params,
                    local_vars,
                    function_name,
                    function_line,
                );
                self.visit_expr_for_names(
                    &subscript.slice,
                    file_path,
                    content,
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
                        declared_params,
                        local_vars,
                        function_name,
                        function_line,
                    );
                }
            }
            Expr::Dict(dict) => {
                for item in &dict.items {
                    if let Some(ref k) = item.key {
                        self.visit_expr_for_names(
                            k,
                            file_path,
                            content,
                            declared_params,
                            local_vars,
                            function_name,
                            function_line,
                        );
                    }
                    self.visit_expr_for_names(
                        &item.value,
                        file_path,
                        content,
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
            if let Expr::StringLiteral(string_lit) = &*expr_stmt.value {
                // Check if the constant is a string
                return Some(self.format_docstring(string_lit.value.to_string()));
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
        returns: &Option<Box<Expr>>,
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
                    if self.contains_yield(&if_stmt.body) {
                        return true;
                    }
                    // Check elif/else clauses
                    for clause in &if_stmt.elif_else_clauses {
                        if self.contains_yield(&clause.body) {
                            return true;
                        }
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
                    // Note: ExceptHandler struct doesn't expose body in current API
                    // This is a limitation of rustpython-parser 0.4.0
                }
                _ => {}
            }
        }
        false
    }

    fn extract_yielded_type(&self, expr: &Expr, content: &str) -> Option<String> {
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
    fn expr_to_string(&self, expr: &Expr, _content: &str) -> String {
        match expr {
            Expr::Name(name) => name.id.to_string(),
            Expr::Attribute(attr) => {
                format!(
                    "{}.{}",
                    self.expr_to_string(&attr.value, _content),
                    attr.attr
                )
            }
            Expr::Subscript(subscript) => {
                let base = self.expr_to_string(&subscript.value, _content);
                let slice = self.expr_to_string(&subscript.slice, _content);
                format!("{}[{}]", base, slice)
            }
            Expr::Tuple(tuple) => {
                let elements: Vec<String> = tuple
                    .elts
                    .iter()
                    .map(|e| self.expr_to_string(e, _content))
                    .collect();
                elements.join(", ")
            }
            // Ruff has different literal types instead of Constant
            Expr::NumberLiteral(num) => format!("{:?}", num.value),
            Expr::StringLiteral(s) => format!("{:?}", s.value),
            Expr::BytesLiteral(b) => format!("{:?}", b.value),
            Expr::BooleanLiteral(b) => b.value.to_string(),
            Expr::NoneLiteral(_) => "None".to_string(),
            Expr::EllipsisLiteral(_) => "...".to_string(),
            Expr::BinOp(binop) if matches!(binop.op, ruff_python_ast::Operator::BitOr) => {
                // Handle union types like str | int
                format!(
                    "{} | {}",
                    self.expr_to_string(&binop.left, _content),
                    self.expr_to_string(&binop.right, _content)
                )
            }
            _ => {
                // Fallback for complex types we don't handle yet
                "Any".to_string()
            }
        }
    }

    fn get_line_from_offset(&self, offset: usize, content: &str) -> usize {
        // Count newlines before this offset, then add 1 for 1-based line numbers
        content[..offset].matches('\n').count() + 1
    }

    fn get_char_position_from_offset(&self, offset: usize, content: &str) -> usize {
        // Find the last newline before this offset
        if let Some(line_start) = content[..offset].rfind('\n') {
            // Character position is offset from start of line (after the newline)
            offset - line_start - 1
        } else {
            // No newline found, we're on the first line
            offset
        }
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

        // Priority 4: If still no match, this means the fixture is defined somewhere
        // unrelated to the current file's hierarchy. This is unusual but can happen
        // when fixtures are defined in unrelated test directories.
        // Return the first definition sorted by path for determinism.
        warn!(
            "No fixture {} found following priority rules (same file, conftest hierarchy, third-party)",
            fixture_name
        );
        warn!(
            "Falling back to first definition by path (deterministic fallback for unrelated fixtures)"
        );

        let mut defs: Vec<_> = definitions.iter().cloned().collect();
        defs.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        defs.first().cloned()
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

        // Priority 4: Deterministic fallback - return first definition by path (excluding specified)
        warn!(
            "No fixture {} found following priority rules (excluding specified)",
            fixture_name
        );
        warn!(
            "Falling back to first definition by path (deterministic fallback for unrelated fixtures)"
        );

        let mut defs: Vec<_> = definitions
            .iter()
            .filter(|def| {
                if let Some(excluded) = exclude {
                    def != &excluded
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        defs.sort_by(|a, b| a.file_path.cmp(&b.file_path));
        defs.first().cloned()
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
        let chars: Vec<char> = line.chars().collect();

        // If cursor is beyond the line, return None
        if character > chars.len() {
            return None;
        }

        // Check if cursor is ON an identifier character
        if character < chars.len() {
            let c = chars[character];
            if c.is_alphanumeric() || c == '_' {
                // Cursor is ON an identifier character, extract the word
                let mut start = character;
                while start > 0 {
                    let prev_c = chars[start - 1];
                    if !prev_c.is_alphanumeric() && prev_c != '_' {
                        break;
                    }
                    start -= 1;
                }

                let mut end = character;
                while end < chars.len() {
                    let curr_c = chars[end];
                    if !curr_c.is_alphanumeric() && curr_c != '_' {
                        break;
                    }
                    end += 1;
                }

                if start < end {
                    return Some(chars[start..end].iter().collect());
                }
            }
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
        let parsed = parse_module(&content).ok()?;
        let module = parsed.into_syntax();

        self.find_enclosing_function(&module.body, &content, target_line, character as usize)
    }

    fn find_enclosing_function(
        &self,
        stmts: &[Stmt],
        content: &str,
        target_line: usize,
        _target_char: usize,
    ) -> Option<(String, bool, Vec<String>)> {
        for stmt in stmts {
            if let Stmt::FunctionDef(func_def) = stmt {
                let func_start_line = content[..func_def.range().start().to_usize()]
                    .matches('\n')
                    .count()
                    + 1;
                let func_end_line = content[..func_def.range().end().to_usize()]
                    .matches('\n')
                    .count()
                    + 1;

                // Check if target is within this function's range (handles both sync and async)
                if target_line >= func_start_line && target_line <= func_end_line {
                    let is_fixture = func_def
                        .decorator_list
                        .iter()
                        .any(Self::is_fixture_decorator);
                    let is_test = func_def.name.starts_with("test_");

                    // Only return if it's a test or fixture
                    if is_test || is_fixture {
                        let params: Vec<String> = func_def
                            .parameters
                            .args
                            .iter()
                            .map(|arg| arg.parameter.name.to_string())
                            .collect();

                        return Some((func_def.name.to_string(), is_fixture, params));
                    }
                }
            }
        }

        None
    }
}
