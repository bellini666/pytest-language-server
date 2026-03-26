//! File analysis and AST parsing for fixture extraction.
//!
//! This module contains the core logic for parsing Python files and extracting
//! fixture definitions and usages. Docstring extraction is in `docstring.rs`
//! and undeclared fixture scanning is in `undeclared.rs`.

use super::decorators;
use super::types::{FixtureDefinition, FixtureUsage, TypeImportSpec};
use super::FixtureDatabase;
use once_cell::sync::Lazy;
use rustpython_parser::ast::{ArgWithDefault, Arguments, Expr, Stmt};
use rustpython_parser::{parse, Mode};
use std::collections::{HashMap, HashSet};
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
            // Insert into DashMap *before* the second pass: undeclared-fixture
            // scanning (`scan_function_body_for_undeclared_fixtures`) reads
            // `self.imports` during `visit_stmt`, so the data must be available.
            // The clone is unavoidable because `resolve_return_type_imports`
            // also needs a local reference to the set.
            self.imports
                .insert(file_path.clone(), module_level_names.clone());

            // Build a name→TypeImportSpec map from every import statement in the file.
            // Used during fixture analysis to resolve return-type annotation imports.
            let import_map = self.build_name_to_import_map(&module.body, &file_path);

            // Collect type aliases so that `-> MyType` can be expanded to the
            // underlying type before import resolution.
            let type_aliases = self.collect_type_aliases(&module.body, content);

            // Second pass: analyze fixtures and tests
            for stmt in &module.body {
                self.visit_stmt(
                    stmt,
                    &file_path,
                    is_conftest,
                    content,
                    &line_index,
                    &import_map,
                    &module_level_names,
                    &type_aliases,
                );
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
        is_parameter: bool,
    ) {
        let file_path_buf = file_path.to_path_buf();
        let usage = FixtureUsage {
            name: fixture_name.clone(),
            file_path: file_path_buf.clone(),
            line,
            start_char,
            end_char,
            is_parameter,
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
    pub(crate) fn record_fixture_definition(&self, definition: FixtureDefinition) {
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
    #[allow(clippy::too_many_arguments)]
    fn visit_stmt(
        &self,
        stmt: &Stmt,
        file_path: &PathBuf,
        _is_conftest: bool,
        content: &str,
        line_index: &[usize],
        import_map: &HashMap<String, TypeImportSpec>,
        module_level_names: &HashSet<String>,
        type_aliases: &HashMap<String, String>,
    ) {
        // First check for assignment-style fixtures: fixture_name = pytest.fixture()(func)
        if let Stmt::Assign(assign) = stmt {
            self.visit_assignment_fixture(assign, file_path, content, line_index);

            // Check for pytestmark = pytest.mark.usefixtures(...) or
            // pytestmark = [pytest.mark.usefixtures(...), ...]
            let is_pytestmark = assign.targets.iter().any(
                |target| matches!(target, Expr::Name(name) if name.id.as_str() == "pytestmark"),
            );
            if is_pytestmark {
                self.visit_pytestmark_assignment(Some(&assign.value), file_path, line_index);
            }
        }

        // Check for annotated pytestmark: pytestmark: T = pytest.mark.usefixtures(...)
        if let Stmt::AnnAssign(ann_assign) = stmt {
            let is_pytestmark = matches!(
                ann_assign.target.as_ref(),
                Expr::Name(name) if name.id.as_str() == "pytestmark"
            );
            if is_pytestmark {
                self.visit_pytestmark_assignment(
                    ann_assign.value.as_deref(),
                    file_path,
                    line_index,
                );
            }
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
                        false, // usefixtures string — not a function parameter
                    );
                }
            }

            for class_stmt in &class_def.body {
                self.visit_stmt(
                    class_stmt,
                    file_path,
                    _is_conftest,
                    content,
                    line_index,
                    import_map,
                    module_level_names,
                    type_aliases,
                );
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
                    false, // usefixtures string — not a function parameter
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
                    false, // parametrize indirect string — not a function parameter
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
            let autouse = decorators::extract_fixture_autouse(decorator);

            let line = self.get_line_from_offset(range.start().to_usize(), line_index);
            let docstring = self.extract_docstring(body);
            let raw_return_type = self.extract_return_type(returns, body, content);
            let return_type = raw_return_type.map(|rt| {
                if type_aliases.is_empty() {
                    rt
                } else {
                    let expanded = Self::expand_type_aliases(&rt, type_aliases);
                    if expanded != rt {
                        info!(
                            "Expanded type alias in fixture '{}': {} → {}",
                            fixture_name, rt, expanded
                        );
                    }
                    expanded
                }
            });
            let return_type_imports = match &return_type {
                Some(rt) => {
                    self.resolve_return_type_imports(rt, import_map, module_level_names, file_path)
                }
                None => vec![],
            };

            info!(
                "Found fixture definition: {} (function: {}, scope: {:?}) at {:?}:{}",
                fixture_name, func_name, scope, file_path, line
            );

            let (start_char, end_char) = self.find_function_name_position(content, line, func_name);

            let is_third_party = file_path.to_string_lossy().contains("site-packages")
                || self.is_editable_install_third_party(file_path);
            let is_plugin = self.plugin_fixture_files.contains_key(file_path);

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
                return_type_imports,
                is_third_party,
                is_plugin,
                dependencies: dependencies.clone(),
                scope,
                yield_line: self.find_yield_line(body, line_index),
                autouse,
            };

            self.record_fixture_definition(definition);

            // Record each dependency as a usage
            for arg in Self::all_args(args) {
                let arg_name = arg.def.arg.as_str();

                // `request` is excluded from *dependencies* (it is a special pytest
                // injection, not a regular fixture), but we DO record it as a usage
                // so that inlay hints and type-annotation code actions work on it.
                if arg_name != "self" {
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
                        true, // actual function parameter — can receive a type annotation
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
                        true, // actual function parameter — can receive a type annotation
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
                                file_path.to_string_lossy().contains("site-packages")
                                    || self.is_editable_install_third_party(file_path);
                            let is_plugin = self.plugin_fixture_files.contains_key(file_path);
                            let definition = FixtureDefinition {
                                name: fixture_name.to_string(),
                                file_path: file_path.clone(),
                                line,
                                end_line: line, // Assignment-style fixtures are single-line
                                start_char,
                                end_char,
                                docstring: None,
                                return_type: None,
                                return_type_imports: vec![],
                                is_third_party,
                                is_plugin,
                                dependencies: Vec::new(), // Assignment-style fixtures don't have explicit dependencies
                                scope: decorators::extract_fixture_scope(&outer_call.func)
                                    .unwrap_or_default(),
                                yield_line: None, // Assignment-style fixtures don't have yield statements
                                autouse: false,   // Assignment-style fixtures are never autouse
                            };

                            self.record_fixture_definition(definition);
                        }
                    }
                }
            }
        }
    }

    /// Handle pytestmark usefixtures — covers both plain and annotated assignments:
    ///   pytestmark = pytest.mark.usefixtures("fix1", "fix2")
    ///   pytestmark = [pytest.mark.usefixtures("fix1"), pytest.mark.skip]
    ///   pytestmark = (pytest.mark.usefixtures("fix1"), pytest.mark.usefixtures("fix2"))
    ///   pytestmark: list[MarkDecorator] = [pytest.mark.usefixtures("fix1"), ...]
    ///
    /// `value` is `None` for bare annotated assignments (`pytestmark: T`) which are a no-op.
    fn visit_pytestmark_assignment(
        &self,
        value: Option<&Expr>,
        file_path: &PathBuf,
        line_index: &[usize],
    ) {
        let Some(value) = value else {
            return;
        };

        let usefixtures = decorators::extract_usefixtures_from_expr(value);
        for (fixture_name, range) in usefixtures {
            let usage_line = self.get_line_from_offset(range.start().to_usize(), line_index);
            let start_char =
                self.get_char_position_from_offset(range.start().to_usize(), line_index);
            let end_char = self.get_char_position_from_offset(range.end().to_usize(), line_index);

            info!(
                "Found usefixtures usage via pytestmark assignment: {} at {:?}:{}:{}",
                fixture_name, file_path, usage_line, start_char
            );

            self.record_fixture_usage(
                file_path,
                fixture_name,
                usage_line,
                start_char.saturating_add(1),
                end_char.saturating_sub(1),
                false, // pytestmark usefixtures string — not a function parameter
            );
        }
    }
}

