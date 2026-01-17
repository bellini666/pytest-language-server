//! File analysis and AST parsing for fixture extraction.
//!
//! This module contains the core logic for parsing Python files and extracting
//! fixture definitions and usages. Docstring extraction is in `docstring.rs`
//! and undeclared fixture scanning is in `undeclared.rs`.

use super::decorators;
use super::types::{FixtureDefinition, FixtureScope, FixtureUsage};
use super::FixtureDatabase;
use rustpython_parser::ast::{ArgWithDefault, Arguments, Expr, Stmt};
use rustpython_parser::{parse, Mode};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

impl FixtureDatabase {
    /// Analyze a Python file for fixtures and usages.
    /// This is the public API - it cleans up previous definitions before analyzing.
    pub fn analyze_file(&self, file_path: PathBuf, content: &str) {
        self.analyze_file_internal(file_path, content, true);
    }

    /// Analyze a file without cleaning up previous definitions.
    /// Used during initial workspace scan when we know the database is empty.
    pub(crate) fn analyze_file_fresh(&self, file_path: PathBuf, content: &str) {
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
            .insert(file_path.clone(), std::sync::Arc::new(content.to_string()));

        // Parse the Python code
        let parsed = match parse(content, Mode::Module, "") {
            Ok(ast) => ast,
            Err(e) => {
                // Keep existing fixture data when parse fails (user is likely editing)
                // This provides better LSP experience during editing with syntax errors
                debug!(
                    "Failed to parse Python file {:?}: {} - keeping previous data",
                    file_path, e
                );
                return;
            }
        };

        // Clear previous usages for this file (only after successful parse)
        self.cleanup_usages_for_file(&file_path);
        self.usages.remove(&file_path);

        // Clear previous undeclared fixtures for this file
        self.undeclared_fixtures.remove(&file_path);

        // Clear previous imports for this file
        self.imports.remove(&file_path);

        // Note: line_index_cache uses content-hash-based invalidation,
        // so we don't need to clear it here - get_line_index will detect
        // if the content has changed and rebuild if necessary.

        // Clear previous fixture definitions from this file (only when re-analyzing)
        // Skip this during initial workspace scan for performance
        if cleanup_previous {
            self.cleanup_definitions_for_file(&file_path);
        }

        // Check if this is a conftest.py
        let is_conftest = file_path
            .file_name()
            .map(|n| n == "conftest.py")
            .unwrap_or(false);
        debug!("is_conftest: {}", is_conftest);

        // Get or build line index for O(1) line lookups (cached for performance)
        let line_index = self.get_line_index(&file_path, content);

        // Process each statement in the module
        if let rustpython_parser::ast::Mod::Module(module) = parsed {
            debug!("Module has {} statements", module.body.len());

            // First pass: collect all module-level names (imports, assignments, function/class defs)
            let mut module_level_names = HashSet::new();
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

        // Periodically evict cache entries to prevent unbounded memory growth
        self.evict_cache_if_needed();
    }

    /// Remove definitions that were in a specific file.
    /// Uses the file_definitions reverse index for efficient O(m) cleanup
    /// where m = number of fixtures in this file, rather than O(n) where
    /// n = total number of unique fixture names.
    ///
    /// Deadlock-free design:
    /// 1. Atomically remove the set of fixture names from file_definitions
    /// 2. For each fixture name, get a mutable reference, modify, then drop
    /// 3. Only after dropping the reference, remove empty entries
    fn cleanup_definitions_for_file(&self, file_path: &PathBuf) {
        // Step 1: Atomically remove and get the fixture names for this file
        let fixture_names = match self.file_definitions.remove(file_path) {
            Some((_, names)) => names,
            None => return, // No fixtures defined in this file
        };

        // Step 2: For each fixture name, remove definitions from this file
        for fixture_name in fixture_names {
            let should_remove = {
                // Get mutable reference, modify in place, check if empty
                if let Some(mut defs) = self.definitions.get_mut(&fixture_name) {
                    defs.retain(|def| def.file_path != *file_path);
                    defs.is_empty()
                } else {
                    false
                }
            }; // RefMut dropped here - safe to call remove_if now

            // Step 3: Remove empty entries atomically
            if should_remove {
                // Use remove_if to ensure we only remove if still empty
                // (another thread might have added a definition)
                self.definitions
                    .remove_if(&fixture_name, |_, defs| defs.is_empty());
            }
        }
    }

    /// Remove usages from the usage_by_fixture reverse index for a specific file.
    /// Called before re-analyzing a file to clean up stale entries.
    ///
    /// Collects all keys first (without filtering) to avoid holding read locks
    /// while doing the filter check, which could cause deadlocks.
    fn cleanup_usages_for_file(&self, file_path: &PathBuf) {
        // Collect all keys first to avoid holding any locks during iteration
        let all_keys: Vec<String> = self
            .usage_by_fixture
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // Process each key - check if it has usages from this file and clean up
        for fixture_name in all_keys {
            let should_remove = {
                if let Some(mut usages) = self.usage_by_fixture.get_mut(&fixture_name) {
                    let had_usages = usages.iter().any(|(path, _)| path == file_path);
                    if had_usages {
                        usages.retain(|(path, _)| path != file_path);
                    }
                    usages.is_empty()
                } else {
                    false
                }
            };

            if should_remove {
                self.usage_by_fixture
                    .remove_if(&fixture_name, |_, usages| usages.is_empty());
            }
        }
    }

    /// Build an index of line start offsets for O(1) line number lookups.
    /// Uses memchr for SIMD-accelerated newline searching.
    pub(crate) fn build_line_index(content: &str) -> Vec<usize> {
        let bytes = content.as_bytes();
        let mut line_index = Vec::with_capacity(content.len() / 30);
        line_index.push(0);
        for i in memchr::memchr_iter(b'\n', bytes) {
            line_index.push(i + 1);
        }
        line_index
    }

    /// Get line number (1-based) from byte offset
    pub(crate) fn get_line_from_offset(&self, offset: usize, line_index: &[usize]) -> usize {
        match line_index.binary_search(&offset) {
            Ok(line) => line + 1,
            Err(line) => line,
        }
    }

    /// Get character position within a line from byte offset
    pub(crate) fn get_char_position_from_offset(
        &self,
        offset: usize,
        line_index: &[usize],
    ) -> usize {
        let line = self.get_line_from_offset(offset, line_index);
        let line_start = line_index[line - 1];
        offset.saturating_sub(line_start)
    }

    /// Returns an iterator over all function arguments including positional-only,
    /// regular positional, and keyword-only arguments.
    /// This is needed because pytest fixtures can be declared as any of these types.
    pub(crate) fn all_args(args: &Arguments) -> impl Iterator<Item = &ArgWithDefault> {
        args.posonlyargs
            .iter()
            .chain(args.args.iter())
            .chain(args.kwonlyargs.iter())
    }

    /// Helper to record a fixture usage in the database.
    /// Reduces code duplication across multiple call sites.
    /// Also maintains usage_by_fixture reverse index for efficient reference lookups.
    fn record_fixture_usage(
        &self,
        file_path: &Path,
        fixture_name: String,
        line: usize,
        start_char: usize,
        end_char: usize,
    ) {
        let file_path_buf = file_path.to_path_buf();
        let usage = FixtureUsage {
            name: fixture_name.clone(),
            file_path: file_path_buf.clone(),
            line,
            start_char,
            end_char,
        };

        // Add to per-file usages map
        self.usages
            .entry(file_path_buf.clone())
            .or_default()
            .push(usage.clone());

        // Add to reverse index for efficient reference lookups
        self.usage_by_fixture
            .entry(fixture_name)
            .or_default()
            .push((file_path_buf, usage));
    }

    /// Helper to record a fixture definition in the database.
    /// Also maintains the file_definitions reverse index for efficient cleanup.
    fn record_fixture_definition(&self, definition: FixtureDefinition) {
        let file_path = definition.file_path.clone();
        let fixture_name = definition.name.clone();

        // Add to main definitions map
        self.definitions
            .entry(fixture_name.clone())
            .or_default()
            .push(definition);

        // Maintain reverse index for efficient cleanup
        self.file_definitions
            .entry(file_path)
            .or_default()
            .insert(fixture_name);

        // Invalidate cycle cache since definitions changed
        self.invalidate_cycle_cache();
    }

    /// Visit a statement and extract fixture definitions and usages
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
                let usefixtures = decorators::extract_usefixtures_names(decorator);
                for (fixture_name, range) in usefixtures {
                    let usage_line =
                        self.get_line_from_offset(range.start().to_usize(), line_index);
                    let start_char =
                        self.get_char_position_from_offset(range.start().to_usize(), line_index);
                    let end_char =
                        self.get_char_position_from_offset(range.end().to_usize(), line_index);

                    info!(
                        "Found usefixtures usage on class: {} at {:?}:{}:{}",
                        fixture_name, file_path, usage_line, start_char
                    );

                    self.record_fixture_usage(
                        file_path,
                        fixture_name,
                        usage_line,
                        start_char + 1,
                        end_char - 1,
                    );
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
            let usefixtures = decorators::extract_usefixtures_names(decorator);
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

                self.record_fixture_usage(
                    file_path,
                    fixture_name,
                    usage_line,
                    start_char + 1,
                    end_char - 1,
                );
            }
        }

