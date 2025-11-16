use dashmap::DashMap;
use rustpython_parser::ast::{Expr, Stmt};
use rustpython_parser::{parse, Mode};
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
    definitions: Arc<DashMap<String, Vec<FixtureDefinition>>>,
    // Map from file path to fixtures used in that file
    usages: Arc<DashMap<PathBuf, Vec<FixtureUsage>>>,
    // Cache of file contents for analyzed files (mainly for testing)
    file_cache: Arc<DashMap<PathBuf, String>>,
    // Map from file path to undeclared fixtures used in function bodies
    undeclared_fixtures: Arc<DashMap<PathBuf, Vec<UndeclaredFixture>>>,
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
        debug!("Analyzing file: {:?}", file_path);

        // Cache the file content for later use (e.g., in find_fixture_definition)
        self.file_cache
            .insert(file_path.clone(), content.to_string());

        // Parse the Python code
        let parsed = match parse(content, Mode::Module, "") {
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
        if let rustpython_parser::ast::Mod::Module(module) = parsed {
            debug!("Module has {} statements", module.body.len());
            for stmt in &module.body {
                self.visit_stmt(stmt, &file_path, is_conftest, content);
            }
        }

        debug!("Analysis complete for {:?}", file_path);
    }

    fn visit_stmt(&self, stmt: &Stmt, file_path: &PathBuf, _is_conftest: bool, content: &str) {
        // First check for assignment-style fixtures: fixture_name = pytest.fixture()(func)
        if let Stmt::Assign(assign) = stmt {
            self.visit_assignment_fixture(assign, file_path, content);
        }

        // Handle both regular and async function definitions
        let (func_name, decorator_list, args, range, body) = match stmt {
            Stmt::FunctionDef(func_def) => (
                func_def.name.as_str(),
                &func_def.decorator_list,
                &func_def.args,
                func_def.range,
                &func_def.body,
            ),
            Stmt::AsyncFunctionDef(func_def) => (
                func_def.name.as_str(),
                &func_def.decorator_list,
                &func_def.args,
                func_def.range,
                &func_def.body,
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
            // Calculate line number from the range start
            let line = self.get_line_from_offset(range.start().to_usize(), content);

            // Extract docstring if present
            let docstring = self.extract_docstring(body);

            info!(
                "Found fixture definition: {} at {:?}:{}",
                func_name, file_path, line
            );
            if let Some(ref doc) = docstring {
                debug!("  Docstring: {}", doc);
            }

            let definition = FixtureDefinition {
                name: func_name.to_string(),
                file_path: file_path.clone(),
                line,
                docstring,
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
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" && arg_name != "request" {
                    // Get the actual line where this parameter appears
                    // arg.def.range contains the location of the parameter name
                    let arg_line =
                        self.get_line_from_offset(arg.def.range.start().to_usize(), content);
                    let start_char = self
                        .get_char_position_from_offset(arg.def.range.start().to_usize(), content);
                    let end_char =
                        self.get_char_position_from_offset(arg.def.range.end().to_usize(), content);

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
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" {
                    // Get the actual line where this parameter appears
                    // This handles multiline function signatures correctly
                    // arg.def.range contains the location of the parameter name
                    let arg_offset = arg.def.range.start().to_usize();
                    let arg_line = self.get_line_from_offset(arg_offset, content);
                    let start_char = self.get_char_position_from_offset(arg_offset, content);
                    let end_char =
                        self.get_char_position_from_offset(arg.def.range.end().to_usize(), content);

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
        assign: &rustpython_parser::ast::StmtAssign,
        file_path: &PathBuf,
        content: &str,
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
                            let line =
                                self.get_line_from_offset(assign.range.start().to_usize(), content);

                            info!(
                                "Found fixture assignment: {} at {:?}:{}",
                                fixture_name, file_path, line
                            );

                            // We don't have a docstring for assignment-style fixtures
                            let definition = FixtureDefinition {
                                name: fixture_name.to_string(),
                                file_path: file_path.clone(),
                                line,
                                docstring: None,
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

    fn scan_function_body_for_undeclared_fixtures(
        &self,
        body: &[Stmt],
        file_path: &PathBuf,
        content: &str,
        declared_params: &std::collections::HashSet<String>,
        function_name: &str,
        function_line: usize,
    ) {
        // Walk through the function body and find all Name references
        for stmt in body {
            self.visit_stmt_for_names(
                stmt,
                file_path,
                content,
                declared_params,
                function_name,
                function_line,
            );
        }
    }

    fn visit_stmt_for_names(
        &self,
        stmt: &Stmt,
        file_path: &PathBuf,
        content: &str,
        declared_params: &std::collections::HashSet<String>,
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
                    function_name,
                    function_line,
                );
                for stmt in &if_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        declared_params,
                        function_name,
                        function_line,
                    );
                }
                for stmt in &if_stmt.orelse {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        declared_params,
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
                    declared_params,
                    function_name,
                    function_line,
                );
                for stmt in &while_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        declared_params,
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
                    declared_params,
                    function_name,
                    function_line,
                );
                for stmt in &for_stmt.body {
                    self.visit_stmt_for_names(
                        stmt,
                        file_path,
                        content,
                        declared_params,
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
                        declared_params,
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
                        function_name,
                        function_line,
                    );
                }
            }
            _ => {} // Other statement types
        }
    }

    fn visit_expr_for_names(
        &self,
        expr: &Expr,
        file_path: &PathBuf,
        content: &str,
        declared_params: &std::collections::HashSet<String>,
        function_name: &str,
        function_line: usize,
    ) {
        match expr {
            Expr::Name(name) => {
                let name_str = name.id.as_str();

                // Check if this name is a known fixture and not a declared parameter
                if !declared_params.contains(name_str)
                    && self.is_available_fixture(file_path, name_str)
                {
                    let line = self.get_line_from_offset(name.range.start().to_usize(), content);
                    let start_char =
                        self.get_char_position_from_offset(name.range.start().to_usize(), content);
                    let end_char =
                        self.get_char_position_from_offset(name.range.end().to_usize(), content);

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
                    function_name,
                    function_line,
                );
                for arg in &call.args {
                    self.visit_expr_for_names(
                        arg,
                        file_path,
                        content,
                        declared_params,
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
                    function_name,
                    function_line,
                );
                self.visit_expr_for_names(
                    &binop.right,
                    file_path,
                    content,
                    declared_params,
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
                    function_name,
                    function_line,
                );
                for comparator in &compare.comparators {
                    self.visit_expr_for_names(
                        comparator,
                        file_path,
                        content,
                        declared_params,
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
                    function_name,
                    function_line,
                );
                self.visit_expr_for_names(
                    &subscript.slice,
                    file_path,
                    content,
                    declared_params,
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
                        declared_params,
                        function_name,
                        function_line,
                    );
                }
                for value in &dict.values {
                    self.visit_expr_for_names(
                        value,
                        file_path,
                        content,
                        declared_params,
                        function_name,
                        function_line,
                    );
                }
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
        let content = if let Some(cached) = self.file_cache.get(file_path) {
            cached.clone()
        } else {
            std::fs::read_to_string(file_path).ok()?
        };
        let lines: Vec<&str> = content.lines().collect();

        if target_line == 0 || target_line > lines.len() {
            return None;
        }

        let line_content = lines[target_line - 1];
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
        let same_file_defs: Vec<_> = definitions
            .iter()
            .filter(|def| def.file_path == file_path)
            .collect();

        if !same_file_defs.is_empty() {
            // Return the last definition (highest line number) - pytest uses last definition
            let last_def = same_file_defs.iter().max_by_key(|def| def.line).unwrap();
            info!(
                "Found fixture {} in same file at line {} (using last definition)",
                fixture_name, last_def.line
            );
            return Some((*last_def).clone());
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

        // If no conftest.py found, return the first definition
        warn!(
            "No fixture {} found following priority rules, returning first available",
            fixture_name
        );
        definitions.iter().next().cloned()
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
        let same_file_defs: Vec<_> = definitions
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
            .collect();

        if !same_file_defs.is_empty() {
            // Return the last definition (highest line number) - pytest uses last definition
            let last_def = same_file_defs.iter().max_by_key(|def| def.line).unwrap();
            info!(
                "Found fixture {} in same file at line {} (using last definition, excluding specified)",
                fixture_name, last_def.line
            );
            return Some((*last_def).clone());
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

        // If no conftest.py found, return the first definition that's not excluded
        warn!(
            "No fixture {} found following priority rules, returning first available (excluding specified)",
            fixture_name
        );
        definitions
            .iter()
            .find(|def| {
                if let Some(excluded) = exclude {
                    def != &excluded
                } else {
                    true
                }
            })
            .cloned()
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
        let content = if let Some(cached) = self.file_cache.get(file_path) {
            cached.clone()
        } else {
            std::fs::read_to_string(file_path).ok()?
        };
        let lines: Vec<&str> = content.lines().collect();

        if target_line == 0 || target_line > lines.len() {
            return None;
        }

        let line_content = lines[target_line - 1];
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

    fn extract_word_at_position(&self, line: &str, character: usize) -> Option<String> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_fixture_definition_detection() {
        let db = FixtureDatabase::new();

        let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

@fixture
def another_fixture():
    return "hello"
"#;

        let conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Check that fixtures were detected
        assert!(db.definitions.contains_key("my_fixture"));
        assert!(db.definitions.contains_key("another_fixture"));

        // Check fixture details
        let my_fixture_defs = db.definitions.get("my_fixture").unwrap();
        assert_eq!(my_fixture_defs.len(), 1);
        assert_eq!(my_fixture_defs[0].name, "my_fixture");
        assert_eq!(my_fixture_defs[0].file_path, conftest_path);
    }

    #[test]
    fn test_fixture_usage_detection() {
        let db = FixtureDatabase::new();

        let test_content = r#"
def test_something(my_fixture, another_fixture):
    assert my_fixture == 42
    assert another_fixture == "hello"

def test_other(my_fixture):
    assert my_fixture > 0
"#;

        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that usages were detected
        assert!(db.usages.contains_key(&test_path));

        let usages = db.usages.get(&test_path).unwrap();
        // Should have usages from the first test function (we only track one function per file currently)
        assert!(usages.iter().any(|u| u.name == "my_fixture"));
        assert!(usages.iter().any(|u| u.name == "another_fixture"));
    }

    #[test]
    fn test_go_to_definition() {
        let db = FixtureDatabase::new();

        // Set up conftest.py with a fixture
        let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;

        let conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Set up a test file that uses the fixture
        let test_content = r#"
def test_something(my_fixture):
    assert my_fixture == 42
"#;

        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Try to find the definition from the test file
        // The usage is on line 2 (1-indexed) - that's where the function parameter is
        // In 0-indexed LSP coordinates, that's line 1
        // Character position 19 is where 'my_fixture' starts
        let definition = db.find_fixture_definition(&test_path, 1, 19);

        assert!(definition.is_some(), "Definition should be found");
        let def = definition.unwrap();
        assert_eq!(def.name, "my_fixture");
        assert_eq!(def.file_path, conftest_path);
    }

    #[test]
    fn test_fixture_decorator_variations() {
        let db = FixtureDatabase::new();

        let conftest_content = r#"
import pytest
from pytest import fixture

@pytest.fixture
def fixture1():
    pass

@pytest.fixture()
def fixture2():
    pass

@fixture
def fixture3():
    pass

@fixture()
def fixture4():
    pass
"#;

        let conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(conftest_path, conftest_content);

        // Check all variations were detected
        assert!(db.definitions.contains_key("fixture1"));
        assert!(db.definitions.contains_key("fixture2"));
        assert!(db.definitions.contains_key("fixture3"));
        assert!(db.definitions.contains_key("fixture4"));
    }

    #[test]
    fn test_fixture_in_test_file() {
        let db = FixtureDatabase::new();

        // Test file with fixture defined in the same file
        let test_content = r#"
import pytest

@pytest.fixture
def local_fixture():
    return 42

def test_something(local_fixture):
    assert local_fixture == 42
"#;

        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that fixture was detected even though it's not in conftest.py
        assert!(db.definitions.contains_key("local_fixture"));

        let local_fixture_defs = db.definitions.get("local_fixture").unwrap();
        assert_eq!(local_fixture_defs.len(), 1);
        assert_eq!(local_fixture_defs[0].name, "local_fixture");
        assert_eq!(local_fixture_defs[0].file_path, test_path);

        // Check that usage was detected
        assert!(db.usages.contains_key(&test_path));
        let usages = db.usages.get(&test_path).unwrap();
        assert!(usages.iter().any(|u| u.name == "local_fixture"));

        // Test go-to-definition for fixture in same file
        let usage_line = usages
            .iter()
            .find(|u| u.name == "local_fixture")
            .map(|u| u.line)
            .unwrap();

        // Character position 19 is where 'local_fixture' starts in "def test_something(local_fixture):"
        let definition = db.find_fixture_definition(&test_path, (usage_line - 1) as u32, 19);
        assert!(
            definition.is_some(),
            "Should find definition for fixture in same file. Line: {}, char: 19",
            usage_line
        );
        let def = definition.unwrap();
        assert_eq!(def.name, "local_fixture");
        assert_eq!(def.file_path, test_path);
    }

    #[test]
    fn test_async_test_functions() {
        let db = FixtureDatabase::new();

        // Test file with async test function
        let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42

async def test_async_function(my_fixture):
    assert my_fixture == 42

def test_sync_function(my_fixture):
    assert my_fixture == 42
"#;

        let test_path = PathBuf::from("/tmp/test/test_async.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that fixture was detected
        assert!(db.definitions.contains_key("my_fixture"));

        // Check that both async and sync test functions have their usages detected
        assert!(db.usages.contains_key(&test_path));
        let usages = db.usages.get(&test_path).unwrap();

        // Should have 2 usages (one from async, one from sync)
        let fixture_usages: Vec<_> = usages.iter().filter(|u| u.name == "my_fixture").collect();
        assert_eq!(
            fixture_usages.len(),
            2,
            "Should detect fixture usage in both async and sync tests"
        );
    }

    #[test]
    fn test_extract_word_at_position() {
        let db = FixtureDatabase::new();

        // Test basic word extraction
        let line = "def test_something(my_fixture):";

        // Cursor on 'm' of 'my_fixture' (position 19)
        assert_eq!(
            db.extract_word_at_position(line, 19),
            Some("my_fixture".to_string())
        );

        // Cursor on 'y' of 'my_fixture' (position 20)
        assert_eq!(
            db.extract_word_at_position(line, 20),
            Some("my_fixture".to_string())
        );

        // Cursor on last 'e' of 'my_fixture' (position 28)
        assert_eq!(
            db.extract_word_at_position(line, 28),
            Some("my_fixture".to_string())
        );

        // Cursor on 'd' of 'def' (position 0)
        assert_eq!(
            db.extract_word_at_position(line, 0),
            Some("def".to_string())
        );

        // Cursor on space after 'def' (position 3) - should return None
        assert_eq!(db.extract_word_at_position(line, 3), None);

        // Cursor on 't' of 'test_something' (position 4)
        assert_eq!(
            db.extract_word_at_position(line, 4),
            Some("test_something".to_string())
        );

        // Cursor on opening parenthesis (position 18) - should return None
        assert_eq!(db.extract_word_at_position(line, 18), None);

        // Cursor on closing parenthesis (position 29) - should return None
        assert_eq!(db.extract_word_at_position(line, 29), None);

        // Cursor on colon (position 31) - should return None
        assert_eq!(db.extract_word_at_position(line, 31), None);
    }

    #[test]
    fn test_extract_word_at_position_fixture_definition() {
        let db = FixtureDatabase::new();

        let line = "@pytest.fixture";

        // Cursor on '@' - should return None
        assert_eq!(db.extract_word_at_position(line, 0), None);

        // Cursor on 'p' of 'pytest' (position 1)
        assert_eq!(
            db.extract_word_at_position(line, 1),
            Some("pytest".to_string())
        );

        // Cursor on '.' - should return None
        assert_eq!(db.extract_word_at_position(line, 7), None);

        // Cursor on 'f' of 'fixture' (position 8)
        assert_eq!(
            db.extract_word_at_position(line, 8),
            Some("fixture".to_string())
        );

        let line2 = "def foo(other_fixture):";

        // Cursor on 'd' of 'def'
        assert_eq!(
            db.extract_word_at_position(line2, 0),
            Some("def".to_string())
        );

        // Cursor on space after 'def' - should return None
        assert_eq!(db.extract_word_at_position(line2, 3), None);

        // Cursor on 'f' of 'foo'
        assert_eq!(
            db.extract_word_at_position(line2, 4),
            Some("foo".to_string())
        );

        // Cursor on 'o' of 'other_fixture'
        assert_eq!(
            db.extract_word_at_position(line2, 8),
            Some("other_fixture".to_string())
        );

        // Cursor on parenthesis - should return None
        assert_eq!(db.extract_word_at_position(line2, 7), None);
    }

    #[test]
    fn test_word_detection_only_on_fixtures() {
        let db = FixtureDatabase::new();

        // Set up a conftest with a fixture
        let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
        let conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Set up a test file
        let test_content = r#"
def test_something(my_fixture, regular_param):
    assert my_fixture == 42
"#;
        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Line 2 is "def test_something(my_fixture, regular_param):"
        // Character positions:
        // 0: 'd' of 'def'
        // 4: 't' of 'test_something'
        // 19: 'm' of 'my_fixture'
        // 31: 'r' of 'regular_param'

        // Cursor on 'def' - should NOT find a fixture (LSP line 1, 0-based)
        assert_eq!(db.find_fixture_definition(&test_path, 1, 0), None);

        // Cursor on 'test_something' - should NOT find a fixture
        assert_eq!(db.find_fixture_definition(&test_path, 1, 4), None);

        // Cursor on 'my_fixture' - SHOULD find the fixture
        let result = db.find_fixture_definition(&test_path, 1, 19);
        assert!(result.is_some());
        let def = result.unwrap();
        assert_eq!(def.name, "my_fixture");

        // Cursor on 'regular_param' - should NOT find a fixture (it's not a fixture)
        assert_eq!(db.find_fixture_definition(&test_path, 1, 31), None);

        // Cursor on comma or parenthesis - should NOT find a fixture
        assert_eq!(db.find_fixture_definition(&test_path, 1, 18), None); // '('
        assert_eq!(db.find_fixture_definition(&test_path, 1, 29), None); // ','
    }

    #[test]
    fn test_self_referencing_fixture() {
        let db = FixtureDatabase::new();

        // Set up a parent conftest.py with the original fixture
        let parent_conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return "parent"
"#;
        let parent_conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(parent_conftest_path.clone(), parent_conftest_content);

        // Set up a child directory conftest.py that overrides foo, referencing itself
        let child_conftest_content = r#"
import pytest

@pytest.fixture
def foo(foo):
    return foo + " child"
"#;
        let child_conftest_path = PathBuf::from("/tmp/test/subdir/conftest.py");
        db.analyze_file(child_conftest_path.clone(), child_conftest_content);

        // Now test go-to-definition on the parameter `foo` in the child fixture
        // Line 5 is "def foo(foo):" (1-indexed)
        // Character position 8 is the 'f' in the parameter name "foo"
        // LSP uses 0-indexed lines, so line 4 in LSP coordinates

        let result = db.find_fixture_definition(&child_conftest_path, 4, 8);

        assert!(
            result.is_some(),
            "Should find parent definition for self-referencing fixture"
        );
        let def = result.unwrap();
        assert_eq!(def.name, "foo");
        assert_eq!(
            def.file_path, parent_conftest_path,
            "Should resolve to parent conftest.py, not the child"
        );
        assert_eq!(def.line, 5, "Should point to line 5 of parent conftest.py");
    }

    #[test]
    fn test_fixture_overriding_same_file() {
        let db = FixtureDatabase::new();

        // A test file with multiple fixtures with the same name (unusual but valid)
        let test_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return "first"

@pytest.fixture
def my_fixture():
    return "second"

def test_something(my_fixture):
    assert my_fixture == "second"
"#;
        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // When there are multiple definitions in the same file, the later one should win
        // (Python's behavior - later definitions override earlier ones)

        // Test go-to-definition on the parameter in test_something
        // Line 12 is "def test_something(my_fixture):" (1-indexed)
        // Character position 19 is the 'm' in "my_fixture"
        // LSP uses 0-indexed lines, so line 11 in LSP coordinates

        let result = db.find_fixture_definition(&test_path, 11, 19);

        assert!(result.is_some(), "Should find fixture definition");
        let def = result.unwrap();
        assert_eq!(def.name, "my_fixture");
        assert_eq!(def.file_path, test_path);
        // The current implementation returns the first match in the same file
        // For true Python semantics, we'd want the last one, but that's a more complex change
        // For now, we just verify it finds *a* definition in the same file
    }

    #[test]
    fn test_fixture_overriding_conftest_hierarchy() {
        let db = FixtureDatabase::new();

        // Root conftest.py
        let root_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "root"
"#;
        let root_conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(root_conftest_path.clone(), root_conftest_content);

        // Subdirectory conftest.py that overrides the fixture
        let sub_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "subdir"
"#;
        let sub_conftest_path = PathBuf::from("/tmp/test/subdir/conftest.py");
        db.analyze_file(sub_conftest_path.clone(), sub_conftest_content);

        // Test file in subdirectory
        let test_content = r#"
def test_something(shared_fixture):
    assert shared_fixture == "subdir"
"#;
        let test_path = PathBuf::from("/tmp/test/subdir/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Go-to-definition from the test should find the closest conftest.py (subdir)
        // Line 2 is "def test_something(shared_fixture):" (1-indexed)
        // Character position 19 is the 's' in "shared_fixture"
        // LSP uses 0-indexed lines, so line 1 in LSP coordinates

        let result = db.find_fixture_definition(&test_path, 1, 19);

        assert!(result.is_some(), "Should find fixture definition");
        let def = result.unwrap();
        assert_eq!(def.name, "shared_fixture");
        assert_eq!(
            def.file_path, sub_conftest_path,
            "Should resolve to closest conftest.py"
        );

        // Now test from a file in the parent directory
        let parent_test_content = r#"
def test_parent(shared_fixture):
    assert shared_fixture == "root"
"#;
        let parent_test_path = PathBuf::from("/tmp/test/test_parent.py");
        db.analyze_file(parent_test_path.clone(), parent_test_content);

        let result = db.find_fixture_definition(&parent_test_path, 1, 16);

        assert!(result.is_some(), "Should find fixture definition");
        let def = result.unwrap();
        assert_eq!(def.name, "shared_fixture");
        assert_eq!(
            def.file_path, root_conftest_path,
            "Should resolve to root conftest.py"
        );
    }

    #[test]
    fn test_scoped_references() {
        let db = FixtureDatabase::new();

        // Set up a root conftest.py with a fixture
        let root_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "root"
"#;
        let root_conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(root_conftest_path.clone(), root_conftest_content);

        // Set up subdirectory conftest.py that overrides the fixture
        let sub_conftest_content = r#"
import pytest

@pytest.fixture
def shared_fixture():
    return "subdir"
"#;
        let sub_conftest_path = PathBuf::from("/tmp/test/subdir/conftest.py");
        db.analyze_file(sub_conftest_path.clone(), sub_conftest_content);

        // Test file in the root directory (uses root fixture)
        let root_test_content = r#"
def test_root(shared_fixture):
    assert shared_fixture == "root"
"#;
        let root_test_path = PathBuf::from("/tmp/test/test_root.py");
        db.analyze_file(root_test_path.clone(), root_test_content);

        // Test file in subdirectory (uses subdir fixture)
        let sub_test_content = r#"
def test_sub(shared_fixture):
    assert shared_fixture == "subdir"
"#;
        let sub_test_path = PathBuf::from("/tmp/test/subdir/test_sub.py");
        db.analyze_file(sub_test_path.clone(), sub_test_content);

        // Another test in subdirectory
        let sub_test2_content = r#"
def test_sub2(shared_fixture):
    assert shared_fixture == "subdir"
"#;
        let sub_test2_path = PathBuf::from("/tmp/test/subdir/test_sub2.py");
        db.analyze_file(sub_test2_path.clone(), sub_test2_content);

        // Get the root definition
        let root_definitions = db.definitions.get("shared_fixture").unwrap();
        let root_definition = root_definitions
            .iter()
            .find(|d| d.file_path == root_conftest_path)
            .unwrap();

        // Get the subdir definition
        let sub_definition = root_definitions
            .iter()
            .find(|d| d.file_path == sub_conftest_path)
            .unwrap();

        // Find references for the root definition
        let root_refs = db.find_references_for_definition(root_definition);

        // Should only include the test in the root directory
        assert_eq!(
            root_refs.len(),
            1,
            "Root definition should have 1 reference (from root test)"
        );
        assert_eq!(root_refs[0].file_path, root_test_path);

        // Find references for the subdir definition
        let sub_refs = db.find_references_for_definition(sub_definition);

        // Should include both tests in the subdirectory
        assert_eq!(
            sub_refs.len(),
            2,
            "Subdir definition should have 2 references (from subdir tests)"
        );

        let sub_ref_paths: Vec<_> = sub_refs.iter().map(|r| &r.file_path).collect();
        assert!(sub_ref_paths.contains(&&sub_test_path));
        assert!(sub_ref_paths.contains(&&sub_test2_path));

        // Verify that all references by name returns 3 total
        let all_refs = db.find_fixture_references("shared_fixture");
        assert_eq!(
            all_refs.len(),
            3,
            "Should find 3 total references across all scopes"
        );
    }

    #[test]
    fn test_multiline_parameters() {
        let db = FixtureDatabase::new();

        // Conftest with fixture
        let conftest_content = r#"
import pytest

@pytest.fixture
def foo():
    return 42
"#;
        let conftest_path = PathBuf::from("/tmp/test/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Test file with multiline parameters
        let test_content = r#"
def test_xxx(
    foo,
):
    assert foo == 42
"#;
        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Line 3 (1-indexed) is "    foo," - the parameter line
        // In LSP coordinates, that's line 2 (0-indexed)
        // Character position 4 is the 'f' in 'foo'

        // Debug: Check what usages were recorded
        if let Some(usages) = db.usages.get(&test_path) {
            println!("Usages recorded:");
            for usage in usages.iter() {
                println!("  {} at line {} (1-indexed)", usage.name, usage.line);
            }
        } else {
            println!("No usages recorded for test file");
        }

        // The content has a leading newline, so:
        // Line 1: (empty)
        // Line 2: def test_xxx(
        // Line 3:     foo,
        // Line 4: ):
        // Line 5:     assert foo == 42

        // foo is at line 3 (1-indexed) = line 2 (0-indexed LSP)
        let result = db.find_fixture_definition(&test_path, 2, 4);

        assert!(
            result.is_some(),
            "Should find fixture definition when cursor is on parameter line"
        );
        let def = result.unwrap();
        assert_eq!(def.name, "foo");
    }

    #[test]
    fn test_find_references_from_usage() {
        let db = FixtureDatabase::new();

        // Simple fixture and usage in the same file
        let test_content = r#"
import pytest

@pytest.fixture
def foo(): ...


def test_xxx(foo):
    pass
"#;
        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get the foo definition
        let foo_defs = db.definitions.get("foo").unwrap();
        assert_eq!(foo_defs.len(), 1, "Should have exactly one foo definition");
        let foo_def = &foo_defs[0];
        assert_eq!(foo_def.line, 5, "foo definition should be on line 5");

        // Get references for the definition
        let refs_from_def = db.find_references_for_definition(foo_def);
        println!("References from definition:");
        for r in &refs_from_def {
            println!("  {} at line {}", r.name, r.line);
        }

        assert_eq!(
            refs_from_def.len(),
            1,
            "Should find 1 usage reference (test_xxx parameter)"
        );
        assert_eq!(refs_from_def[0].line, 8, "Usage should be on line 8");

        // Now simulate what happens when user clicks on the usage (line 8, char 13 - the 'f' in 'foo')
        // This is LSP line 7 (0-indexed)
        let fixture_name = db.find_fixture_at_position(&test_path, 7, 13);
        println!(
            "\nfind_fixture_at_position(line 7, char 13): {:?}",
            fixture_name
        );

        assert_eq!(
            fixture_name,
            Some("foo".to_string()),
            "Should find fixture name at usage position"
        );

        let resolved_def = db.find_fixture_definition(&test_path, 7, 13);
        println!(
            "\nfind_fixture_definition(line 7, char 13): {:?}",
            resolved_def.as_ref().map(|d| (d.line, &d.file_path))
        );

        assert!(resolved_def.is_some(), "Should resolve usage to definition");
        assert_eq!(
            resolved_def.unwrap(),
            *foo_def,
            "Should resolve to the correct definition"
        );
    }

    #[test]
    fn test_find_references_with_ellipsis_body() {
        // This reproduces the structure from strawberry test_codegen.py
        let db = FixtureDatabase::new();

        let test_content = r#"@pytest.fixture
def foo(): ...


def test_xxx(foo):
    pass
"#;
        let test_path = PathBuf::from("/tmp/test/test_codegen.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check what line foo definition is on
        let foo_defs = db.definitions.get("foo");
        println!(
            "foo definitions: {:?}",
            foo_defs
                .as_ref()
                .map(|defs| defs.iter().map(|d| d.line).collect::<Vec<_>>())
        );

        // Check what line foo usage is on
        if let Some(usages) = db.usages.get(&test_path) {
            println!("usages:");
            for u in usages.iter() {
                println!("  {} at line {}", u.name, u.line);
            }
        }

        assert!(foo_defs.is_some(), "Should find foo definition");
        let foo_def = &foo_defs.unwrap()[0];

        // Get the usage line
        let usages = db.usages.get(&test_path).unwrap();
        let foo_usage = usages.iter().find(|u| u.name == "foo").unwrap();

        // Test from usage position (LSP coordinates are 0-indexed)
        let usage_lsp_line = (foo_usage.line - 1) as u32;
        println!("\nTesting from usage at LSP line {}", usage_lsp_line);

        let fixture_name = db.find_fixture_at_position(&test_path, usage_lsp_line, 13);
        assert_eq!(
            fixture_name,
            Some("foo".to_string()),
            "Should find foo at usage"
        );

        let def_from_usage = db.find_fixture_definition(&test_path, usage_lsp_line, 13);
        assert!(
            def_from_usage.is_some(),
            "Should resolve usage to definition"
        );
        assert_eq!(def_from_usage.unwrap(), *foo_def);
    }

    #[test]
    fn test_fixture_hierarchy_parent_references() {
        // Test that finding references from a parent fixture definition
        // includes child fixture definitions but NOT the child's usages
        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    """Parent fixture"""
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    """Child override that uses parent"""
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file in subdir using the child fixture
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/subdir/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get parent definition
        let parent_defs = db.definitions.get("cli_runner").unwrap();
        let parent_def = parent_defs
            .iter()
            .find(|d| d.file_path == parent_conftest)
            .unwrap();

        println!(
            "\nParent definition: {:?}:{}",
            parent_def.file_path, parent_def.line
        );

        // Find references for parent definition
        let refs = db.find_references_for_definition(parent_def);

        println!("\nReferences for parent definition:");
        for r in &refs {
            println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
        }

        // Parent references should include:
        // 1. The child fixture definition (line 5 in child conftest)
        // 2. The child's parameter that references the parent (line 5 in child conftest)
        // But NOT:
        // 3. test_one and test_two usages (they resolve to child, not parent)

        assert!(
            refs.len() <= 2,
            "Parent should have at most 2 references: child definition and its parameter, got {}",
            refs.len()
        );

        // Should include the child conftest
        let child_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == child_conftest)
            .collect();
        assert!(
            !child_refs.is_empty(),
            "Parent references should include child fixture definition"
        );

        // Should NOT include test file usages
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        assert!(
            test_refs.is_empty(),
            "Parent references should NOT include child's test file usages"
        );
    }

    #[test]
    fn test_fixture_hierarchy_child_references() {
        // Test that finding references from a child fixture definition
        // includes usages in the same directory (that resolve to the child)
        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file using child fixture
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/subdir/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get child definition
        let child_defs = db.definitions.get("cli_runner").unwrap();
        let child_def = child_defs
            .iter()
            .find(|d| d.file_path == child_conftest)
            .unwrap();

        println!(
            "\nChild definition: {:?}:{}",
            child_def.file_path, child_def.line
        );

        // Find references for child definition
        let refs = db.find_references_for_definition(child_def);

        println!("\nReferences for child definition:");
        for r in &refs {
            println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
        }

        // Child references should include test_one and test_two
        assert!(
            refs.len() >= 2,
            "Child should have at least 2 references from test file, got {}",
            refs.len()
        );

        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        assert_eq!(
            test_refs.len(),
            2,
            "Should have 2 references from test file"
        );
    }

    #[test]
    fn test_fixture_hierarchy_child_parameter_references() {
        // Test that finding references from a child fixture's parameter
        // (which references the parent) includes the child fixture definition
        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // When user clicks on the parameter "cli_runner" in the child definition,
        // it should resolve to the parent definition
        // Line 5 (1-indexed) = line 4 (0-indexed LSP), char 15 is in the parameter name
        let resolved_def = db.find_fixture_definition(&child_conftest, 4, 15);

        assert!(
            resolved_def.is_some(),
            "Child parameter should resolve to parent definition"
        );

        let def = resolved_def.unwrap();
        assert_eq!(
            def.file_path, parent_conftest,
            "Should resolve to parent conftest"
        );

        // Find references for parent definition
        let refs = db.find_references_for_definition(&def);

        println!("\nReferences for parent (from child parameter):");
        for r in &refs {
            println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
        }

        // Should include the child fixture's parameter usage
        let child_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == child_conftest)
            .collect();
        assert!(
            !child_refs.is_empty(),
            "Parent references should include child fixture parameter"
        );
    }

    #[test]
    fn test_fixture_hierarchy_usage_from_test() {
        // Test that finding references from a test function parameter
        // includes the definition it resolves to and other usages
        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/subdir/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file using child fixture
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass

def test_three(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/subdir/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Click on cli_runner in test_one (line 2, 1-indexed = line 1, 0-indexed)
        let resolved_def = db.find_fixture_definition(&test_path, 1, 13);

        assert!(
            resolved_def.is_some(),
            "Usage should resolve to child definition"
        );

        let def = resolved_def.unwrap();
        assert_eq!(
            def.file_path, child_conftest,
            "Should resolve to child conftest (not parent)"
        );

        // Find references for the resolved definition
        let refs = db.find_references_for_definition(&def);

        println!("\nReferences for child (from test usage):");
        for r in &refs {
            println!("  {} at {:?}:{}", r.name, r.file_path, r.line);
        }

        // Should include all three test usages
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
        assert_eq!(test_refs.len(), 3, "Should find all 3 usages in test file");
    }

    #[test]
    fn test_fixture_hierarchy_multiple_levels() {
        // Test a three-level hierarchy: grandparent -> parent -> child
        let db = FixtureDatabase::new();

        // Grandparent
        let grandparent_content = r#"
import pytest

@pytest.fixture
def db():
    return "grandparent_db"
"#;
        let grandparent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(grandparent_conftest.clone(), grandparent_content);

        // Parent overrides
        let parent_content = r#"
import pytest

@pytest.fixture
def db(db):
    return f"parent_{db}"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/api/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child overrides again
        let child_content = r#"
import pytest

@pytest.fixture
def db(db):
    return f"child_{db}"
"#;
        let child_conftest = PathBuf::from("/tmp/project/api/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file at child level
        let test_content = r#"
def test_db(db):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/api/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get all definitions
        let all_defs = db.definitions.get("db").unwrap();
        assert_eq!(all_defs.len(), 3, "Should have 3 definitions");

        let grandparent_def = all_defs
            .iter()
            .find(|d| d.file_path == grandparent_conftest)
            .unwrap();
        let parent_def = all_defs
            .iter()
            .find(|d| d.file_path == parent_conftest)
            .unwrap();
        let child_def = all_defs
            .iter()
            .find(|d| d.file_path == child_conftest)
            .unwrap();

        // Test from test file - should resolve to child
        let resolved = db.find_fixture_definition(&test_path, 1, 12);
        assert_eq!(
            resolved.as_ref(),
            Some(child_def),
            "Test should use child definition"
        );

        // Child's references should include test file
        let child_refs = db.find_references_for_definition(child_def);
        let test_refs: Vec<_> = child_refs
            .iter()
            .filter(|r| r.file_path == test_path)
            .collect();
        assert!(
            !test_refs.is_empty(),
            "Child should have test file references"
        );

        // Parent's references should include child's parameter, but not test file
        let parent_refs = db.find_references_for_definition(parent_def);
        let child_param_refs: Vec<_> = parent_refs
            .iter()
            .filter(|r| r.file_path == child_conftest)
            .collect();
        let test_refs_in_parent: Vec<_> = parent_refs
            .iter()
            .filter(|r| r.file_path == test_path)
            .collect();

        assert!(
            !child_param_refs.is_empty(),
            "Parent should have child parameter reference"
        );
        assert!(
            test_refs_in_parent.is_empty(),
            "Parent should NOT have test file references"
        );

        // Grandparent's references should include parent's parameter, but not child's stuff
        let grandparent_refs = db.find_references_for_definition(grandparent_def);
        let parent_param_refs: Vec<_> = grandparent_refs
            .iter()
            .filter(|r| r.file_path == parent_conftest)
            .collect();
        let child_refs_in_gp: Vec<_> = grandparent_refs
            .iter()
            .filter(|r| r.file_path == child_conftest)
            .collect();

        assert!(
            !parent_param_refs.is_empty(),
            "Grandparent should have parent parameter reference"
        );
        assert!(
            child_refs_in_gp.is_empty(),
            "Grandparent should NOT have child references"
        );
    }

    #[test]
    fn test_fixture_hierarchy_same_file_override() {
        // Test that a fixture can be overridden in the same file
        // (less common but valid pytest pattern)
        let db = FixtureDatabase::new();

        let content = r#"
import pytest

@pytest.fixture
def base():
    return "base"

@pytest.fixture
def base(base):
    return f"override_{base}"

def test_uses_override(base):
    pass
"#;
        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), content);

        let defs = db.definitions.get("base").unwrap();
        assert_eq!(defs.len(), 2, "Should have 2 definitions in same file");

        println!("\nDefinitions found:");
        for d in defs.iter() {
            println!("  base at line {}", d.line);
        }

        // Check usages
        if let Some(usages) = db.usages.get(&test_path) {
            println!("\nUsages found:");
            for u in usages.iter() {
                println!("  {} at line {}", u.name, u.line);
            }
        } else {
            println!("\nNo usages found!");
        }

        // The test should resolve to the second definition (override)
        // Line 12 (1-indexed) = line 11 (0-indexed LSP)
        // Character position: "def test_uses_override(base):" - 'b' is at position 23
        let resolved = db.find_fixture_definition(&test_path, 11, 23);

        println!("\nResolved: {:?}", resolved.as_ref().map(|d| d.line));

        assert!(resolved.is_some(), "Should resolve to override definition");

        // The second definition should be at line 9 (1-indexed)
        let override_def = defs.iter().find(|d| d.line == 9).unwrap();
        println!("Override def at line: {}", override_def.line);
        assert_eq!(resolved.as_ref(), Some(override_def));
    }

    #[test]
    fn test_cursor_position_on_definition_line() {
        // Debug test to understand what happens at different cursor positions
        // on a fixture definition line with a self-referencing parameter
        let db = FixtureDatabase::new();

        // Add a parent conftest with parent fixture
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        let content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let test_path = PathBuf::from("/tmp/test/test_example.py");
        db.analyze_file(test_path.clone(), content);

        // Line 5 (1-indexed): "def cli_runner(cli_runner):"
        // Position 0: 'd' in def
        // Position 4: 'c' in cli_runner (function name)
        // Position 15: '('
        // Position 16: 'c' in cli_runner (parameter name)

        println!("\n=== Testing character positions on line 5 ===");

        // Check usages
        if let Some(usages) = db.usages.get(&test_path) {
            println!("\nUsages found:");
            for u in usages.iter() {
                println!(
                    "  {} at line {}, chars {}-{}",
                    u.name, u.line, u.start_char, u.end_char
                );
            }
        } else {
            println!("\nNo usages found!");
        }

        // Test clicking on function name 'cli_runner' - should be treated as definition
        let line_content = "def cli_runner(cli_runner):";
        println!("\nLine content: '{}'", line_content);

        // Position 4 = 'c' in function name cli_runner
        println!("\nPosition 4 (function name):");
        let word_at_4 = db.extract_word_at_position(line_content, 4);
        println!("  Word at cursor: {:?}", word_at_4);
        let fixture_name_at_4 = db.find_fixture_at_position(&test_path, 4, 4);
        println!("  find_fixture_at_position: {:?}", fixture_name_at_4);
        let resolved_4 = db.find_fixture_definition(&test_path, 4, 4); // Line 5 = index 4
        println!(
            "  Resolved: {:?}",
            resolved_4.as_ref().map(|d| (d.name.as_str(), d.line))
        );

        // Position 16 = 'c' in parameter name cli_runner
        println!("\nPosition 16 (parameter name):");
        let word_at_16 = db.extract_word_at_position(line_content, 16);
        println!("  Word at cursor: {:?}", word_at_16);

        // Manual check: does the usage check work?
        if let Some(usages) = db.usages.get(&test_path) {
            for usage in usages.iter() {
                println!("  Checking usage: {} at line {}", usage.name, usage.line);
                if usage.line == 5 && usage.name == "cli_runner" {
                    println!("    MATCH! Usage matches our position");
                }
            }
        }

        let fixture_name_at_16 = db.find_fixture_at_position(&test_path, 4, 16);
        println!("  find_fixture_at_position: {:?}", fixture_name_at_16);
        let resolved_16 = db.find_fixture_definition(&test_path, 4, 16); // Line 5 = index 4
        println!(
            "  Resolved: {:?}",
            resolved_16.as_ref().map(|d| (d.name.as_str(), d.line))
        );

        // Expected behavior:
        // - Position 4 (function name): should resolve to CHILD (line 5) - we're ON the definition
        // - Position 16 (parameter): should resolve to PARENT (line 5 in conftest) - it's a usage

        assert_eq!(word_at_4, Some("cli_runner".to_string()));
        assert_eq!(word_at_16, Some("cli_runner".to_string()));

        // Check the actual resolution
        println!("\n=== ACTUAL vs EXPECTED ===");
        println!("Position 4 (function name):");
        println!(
            "  Actual: {:?}",
            resolved_4.as_ref().map(|d| (&d.file_path, d.line))
        );
        println!("  Expected: test file, line 5 (the child definition itself)");

        println!("\nPosition 16 (parameter):");
        println!(
            "  Actual: {:?}",
            resolved_16.as_ref().map(|d| (&d.file_path, d.line))
        );
        println!("  Expected: conftest, line 5 (the parent definition)");

        // The BUG: both return the same thing (child at line 5)
        // Position 4: returning child is CORRECT (though find_fixture_definition returns None,
        //             main.rs falls back to get_definition_at_line which is correct)
        // Position 16: returning child is WRONG - should return parent (line 5 in conftest)

        if let Some(ref def) = resolved_16 {
            assert_eq!(
                def.file_path, parent_conftest,
                "Parameter should resolve to parent definition"
            );
        } else {
            panic!("Position 16 (parameter) should resolve to parent definition");
        }
    }

    #[test]
    fn test_undeclared_fixture_detection_in_test() {
        let db = FixtureDatabase::new();

        // Add a fixture definition in conftest
        let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
        let conftest_path = PathBuf::from("/tmp/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Add a test that uses the fixture without declaring it
        let test_content = r#"
def test_example():
    result = my_fixture.get()
    assert result == 42
"#;
        let test_path = PathBuf::from("/tmp/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that undeclared fixture was detected
        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert_eq!(undeclared.len(), 1, "Should detect one undeclared fixture");

        let fixture = &undeclared[0];
        assert_eq!(fixture.name, "my_fixture");
        assert_eq!(fixture.function_name, "test_example");
        assert_eq!(fixture.line, 3); // Line 3: "result = my_fixture.get()"
    }

    #[test]
    fn test_undeclared_fixture_detection_in_fixture() {
        let db = FixtureDatabase::new();

        // Add fixture definitions in conftest
        let conftest_content = r#"
import pytest

@pytest.fixture
def base_fixture():
    return "base"

@pytest.fixture
def helper_fixture():
    return "helper"
"#;
        let conftest_path = PathBuf::from("/tmp/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Add a fixture that uses another fixture without declaring it
        let test_content = r#"
import pytest

@pytest.fixture
def my_fixture(base_fixture):
    data = helper_fixture.value
    return f"{base_fixture}-{data}"
"#;
        let test_path = PathBuf::from("/tmp/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that undeclared fixture was detected
        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert_eq!(undeclared.len(), 1, "Should detect one undeclared fixture");

        let fixture = &undeclared[0];
        assert_eq!(fixture.name, "helper_fixture");
        assert_eq!(fixture.function_name, "my_fixture");
        assert_eq!(fixture.line, 6); // Line 6: "data = helper_fixture.value"
    }

    #[test]
    fn test_no_false_positive_for_declared_fixtures() {
        let db = FixtureDatabase::new();

        // Add a fixture definition in conftest
        let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
        let conftest_path = PathBuf::from("/tmp/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Add a test that properly declares the fixture as a parameter
        let test_content = r#"
def test_example(my_fixture):
    result = my_fixture
    assert result == 42
"#;
        let test_path = PathBuf::from("/tmp/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that no undeclared fixtures were detected
        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert_eq!(
            undeclared.len(),
            0,
            "Should not detect any undeclared fixtures"
        );
    }

    #[test]
    fn test_no_false_positive_for_non_fixtures() {
        let db = FixtureDatabase::new();

        // Add a test that uses regular variables (not fixtures)
        let test_content = r#"
def test_example():
    my_variable = 42
    result = my_variable + 10
    assert result == 52
"#;
        let test_path = PathBuf::from("/tmp/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Check that no undeclared fixtures were detected
        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert_eq!(
            undeclared.len(),
            0,
            "Should not detect any undeclared fixtures"
        );
    }

    #[test]
    fn test_undeclared_fixture_not_available_in_hierarchy() {
        let db = FixtureDatabase::new();

        // Add a fixture in a different directory (not in hierarchy)
        let other_conftest = r#"
import pytest

@pytest.fixture
def other_fixture():
    return "other"
"#;
        let other_path = PathBuf::from("/other/conftest.py");
        db.analyze_file(other_path.clone(), other_conftest);

        // Add a test that uses a name that happens to match a fixture but isn't available
        let test_content = r#"
def test_example():
    result = other_fixture.value
    assert result == "other"
"#;
        let test_path = PathBuf::from("/tmp/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Should not detect undeclared fixture because it's not in the hierarchy
        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert_eq!(
            undeclared.len(),
            0,
            "Should not detect fixtures not in hierarchy"
        );
    }
}