/// Python builtin types that never require an import statement.
/// Uses O(1) `HashSet` lookup, consistent with `is_standard_library_module()`.
static BUILTINS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "int",
        "str",
        "bool",
        "float",
        "bytes",
        "bytearray",
        "complex",
        "list",
        "dict",
        "tuple",
        "set",
        "frozenset",
        "type",
        "object",
        "None",
        "range",
        "slice",
        "memoryview",
        "property",
        "classmethod",
        "staticmethod",
        "super",
        "Exception",
        "BaseException",
        "ValueError",
        "TypeError",
        "RuntimeError",
        "NotImplementedError",
        "AttributeError",
        "KeyError",
        "IndexError",
        "StopIteration",
        "GeneratorExit",
    ]
    .into_iter()
    .collect()
});

// Second impl block for additional analyzer methods
impl FixtureDatabase {
    // ============ Type alias resolution ============

    /// Collect type aliases defined at module level.
    ///
    /// Recognises two forms:
    ///
    /// 1. **PEP 613** — `MyType: TypeAlias = Dict[str, int]`
    ///    (`Stmt::AnnAssign` where the annotation mentions `TypeAlias`)
    /// 2. **Old-style** — `MyType = Dict[str, int]`
    ///    (`Stmt::Assign` where the target is a single `Expr::Name` whose
    ///    first character is uppercase and the RHS looks like a type expression)
    ///
    /// Returns a mapping from alias name to the expanded type string.
    pub(crate) fn collect_type_aliases(
        &self,
        stmts: &[Stmt],
        content: &str,
    ) -> HashMap<String, String> {
        let mut aliases = HashMap::new();

        for stmt in stmts {
            match stmt {
                // PEP 613: `X: TypeAlias = <type_expr>`
                Stmt::AnnAssign(ann_assign) => {
                    if !Self::annotation_is_type_alias(&ann_assign.annotation) {
                        continue;
                    }
                    let Expr::Name(name) = ann_assign.target.as_ref() else {
                        continue;
                    };
                    let Some(value) = &ann_assign.value else {
                        continue;
                    };
                    let expanded = self.expr_to_string(value, content);
                    // Skip aliases that expand to raw `Any`: if the fixture file
                    // writes `MyType: TypeAlias = Any`, the alias name `MyType` is
                    // still in `module_level_names`, so `resolve_return_type_imports`
                    // will correctly generate `from <module> import MyType` for it.
                    // Expanding to `Any` would instead require adding
                    // `from typing import Any`, which misrepresents the intent.
                    if expanded != "Any" {
                        debug!("Type alias (PEP 613): {} = {}", name.id, expanded);
                        aliases.insert(name.id.to_string(), expanded);
                    }
                }

                // Old-style: `X = <type_expr>` where X starts with uppercase
                Stmt::Assign(assign) => {
                    if assign.targets.len() != 1 {
                        continue;
                    }
                    let Expr::Name(name) = &assign.targets[0] else {
                        continue;
                    };
                    // Heuristic: type alias names start with an uppercase letter.
                    if !name.id.starts_with(|c: char| c.is_ascii_uppercase()) {
                        continue;
                    }
                    if !Self::expr_looks_like_type(&assign.value) {
                        continue;
                    }
                    let expanded = self.expr_to_string(&assign.value, content);
                    // Same rationale as the PEP 613 branch above: skip `Any`-valued
                    // aliases so the alias name keeps its locally-defined import path.
                    if expanded != "Any" {
                        debug!("Type alias (old-style): {} = {}", name.id, expanded);
                        aliases.insert(name.id.to_string(), expanded);
                    }
                }

                _ => {}
            }
        }

        aliases
    }