        // Check for @pytest.mark.parametrize with indirect=True on the function
        for decorator in decorator_list {
            let indirect_fixtures = decorators::extract_parametrize_indirect_fixtures(decorator);
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

                self.record_fixture_usage(
                    file_path,
                    fixture_name,
                    usage_line,
                    start_char + 1,
                    end_char - 1,
                );
            }
        }

        // Check if this is a fixture definition
        debug!(
            "Function {} has {} decorators",
            func_name,
            decorator_list.len()
        );
        let fixture_decorator = decorator_list
            .iter()
            .find(|dec| decorators::is_fixture_decorator(dec));

        if let Some(decorator) = fixture_decorator {
            debug!("  Decorator matched as fixture!");

            // Check if the fixture has a custom name
            let fixture_name = decorators::extract_fixture_name_from_decorator(decorator)
                .unwrap_or_else(|| func_name.to_string());

            // Extract scope from decorator (defaults to function scope)
            let scope = decorators::extract_fixture_scope(decorator).unwrap_or_default();

            let line = self.get_line_from_offset(range.start().to_usize(), line_index);
            let docstring = self.extract_docstring(body);
            let return_type = self.extract_return_type(returns, body, content);

            info!(
                "Found fixture definition: {} (function: {}, scope: {:?}) at {:?}:{}",
                fixture_name, func_name, scope, file_path, line
            );

            let (start_char, end_char) = self.find_function_name_position(content, line, func_name);

            let is_third_party = file_path.to_string_lossy().contains("site-packages");

            // Fixtures can depend on other fixtures - collect dependencies first
            let mut declared_params: HashSet<String> = HashSet::new();
            let mut dependencies: Vec<String> = Vec::new();
            declared_params.insert("self".to_string());
            declared_params.insert("request".to_string());
            declared_params.insert(func_name.to_string());

            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());
                // Track as dependency if it's not self/request (these are special)
                if arg_name != "self" && arg_name != "request" {
                    dependencies.push(arg_name.to_string());
                }
            }

            // Calculate end line from the function's range
            let end_line = self.get_line_from_offset(range.end().to_usize(), line_index);

            let definition = FixtureDefinition {
                name: fixture_name.clone(),
                file_path: file_path.clone(),
                line,
                end_line,
                start_char,
                end_char,
                docstring,
                return_type,
                is_third_party,
                dependencies: dependencies.clone(),
                scope,
                yield_line: self.find_yield_line(body, line_index),
            };

            self.record_fixture_definition(definition);

            // Record each dependency as a usage
            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();

                if arg_name != "self" && arg_name != "request" {
                    let arg_line =
                        self.get_line_from_offset(arg.def.range.start().to_usize(), line_index);
                    let start_char = self.get_char_position_from_offset(
                        arg.def.range.start().to_usize(),
                        line_index,
                    );
                    // Use parameter name length, not AST range (which includes type annotation)
                    let end_char = start_char + arg_name.len();

                    info!(
                        "Found fixture dependency: {} at {:?}:{}:{}",
                        arg_name, file_path, arg_line, start_char
                    );

                    self.record_fixture_usage(
                        file_path,
                        arg_name.to_string(),
                        arg_line,
                        start_char,
                        end_char,
                    );
                }
            }

            let function_line = self.get_line_from_offset(range.start().to_usize(), line_index);
            self.scan_function_body_for_undeclared_fixtures(
                body,
                file_path,
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

            let mut declared_params: HashSet<String> = HashSet::new();
            declared_params.insert("self".to_string());
            declared_params.insert("request".to_string());

            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" {
                    let arg_offset = arg.def.range.start().to_usize();
                    let arg_line = self.get_line_from_offset(arg_offset, line_index);
                    let start_char = self.get_char_position_from_offset(arg_offset, line_index);
                    // Use parameter name length, not AST range (which includes type annotation)
                    let end_char = start_char + arg_name.len();

                    debug!(
                        "Parameter {} at offset {}, calculated line {}, char {}",
                        arg_name, arg_offset, arg_line, start_char
                    );
                    info!(
                        "Found fixture usage: {} at {:?}:{}:{}",
                        arg_name, file_path, arg_line, start_char
                    );

                    self.record_fixture_usage(
                        file_path,
                        arg_name.to_string(),
                        arg_line,
                        start_char,
                        end_char,
                    );
                }
            }

            let function_line = self.get_line_from_offset(range.start().to_usize(), line_index);
            self.scan_function_body_for_undeclared_fixtures(
                body,
                file_path,
                line_index,
                &declared_params,
                func_name,
                function_line,
            );
        }
    }

    /// Handle assignment-style fixtures: fixture_name = pytest.fixture()(func)
    fn visit_assignment_fixture(
        &self,
        assign: &rustpython_parser::ast::StmtAssign,
        file_path: &PathBuf,
        _content: &str,
        line_index: &[usize],
    ) {
        if let Expr::Call(outer_call) = &*assign.value {
            if let Expr::Call(inner_call) = &*outer_call.func {
                if decorators::is_fixture_decorator(&inner_call.func) {
                    for target in &assign.targets {
                        if let Expr::Name(name) = target {
                            let fixture_name = name.id.as_str();
                            let line = self
                                .get_line_from_offset(assign.range.start().to_usize(), line_index);

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

                            let is_third_party =
                                file_path.to_string_lossy().contains("site-packages");
                            let definition = FixtureDefinition {
                                name: fixture_name.to_string(),
                                file_path: file_path.clone(),
                                line,
                                end_line: line, // Assignment-style fixtures are single-line
                                start_char,
                                end_char,
                                docstring: None,
                                return_type: None,
                                is_third_party,
                                dependencies: Vec::new(), // Assignment-style fixtures don't have explicit dependencies
                                scope: FixtureScope::default(), // Assignment-style fixtures default to function scope
                                yield_line: None, // Assignment-style fixtures don't have yield statements
                            };

                            self.record_fixture_definition(definition);
                        }
                    }
                }
            }
        }
    }
}

