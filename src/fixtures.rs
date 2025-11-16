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
}

#[derive(Debug)]
pub struct FixtureDatabase {
    // Map from fixture name to all its definitions (can be in multiple conftest.py files)
    definitions: Arc<DashMap<String, Vec<FixtureDefinition>>>,
    // Map from file path to fixtures used in that file
    usages: Arc<DashMap<PathBuf, Vec<FixtureUsage>>>,
    // Cache of file contents for analyzed files (mainly for testing)
    file_cache: Arc<DashMap<PathBuf, String>>,
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
            let func_line = self.get_line_from_offset(range.start().to_usize(), content);
            for arg in &args.args {
                let arg_name = arg.def.arg.as_str();
                if arg_name != "self" && arg_name != "request" {
                    info!(
                        "Found fixture dependency: {} at {:?}:{}",
                        arg_name, file_path, func_line
                    );

                    let usage = FixtureUsage {
                        name: arg_name.to_string(),
                        file_path: file_path.clone(),
                        line: func_line,
                    };

                    self.usages
                        .entry(file_path.clone())
                        .or_default()
                        .push(usage);
                }
            }
        }

        // Check if this is a test function
        if func_name.starts_with("test_") {
            debug!("Found test function: {}", func_name);
            // Use the function definition line for all parameters
            // This way when user is on the function def line, we find the fixtures
            let func_line = self.get_line_from_offset(range.start().to_usize(), content);

            // Extract fixture usages from function parameters
            for arg in &args.args {
                let arg_name = arg.def.arg.as_str();
                if arg_name != "self" {
                    info!(
                        "Found fixture usage: {} at {:?}:{}",
                        arg_name, file_path, func_line
                    );

                    let usage = FixtureUsage {
                        name: arg_name.to_string(),
                        file_path: file_path.clone(),
                        line: func_line, // Use function line, not parameter line
                    };

                    // Append to existing usages for this file
                    self.usages
                        .entry(file_path.clone())
                        .or_default()
                        .push(usage);
                }
            }
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
        content[..offset].lines().count() + 1 // 1-based line numbers
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

        // First, check if this word matches any fixture usage on this line
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line && usage.name == word_at_cursor {
                    info!("Found fixture usage at cursor position: {}", usage.name);
                    // Find the closest definition for this fixture
                    return self.find_closest_definition(file_path, &usage.name);
                }
            }
        }

        debug!("Word at cursor '{}' is not a fixture usage", word_at_cursor);
        None
    }

    fn find_closest_definition(
        &self,
        file_path: &Path,
        fixture_name: &str,
    ) -> Option<FixtureDefinition> {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Check if fixture is defined in the same file (highest priority)
        debug!(
            "Checking for fixture {} in same file: {:?}",
            fixture_name, file_path
        );
        for def in definitions.iter() {
            if def.file_path == file_path {
                info!(
                    "Found fixture {} in same file (highest priority)",
                    fixture_name
                );
                return Some(def.clone());
            }
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
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line {
                    if let Some(ref word) = word_at_cursor {
                        if word == &usage.name {
                            info!("Found fixture usage at cursor position: {}", usage.name);
                            return Some(usage.name.clone());
                        }
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
}