    /// Check whether an annotation expression refers to `TypeAlias`.
    ///
    /// Matches `TypeAlias`, `typing.TypeAlias`, and `typing_extensions.TypeAlias`.
    fn annotation_is_type_alias(expr: &Expr) -> bool {
        match expr {
            Expr::Name(name) => name.id.as_str() == "TypeAlias",
            Expr::Attribute(attr) => {
                attr.attr.as_str() == "TypeAlias"
                    && matches!(
                        attr.value.as_ref(),
                        Expr::Name(n) if n.id.as_str() == "typing" || n.id.as_str() == "typing_extensions"
                    )
            }
            _ => false,
        }
    }

    /// Heuristic: does an expression look like a type annotation?
    ///
    /// Returns `true` for subscripts (`Dict[str, int]`), union operators
    /// (`int | str`), names (`Path`), attributes (`pathlib.Path`), `None`,
    /// and string literals (forward references like `"MyClass"`).
    fn expr_looks_like_type(expr: &Expr) -> bool {
        match expr {
            // Subscript: Dict[str, int], Optional[Path], list[int], etc.
            Expr::Subscript(_) => true,
            // Union: int | str
            Expr::BinOp(binop) => {
                matches!(binop.op, rustpython_parser::ast::Operator::BitOr)
                    && Self::expr_looks_like_type(&binop.left)
                    && Self::expr_looks_like_type(&binop.right)
            }
            // Simple name: uppercase (Path, MyClass) or a known builtin (str, int, …)
            Expr::Name(name) => {
                name.id.starts_with(|c: char| c.is_ascii_uppercase())
                    || BUILTINS.contains(name.id.as_str())
            }
            // Attribute: pathlib.Path
            Expr::Attribute(_) => true,
            // None literal or string literal (forward reference)
            Expr::Constant(c) => matches!(
                c.value,
                rustpython_parser::ast::Constant::None | rustpython_parser::ast::Constant::Str(_)
            ),
            _ => false,
        }
    }

