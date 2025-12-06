//! File analysis and AST parsing for fixture extraction.
//!
//! This module contains all the logic for parsing Python files and extracting
//! fixture definitions, usages, and undeclared fixtures.

use super::decorators;
use super::types::{FixtureDefinition, FixtureUsage, UndeclaredFixture};
use super::FixtureDatabase;
use rustpython_parser::ast::{ArgWithDefault, Arguments, Expr, Stmt};
use rustpython_parser::{parse, Mode};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};

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
    }

    /// Remove definitions that were in a specific file
    fn cleanup_definitions_for_file(&self, file_path: &PathBuf) {
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
                .filter(|def| def.file_path != *file_path)
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

    /// Build an index of line start offsets for O(1) line number lookups
    pub(crate) fn build_line_index(content: &str) -> Vec<usize> {
        let mut line_index = Vec::with_capacity(content.len() / 30);
        line_index.push(0);
        for (i, c) in content.char_indices() {
            if c == '\n' {
                line_index.push(i + 1);
            }
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
            name: fixture_name,
            file_path: file_path_buf.clone(),
            line,
            start_char,
            end_char,
        };
        self.usages.entry(file_path_buf).or_default().push(usage);
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

            let line = self.get_line_from_offset(range.start().to_usize(), line_index);
            let docstring = self.extract_docstring(body);
            let return_type = self.extract_return_type(returns, body, content);

            info!(
                "Found fixture definition: {} (function: {}) at {:?}:{}",
                fixture_name, func_name, file_path, line
            );

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
            let mut declared_params: HashSet<String> = HashSet::new();
            declared_params.insert("self".to_string());
            declared_params.insert("request".to_string());
            declared_params.insert(func_name.to_string());

            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();
                declared_params.insert(arg_name.to_string());

                if arg_name != "self" && arg_name != "request" {
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
                content,
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
    fn collect_names_from_expr(&self, expr: &Expr, names: &mut HashSet<String>) {
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

    // ============ Docstring and return type extraction ============

    fn extract_docstring(&self, body: &[Stmt]) -> Option<String> {
        if let Some(Stmt::Expr(expr_stmt)) = body.first() {
            if let Expr::Constant(constant) = &*expr_stmt.value {
                if let rustpython_parser::ast::Constant::Str(s) = &constant.value {
                    return Some(self.format_docstring(s.to_string()));
                }
            }
        }
        None
    }

    fn format_docstring(&self, docstring: String) -> String {
        super::string_utils::format_docstring(docstring)
    }

    fn extract_return_type(
        &self,
        returns: &Option<Box<rustpython_parser::ast::Expr>>,
        body: &[Stmt],
        content: &str,
    ) -> Option<String> {
        if let Some(return_expr) = returns {
            let has_yield = self.contains_yield(body);

            if has_yield {
                return self.extract_yielded_type(return_expr, content);
            } else {
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
        if let Expr::Subscript(subscript) = expr {
            let _base_name = self.expr_to_string(&subscript.value, content);

            if let Expr::Tuple(tuple) = &*subscript.slice {
                if let Some(first_elem) = tuple.elts.first() {
                    return Some(self.expr_to_string(first_elem, content));
                }
            } else {
                return Some(self.expr_to_string(&subscript.slice, content));
            }
        }

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
                format!(
                    "{} | {}",
                    self.expr_to_string(&binop.left, content),
                    self.expr_to_string(&binop.right, content)
                )
            }
            _ => "Any".to_string(),
        }
    }

    /// Find the character position of a function name in a line
    fn find_function_name_position(
        &self,
        content: &str,
        line: usize,
        func_name: &str,
    ) -> (usize, usize) {
        super::string_utils::find_function_name_position(content, line, func_name)
    }
}

// Third impl block for undeclared fixtures scanning
impl FixtureDatabase {
    #[allow(clippy::too_many_arguments)]
    fn scan_function_body_for_undeclared_fixtures(
        &self,
        body: &[Stmt],
        file_path: &PathBuf,
        content: &str,
        line_index: &[usize],
        declared_params: &HashSet<String>,
        function_name: &str,
        function_line: usize,
    ) {
        // First, collect all local variable names with their definition line numbers
        let mut local_vars = HashMap::new();
        self.collect_local_variables(body, content, line_index, &mut local_vars);

        // Also add imported names to local_vars (they shouldn't be flagged as undeclared fixtures)
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

    #[allow(clippy::only_used_in_recursion)]
    fn collect_local_variables(
        &self,
        body: &[Stmt],
        content: &str,
        line_index: &[usize],
        local_vars: &mut HashMap<String, usize>,
    ) {
        for stmt in body {
            match stmt {
                Stmt::Assign(assign) => {
                    let line =
                        self.get_line_from_offset(assign.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    for target in &assign.targets {
                        self.collect_names_from_expr(target, &mut temp_names);
                    }
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::AnnAssign(ann_assign) => {
                    let line =
                        self.get_line_from_offset(ann_assign.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    self.collect_names_from_expr(&ann_assign.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::AugAssign(aug_assign) => {
                    let line =
                        self.get_line_from_offset(aug_assign.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    self.collect_names_from_expr(&aug_assign.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                }
                Stmt::For(for_stmt) => {
                    let line =
                        self.get_line_from_offset(for_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    self.collect_names_from_expr(&for_stmt.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                    self.collect_local_variables(&for_stmt.body, content, line_index, local_vars);
                }
                Stmt::AsyncFor(for_stmt) => {
                    let line =
                        self.get_line_from_offset(for_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
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
                    let line =
                        self.get_line_from_offset(with_stmt.range.start().to_usize(), line_index);
                    for item in &with_stmt.items {
                        if let Some(ref optional_vars) = item.optional_vars {
                            let mut temp_names = HashSet::new();
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
                            let mut temp_names = HashSet::new();
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

    #[allow(clippy::too_many_arguments)]
    fn visit_stmt_for_names(
        &self,
        stmt: &Stmt,
        file_path: &PathBuf,
        content: &str,
        line_index: &[usize],
        declared_params: &HashSet<String>,
        local_vars: &HashMap<String, usize>,
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
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
    fn visit_expr_for_names(
        &self,
        expr: &Expr,
        file_path: &PathBuf,
        content: &str,
        line_index: &[usize],
        declared_params: &HashSet<String>,
        local_vars: &HashMap<String, usize>,
        function_name: &str,
        function_line: usize,
    ) {
        match expr {
            Expr::Name(name) => {
                let name_str = name.id.as_str();
                let line = self.get_line_from_offset(name.range.start().to_usize(), line_index);

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
            _ => {}
        }
    }

    /// Check if a fixture is available at the given file location
    pub(crate) fn is_available_fixture(&self, file_path: &Path, fixture_name: &str) -> bool {
        if let Some(definitions) = self.definitions.get(fixture_name) {
            for def in definitions.iter() {
                // Fixture is available if it's in the same file
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
}