// Second impl block for additional analyzer methods
impl FixtureDatabase {
    // ============ Module-level name collection ============

    /// Collect all module-level names (imports, assignments, function/class defs)
    fn collect_module_level_names(&self, stmt: &Stmt, names: &mut HashSet<String>) {
        match stmt {
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    let name = alias.asname.as_ref().unwrap_or(&alias.name);
                    names.insert(name.to_string());
                }
            }
            Stmt::ImportFrom(import_from) => {
                for alias in &import_from.names {
                    let name = alias.asname.as_ref().unwrap_or(&alias.name);
                    names.insert(name.to_string());
                }
            }
            Stmt::FunctionDef(func_def) => {
                let is_fixture = func_def
                    .decorator_list
                    .iter()
                    .any(decorators::is_fixture_decorator);
                if !is_fixture {
                    names.insert(func_def.name.to_string());
                }
            }
            Stmt::AsyncFunctionDef(func_def) => {
                let is_fixture = func_def
                    .decorator_list
                    .iter()
                    .any(decorators::is_fixture_decorator);
                if !is_fixture {
                    names.insert(func_def.name.to_string());
                }
            }
            Stmt::ClassDef(class_def) => {
                names.insert(class_def.name.to_string());
            }
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
    pub(crate) fn collect_names_from_expr(&self, expr: &Expr, names: &mut HashSet<String>) {
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

    // Docstring and return type extraction methods are in docstring.rs

    /// Find the character position of a function name in a line
    fn find_function_name_position(
        &self,
        content: &str,
        line: usize,
        func_name: &str,
    ) -> (usize, usize) {
        super::string_utils::find_function_name_position(content, line, func_name)
    }

    /// Find the line number of the first yield statement in a function body.
    /// Returns None if no yield statement is found.
    fn find_yield_line(&self, body: &[Stmt], line_index: &[usize]) -> Option<usize> {
        for stmt in body {
            if let Some(line) = self.find_yield_in_stmt(stmt, line_index) {
                return Some(line);
            }
        }
        None
    }

    /// Recursively search for yield statements in a statement.
    fn find_yield_in_stmt(&self, stmt: &Stmt, line_index: &[usize]) -> Option<usize> {
        match stmt {
            Stmt::Expr(expr_stmt) => self.find_yield_in_expr(&expr_stmt.value, line_index),
            Stmt::If(if_stmt) => {
                // Check body
                for s in &if_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                // Check elif/else
                for s in &if_stmt.orelse {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            Stmt::With(with_stmt) => {
                for s in &with_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            Stmt::AsyncWith(with_stmt) => {
                for s in &with_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            Stmt::Try(try_stmt) => {
                for s in &try_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                for handler in &try_stmt.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = handler;
                    for s in &h.body {
                        if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                            return Some(line);
                        }
                    }
                }
                for s in &try_stmt.orelse {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                for s in &try_stmt.finalbody {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            Stmt::For(for_stmt) => {
                for s in &for_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                for s in &for_stmt.orelse {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            Stmt::AsyncFor(for_stmt) => {
                for s in &for_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                for s in &for_stmt.orelse {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            Stmt::While(while_stmt) => {
                for s in &while_stmt.body {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                for s in &while_stmt.orelse {
                    if let Some(line) = self.find_yield_in_stmt(s, line_index) {
                        return Some(line);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Find yield expression and return its line number.
    fn find_yield_in_expr(&self, expr: &Expr, line_index: &[usize]) -> Option<usize> {
        match expr {
            Expr::Yield(yield_expr) => {
                let line =
                    self.get_line_from_offset(yield_expr.range.start().to_usize(), line_index);
                Some(line)
            }
            Expr::YieldFrom(yield_from) => {
                let line =
                    self.get_line_from_offset(yield_from.range.start().to_usize(), line_index);
                Some(line)
            }
            _ => None,
        }
    }
}

// Undeclared fixtures scanning methods are in undeclared.rs