    /// Expand type aliases in a return-type string.
    ///
    /// Performs a single pass of word-boundary-safe substitution. Each
    /// standalone identifier that matches a key in `type_aliases` is replaced
    /// with the expanded form.  A match is "standalone" when it is not
    /// preceded or followed by an alphanumeric character, underscore, or dot
    /// (preventing partial matches like `MyTypeExtra`).
    ///
    /// Expansion is applied at most `MAX_DEPTH` times to handle aliases that
    /// reference other aliases (e.g. `A = B`, `B = Dict[str, int]`).
    pub(crate) fn expand_type_aliases(
        type_str: &str,
        type_aliases: &HashMap<String, String>,
    ) -> String {
        const MAX_DEPTH: usize = 5;
        let mut result = type_str.to_string();

        for _ in 0..MAX_DEPTH {
            let mut changed = false;
            for (alias, expanded) in type_aliases {
                let new = super::string_utils::replace_identifier(&result, alias, expanded);
                if new != result {
                    result = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        result
    }

    // ============ Return-type import resolution ============

    /// Extract all distinct identifier tokens from a type annotation string.
    ///
    /// Walks the string collecting runs of `[a-zA-Z_][a-zA-Z0-9_]*` characters.
    /// Dotted names like `pathlib.Path` produce two separate tokens (`pathlib`,
    /// `Path`) — each is looked up independently in the import map, which is
    /// correct because:
    /// - `import pathlib` → `import_map["pathlib"]` matches `pathlib`
    /// - `from pathlib import Path` → `import_map["Path"]` matches `Path`
    ///
    /// # Examples
    /// - `"dict[str, Any]"` → `["dict", "str", "Any"]`
    /// - `"Optional[Path]"` → `["Optional", "Path"]`
    /// - `"pathlib.Path"` → `["pathlib", "Path"]`
    /// - `"Path | None"` → `["Path", "None"]`
    /// - `"list[dict[str, Any]]"` → `["list", "dict", "str", "Any"]`
    fn extract_type_identifiers(type_str: &str) -> Vec<&str> {
        let mut identifiers = Vec::new();
        let mut seen = HashSet::new();
        let bytes = type_str.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            let b = bytes[i];
            // Start of an identifier: [a-zA-Z_]
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let ident = &type_str[start..i];
                if seen.insert(ident) {
                    identifiers.push(ident);
                }
            } else {
                i += 1;
            }
        }

        identifiers
    }

    /// Resolve the import spec(s) needed to use a fixture's return type
    /// annotation in a consumer file (e.g. a test file).
    ///
    /// Handles simple types (`Path`), dotted names (`pathlib.Path`), generics
    /// (`Optional[Path]`, `dict[str, Any]`), unions (`Path | None`), and any
    /// nesting thereof.  Every identifier token in the type string is resolved
    /// independently.
    ///
    /// Resolution order **per identifier**:
    /// 1. Builtin types (`int`, `str`, …) — skip, no import needed.
    /// 2. Look up in `import_map` (built from the fixture file's imports).
    /// 3. If the name is locally defined in the fixture file (class,
    ///    assignment, …) but not imported, build an import from
    ///    `fixture_file`'s module path.
    /// 4. Otherwise skip.
    ///
    /// Results are deduplicated by `check_name`.
    fn resolve_return_type_imports(
        &self,
        return_type: &str,
        import_map: &HashMap<String, TypeImportSpec>,
        module_level_names: &HashSet<String>,
        fixture_file: &Path,
    ) -> Vec<TypeImportSpec> {
        let identifiers = Self::extract_type_identifiers(return_type);
        let mut specs: Vec<TypeImportSpec> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();

        for ident in identifiers {
            // Skip builtins — they never need an import.
            if BUILTINS.contains(ident) {
                continue;
            }

            // Avoid duplicates (e.g. `tuple[Path, Path]`).
            if !seen.insert(ident) {
                continue;
            }

            // Check the import map (covers `import X` and `from X import Y`).
            if let Some(spec) = import_map.get(ident) {
                specs.push(spec.clone());
                continue;
            }

            // If the name is defined locally in the fixture file (e.g. a class
            // in conftest.py), build an import from that file's module path.
            if module_level_names.contains(ident) {
                if let Some(module_path) = Self::file_path_to_module_path(fixture_file) {
                    specs.push(TypeImportSpec {
                        check_name: ident.to_string(),
                        import_statement: format!("from {} import {}", module_path, ident),
                    });
                }
            }
        }

        specs
    }

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
