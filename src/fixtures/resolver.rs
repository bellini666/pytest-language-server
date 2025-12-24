//! Fixture resolution and query methods.
//!
//! This module contains methods for finding fixture definitions,
//! references, and providing completion context.

use super::decorators;
use super::types::{
    CompletionContext, FixtureDefinition, FixtureUsage, ParamInsertionInfo, UndeclaredFixture,
};
use super::FixtureDatabase;
use rustpython_parser::ast::{Expr, Ranged, Stmt};
use std::collections::HashSet;
use std::path::Path;
use tracing::{debug, info};

impl FixtureDatabase {
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

        let content = self.get_file_content(file_path)?;
        let line_content = content.lines().nth(target_line.saturating_sub(1))?;
        debug!("Line content: {}", line_content);

        let word_at_cursor = self.extract_word_at_position(line_content, character as usize)?;
        debug!("Word at cursor: {:?}", word_at_cursor);

        // Check if we're inside a fixture definition with the same name (self-referencing)
        let current_fixture_def = self.get_fixture_definition_at_line(file_path, target_line);

        // First, check if this word matches any fixture usage on this line
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line && usage.name == word_at_cursor {
                    let cursor_pos = character as usize;
                    if cursor_pos >= usage.start_char && cursor_pos < usage.end_char {
                        debug!(
                            "Cursor at {} is within usage range {}-{}: {}",
                            cursor_pos, usage.start_char, usage.end_char, usage.name
                        );
                        info!("Found fixture usage at cursor position: {}", usage.name);

                        // If we're in a fixture definition with the same name, skip it
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

    /// Find fixture definition at a given position, checking both usages and definitions.
    ///
    /// This is useful for Call Hierarchy where we want to work on both fixture definition
    /// lines and fixture usage sites.
    pub fn find_fixture_or_definition_at_position(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<FixtureDefinition> {
        // First try to find a usage and resolve it to definition
        if let Some(def) = self.find_fixture_definition(file_path, line, character) {
            return Some(def);
        }

        // If not a usage, check if we're on a fixture definition line
        let target_line = (line + 1) as usize; // Convert from 0-based to 1-based
        let content = self.get_file_content(file_path)?;
        let line_content = content.lines().nth(target_line.saturating_sub(1))?;
        let word_at_cursor = self.extract_word_at_position(line_content, character as usize)?;

        // Check if this word matches a fixture definition at this line
        if let Some(definitions) = self.definitions.get(&word_at_cursor) {
            for def in definitions.iter() {
                if def.file_path == file_path && def.line == target_line {
                    // Verify cursor is within the fixture name
                    if character as usize >= def.start_char && (character as usize) < def.end_char {
                        return Some(def.clone());
                    }
                }
            }
        }

        None
    }

    /// Public method to get the fixture definition at a specific line and name
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

    /// Find the closest fixture definition based on pytest priority rules.
    pub(crate) fn find_closest_definition(
        &self,
        file_path: &Path,
        fixture_name: &str,
    ) -> Option<FixtureDefinition> {
        self.find_closest_definition_with_filter(file_path, fixture_name, |_| true)
    }

    /// Find the closest definition, excluding a specific definition.
    pub(crate) fn find_closest_definition_excluding(
        &self,
        file_path: &Path,
        fixture_name: &str,
        exclude: Option<&FixtureDefinition>,
    ) -> Option<FixtureDefinition> {
        self.find_closest_definition_with_filter(file_path, fixture_name, |def| {
            if let Some(excluded) = exclude {
                def != excluded
            } else {
                true
            }
        })
    }

    /// Internal helper that implements pytest priority rules with a custom filter.
    /// Priority order:
    /// 1. Same file (highest priority, last definition wins)
    /// 2. Closest conftest.py in parent directories (including imported fixtures)
    /// 3. Third-party fixtures from site-packages
    fn find_closest_definition_with_filter<F>(
        &self,
        file_path: &Path,
        fixture_name: &str,
        filter: F,
    ) -> Option<FixtureDefinition>
    where
        F: Fn(&FixtureDefinition) -> bool,
    {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Same file (highest priority)
        debug!(
            "Checking for fixture {} in same file: {:?}",
            fixture_name, file_path
        );

        if let Some(last_def) = definitions
            .iter()
            .filter(|def| def.file_path == file_path && filter(def))
            .max_by_key(|def| def.line)
        {
            info!(
                "Found fixture {} in same file at line {}",
                fixture_name, last_def.line
            );
            return Some(last_def.clone());
        }

        // Priority 2: Search upward through conftest.py files
        let mut current_dir = file_path.parent()?;

        debug!(
            "Searching for fixture {} in conftest.py files starting from {:?}",
            fixture_name, current_dir
        );
        loop {
            let conftest_path = current_dir.join("conftest.py");
            debug!("  Checking conftest.py at: {:?}", conftest_path);

            // First check if the fixture is defined directly in this conftest
            for def in definitions.iter() {
                if def.file_path == conftest_path && filter(def) {
                    info!(
                        "Found fixture {} in conftest.py: {:?}",
                        fixture_name, conftest_path
                    );
                    return Some(def.clone());
                }
            }

            // Then check if the conftest imports this fixture
            // Check both filesystem and file cache for conftest existence
            let conftest_in_cache = self.file_cache.contains_key(&conftest_path);
            if (conftest_path.exists() || conftest_in_cache)
                && self.is_fixture_imported_in_file(fixture_name, &conftest_path)
            {
                // The fixture is imported in this conftest, so it's available here
                // Return the original definition (pytest makes it available at conftest scope)
                debug!(
                    "Fixture {} is imported in conftest.py: {:?}",
                    fixture_name, conftest_path
                );
                // Get any matching definition that passes the filter
                if let Some(def) = definitions.iter().find(|def| filter(def)) {
                    info!(
                        "Found imported fixture {} via conftest.py: {:?} (original: {:?})",
                        fixture_name, conftest_path, def.file_path
                    );
                    return Some(def.clone());
                }
            }

            match current_dir.parent() {
                Some(parent) => current_dir = parent,
                None => break,
            }
        }

        // Priority 3: Third-party fixtures (site-packages)
        debug!(
            "No fixture {} found in conftest hierarchy, checking third-party",
            fixture_name
        );
        for def in definitions.iter() {
            if def.is_third_party && filter(def) {
                info!(
                    "Found third-party fixture {} in site-packages: {:?}",
                    fixture_name, def.file_path
                );
                return Some(def.clone());
            }
        }

        debug!(
            "No fixture {} found in scope for {:?}",
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
        let target_line = (line + 1) as usize;

        debug!(
            "find_fixture_at_position: file={:?}, line={}, char={}",
            file_path, target_line, character
        );

        let content = self.get_file_content(file_path)?;
        let line_content = content.lines().nth(target_line.saturating_sub(1))?;
        debug!("Line content: {}", line_content);

        let word_at_cursor = self.extract_word_at_position(line_content, character as usize);
        debug!("Word at cursor: {:?}", word_at_cursor);

        // Check if this word matches any fixture usage on this line
        if let Some(usages) = self.usages.get(file_path) {
            for usage in usages.iter() {
                if usage.line == target_line {
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

        // Check if we're on a fixture definition line
        for entry in self.definitions.iter() {
            for def in entry.value().iter() {
                if def.file_path == file_path && def.line == target_line {
                    if let Some(ref word) = word_at_cursor {
                        if word == &def.name {
                            info!(
                                "Found fixture definition name at cursor position: {}",
                                def.name
                            );
                            return Some(def.name.clone());
                        }
                    }
                }
            }
        }

        debug!("No fixture found at cursor position");
        None
    }

    /// Extract the word at a given character position in a line
    pub fn extract_word_at_position(&self, line: &str, character: usize) -> Option<String> {
        super::string_utils::extract_word_at_position(line, character)
    }

    /// Find all references (usages) of a fixture by name
    pub fn find_fixture_references(&self, fixture_name: &str) -> Vec<FixtureUsage> {
        info!("Finding all references for fixture: {}", fixture_name);

        let mut all_references = Vec::new();

        for entry in self.usages.iter() {
            let file_path = entry.key();
            let usages = entry.value();

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

    /// Find all references that resolve to a specific fixture definition.
    /// Uses the usage_by_fixture reverse index for O(m) lookup where m = usages of this fixture,
    /// instead of O(n) iteration over all usages.
    pub fn find_references_for_definition(
        &self,
        definition: &FixtureDefinition,
    ) -> Vec<FixtureUsage> {
        info!(
            "Finding references for specific definition: {} at {:?}:{}",
            definition.name, definition.file_path, definition.line
        );

        let mut matching_references = Vec::new();

        // Use reverse index for O(m) lookup instead of O(n) iteration over all usages
        let Some(usages_for_fixture) = self.usage_by_fixture.get(&definition.name) else {
            info!("No references found for fixture: {}", definition.name);
            return matching_references;
        };

        for (file_path, usage) in usages_for_fixture.iter() {
            let fixture_def_at_line = self.get_fixture_definition_at_line(file_path, usage.line);

            let resolved_def = if let Some(ref current_def) = fixture_def_at_line {
                if current_def.name == usage.name {
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
                    self.find_closest_definition(file_path, &usage.name)
                }
            } else {
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

    /// Get all available fixtures for a given file.
    /// Results are cached with version-based invalidation for performance.
    /// Returns Arc to avoid cloning the potentially large Vec on cache hits.
    pub fn get_available_fixtures(&self, file_path: &Path) -> Vec<FixtureDefinition> {
        use std::sync::Arc;

        // Canonicalize path for consistent cache keys
        let file_path = self.get_canonical_path(file_path.to_path_buf());

        // Check cache first
        let current_version = self
            .definitions_version
            .load(std::sync::atomic::Ordering::SeqCst);

        if let Some(cached) = self.available_fixtures_cache.get(&file_path) {
            let (cached_version, cached_fixtures) = cached.value();
            if *cached_version == current_version {
                // Return cloned Vec from Arc (cheap reference count increment)
                return cached_fixtures.as_ref().clone();
            }
        }

        // Compute available fixtures
        let available_fixtures = self.compute_available_fixtures(&file_path);

        // Store in cache
        self.available_fixtures_cache.insert(
            file_path,
            (current_version, Arc::new(available_fixtures.clone())),
        );

        available_fixtures
    }

    /// Internal method to compute available fixtures without caching.
    fn compute_available_fixtures(&self, file_path: &Path) -> Vec<FixtureDefinition> {
        let mut available_fixtures = Vec::new();
        let mut seen_names = HashSet::new();

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

        // Priority 2: Fixtures in conftest.py files (including imported fixtures)
        if let Some(mut current_dir) = file_path.parent() {
            loop {
                let conftest_path = current_dir.join("conftest.py");

                // First add fixtures defined directly in the conftest
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

                // Then add fixtures imported into the conftest
                if self.file_cache.contains_key(&conftest_path) {
                    let mut visited = HashSet::new();
                    let imported_fixtures =
                        self.get_imported_fixtures(&conftest_path, &mut visited);
                    for fixture_name in imported_fixtures {
                        if !seen_names.contains(&fixture_name) {
                            // Get the original definition for this imported fixture
                            if let Some(definitions) = self.definitions.get(&fixture_name) {
                                if let Some(def) = definitions.first() {
                                    available_fixtures.push(def.clone());
                                    seen_names.insert(fixture_name);
                                }
                            }
                        }
                    }
                }

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
                if def.is_third_party && !seen_names.contains(fixture_name.as_str()) {
                    available_fixtures.push(def.clone());
                    seen_names.insert(fixture_name.clone());
                }
            }
        }

        available_fixtures.sort_by(|a, b| a.name.cmp(&b.name));
        available_fixtures
    }

    /// Get the completion context for a given position
    pub fn get_completion_context(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<CompletionContext> {
        let content = self.get_file_content(file_path)?;
        let target_line = (line + 1) as usize;
        let line_index = self.get_line_index(file_path, &content);

        let parsed = self.get_parsed_ast(file_path, &content)?;

        if let rustpython_parser::ast::Mod::Module(module) = parsed.as_ref() {
            // First check if we're inside a decorator
            if let Some(ctx) =
                self.check_decorator_context(&module.body, &content, target_line, &line_index)
            {
                return Some(ctx);
            }

            // Then check for function context
            return self.get_function_completion_context(
                &module.body,
                &content,
                target_line,
                character as usize,
                &line_index,
            );
        }

        None
    }

    /// Check if the cursor is inside a decorator that needs fixture completions
    fn check_decorator_context(
        &self,
        stmts: &[Stmt],
        _content: &str,
        target_line: usize,
        line_index: &[usize],
    ) -> Option<CompletionContext> {
        for stmt in stmts {
            let decorator_list = match stmt {
                Stmt::FunctionDef(f) => &f.decorator_list,
                Stmt::AsyncFunctionDef(f) => &f.decorator_list,
                Stmt::ClassDef(c) => &c.decorator_list,
                _ => continue,
            };

            for decorator in decorator_list {
                let dec_start_line =
                    self.get_line_from_offset(decorator.range().start().to_usize(), line_index);
                let dec_end_line =
                    self.get_line_from_offset(decorator.range().end().to_usize(), line_index);

                if target_line >= dec_start_line && target_line <= dec_end_line {
                    if decorators::is_usefixtures_decorator(decorator) {
                        return Some(CompletionContext::UsefixuturesDecorator);
                    }
                    if decorators::is_parametrize_decorator(decorator) {
                        return Some(CompletionContext::ParametrizeIndirect);
                    }
                }
            }

            // Recursively check class bodies
            if let Stmt::ClassDef(class_def) = stmt {
                if let Some(ctx) =
                    self.check_decorator_context(&class_def.body, _content, target_line, line_index)
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
        line_index: &[usize],
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
                        line_index,
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
                        line_index,
                    ) {
                        return Some(ctx);
                    }
                }
                Stmt::ClassDef(class_def) => {
                    if let Some(ctx) = self.get_function_completion_context(
                        &class_def.body,
                        content,
                        target_line,
                        target_char,
                        line_index,
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
        args: &rustpython_parser::ast::Arguments,
        range: rustpython_parser::text_size::TextRange,
        content: &str,
        target_line: usize,
        _target_char: usize,
        line_index: &[usize],
    ) -> Option<CompletionContext> {
        let func_start_line = self.get_line_from_offset(range.start().to_usize(), line_index);
        let func_end_line = self.get_line_from_offset(range.end().to_usize(), line_index);

        if target_line < func_start_line || target_line > func_end_line {
            return None;
        }

        let is_fixture = decorator_list.iter().any(decorators::is_fixture_decorator);
        let is_test = func_name.as_str().starts_with("test_");

        if !is_test && !is_fixture {
            return None;
        }

        // Collect all parameters
        let params: Vec<String> = FixtureDatabase::all_args(args)
            .map(|arg| arg.def.arg.to_string())
            .collect();

        // Find the line where the function signature ends
        let lines: Vec<&str> = content.lines().collect();

        let mut sig_end_line = func_start_line;
        for (i, line) in lines
            .iter()
            .enumerate()
            .skip(func_start_line.saturating_sub(1))
        {
            if line.contains("):") {
                sig_end_line = i + 1;
                break;
            }
            if i + 1 > func_start_line + 10 {
                break;
            }
        }

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

        for i in (function_line.saturating_sub(1))..lines.len().min(function_line + 10) {
            let line = lines[i];
            if let Some(paren_pos) = line.find("):") {
                let has_params = if let Some(open_pos) = line.find('(') {
                    if open_pos < paren_pos {
                        let params_section = &line[open_pos + 1..paren_pos];
                        !params_section.trim().is_empty()
                    } else {
                        true
                    }
                } else {
                    let before_close = &line[..paren_pos];
                    if !before_close.trim().is_empty() {
                        true
                    } else {
                        let mut found_params = false;
                        for prev_line in lines.iter().take(i).skip(function_line.saturating_sub(1))
                        {
                            if prev_line.contains('(') {
                                if let Some(open_pos) = prev_line.find('(') {
                                    let after_open = &prev_line[open_pos + 1..];
                                    if !after_open.trim().is_empty() {
                                        found_params = true;
                                        break;
                                    }
                                }
                            } else if !prev_line.trim().is_empty() {
                                found_params = true;
                                break;
                            }
                        }
                        found_params
                    }
                };

                return Some(ParamInsertionInfo {
                    line: i + 1,
                    char_pos: paren_pos,
                    needs_comma: has_params,
                });
            }
        }

        None
    }

    /// Check if a position is inside a test or fixture function (parameter or body)
    /// Returns Some((function_name, is_fixture, declared_params)) if inside a function
    #[allow(dead_code)] // Used in tests
    #[allow(dead_code)] // Used in tests
    pub fn is_inside_function(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Option<(String, bool, Vec<String>)> {
        // Try cache first, then file system
        let content = self.get_file_content(file_path)?;

        let target_line = (line + 1) as usize; // Convert to 1-based

        // Parse the file (using cached AST)
        let parsed = self.get_parsed_ast(file_path, &content)?;

        if let rustpython_parser::ast::Mod::Module(module) = parsed.as_ref() {
            return self.find_enclosing_function(
                &module.body,
                &content,
                target_line,
                character as usize,
            );
        }

        None
    }

    #[allow(dead_code)]
    fn find_enclosing_function(
        &self,
        stmts: &[Stmt],
        content: &str,
        target_line: usize,
        _target_char: usize,
    ) -> Option<(String, bool, Vec<String>)> {
        let line_index = Self::build_line_index(content);

        for stmt in stmts {
            match stmt {
                Stmt::FunctionDef(func_def) => {
                    let func_start_line =
                        self.get_line_from_offset(func_def.range.start().to_usize(), &line_index);
                    let func_end_line =
                        self.get_line_from_offset(func_def.range.end().to_usize(), &line_index);

                    // Check if target is within this function's range
                    if target_line >= func_start_line && target_line <= func_end_line {
                        let is_fixture = func_def
                            .decorator_list
                            .iter()
                            .any(decorators::is_fixture_decorator);
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
                    let func_start_line =
                        self.get_line_from_offset(func_def.range.start().to_usize(), &line_index);
                    let func_end_line =
                        self.get_line_from_offset(func_def.range.end().to_usize(), &line_index);

                    if target_line >= func_start_line && target_line <= func_end_line {
                        let is_fixture = func_def
                            .decorator_list
                            .iter()
                            .any(decorators::is_fixture_decorator);
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

    // ============ Cycle Detection ============

    /// Detect circular dependencies in fixtures with caching.
    /// Results are cached and only recomputed when definitions change.
    /// Returns Arc to avoid cloning the potentially large Vec.
    pub fn detect_fixture_cycles(&self) -> std::sync::Arc<Vec<super::types::FixtureCycle>> {
        use std::sync::Arc;

        let current_version = self
            .definitions_version
            .load(std::sync::atomic::Ordering::SeqCst);

        // Check cache first
        if let Some(cached) = self.cycle_cache.get(&()) {
            let (cached_version, cached_cycles) = cached.value();
            if *cached_version == current_version {
                return Arc::clone(cached_cycles);
            }
        }

        // Compute cycles
        let cycles = Arc::new(self.compute_fixture_cycles());

        // Store in cache
        self.cycle_cache
            .insert((), (current_version, Arc::clone(&cycles)));

        cycles
    }

    /// Actually compute fixture cycles using iterative DFS (Tarjan-like approach).
    /// Uses iterative algorithm to avoid stack overflow on deep dependency graphs.
    fn compute_fixture_cycles(&self) -> Vec<super::types::FixtureCycle> {
        use super::types::FixtureCycle;
        use std::collections::HashMap;

        // Build dependency graph: fixture_name -> dependencies (only known fixtures)
        let mut dep_graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut fixture_defs: HashMap<String, FixtureDefinition> = HashMap::new();

        for entry in self.definitions.iter() {
            let fixture_name = entry.key().clone();
            if let Some(def) = entry.value().first() {
                fixture_defs.insert(fixture_name.clone(), def.clone());
                // Only include dependencies that are known fixtures
                let valid_deps: Vec<String> = def
                    .dependencies
                    .iter()
                    .filter(|d| self.definitions.contains_key(*d))
                    .cloned()
                    .collect();
                dep_graph.insert(fixture_name, valid_deps);
            }
        }

        let mut cycles = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut seen_cycles: HashSet<String> = HashSet::new(); // Deduplicate cycles

        // Iterative DFS using explicit stack
        for start_fixture in dep_graph.keys() {
            if visited.contains(start_fixture) {
                continue;
            }

            // Stack entries: (fixture_name, iterator_index, path_to_here)
            let mut stack: Vec<(String, usize, Vec<String>)> =
                vec![(start_fixture.clone(), 0, vec![])];
            let mut rec_stack: HashSet<String> = HashSet::new();

            while let Some((current, idx, mut path)) = stack.pop() {
                if idx == 0 {
                    // First time visiting this node
                    if rec_stack.contains(&current) {
                        // Found a cycle
                        let cycle_start_idx = path.iter().position(|f| f == &current).unwrap_or(0);
                        let mut cycle_path: Vec<String> = path[cycle_start_idx..].to_vec();
                        cycle_path.push(current.clone());

                        // Create a canonical key for deduplication (sorted cycle representation)
                        let mut cycle_key: Vec<String> =
                            cycle_path[..cycle_path.len() - 1].to_vec();
                        cycle_key.sort();
                        let cycle_key_str = cycle_key.join(",");

                        if !seen_cycles.contains(&cycle_key_str) {
                            seen_cycles.insert(cycle_key_str);
                            if let Some(fixture_def) = fixture_defs.get(&current) {
                                cycles.push(FixtureCycle {
                                    cycle_path,
                                    fixture: fixture_def.clone(),
                                });
                            }
                        }
                        continue;
                    }

                    rec_stack.insert(current.clone());
                    path.push(current.clone());
                }

                // Get dependencies for current node
                let deps = match dep_graph.get(&current) {
                    Some(d) => d,
                    None => {
                        rec_stack.remove(&current);
                        continue;
                    }
                };

                if idx < deps.len() {
                    // Push current back with next index
                    stack.push((current.clone(), idx + 1, path.clone()));

                    let dep = &deps[idx];
                    if rec_stack.contains(dep) {
                        // Found a cycle through this dependency
                        let cycle_start_idx = path.iter().position(|f| f == dep).unwrap_or(0);
                        let mut cycle_path: Vec<String> = path[cycle_start_idx..].to_vec();
                        cycle_path.push(dep.clone());

                        let mut cycle_key: Vec<String> =
                            cycle_path[..cycle_path.len() - 1].to_vec();
                        cycle_key.sort();
                        let cycle_key_str = cycle_key.join(",");

                        if !seen_cycles.contains(&cycle_key_str) {
                            seen_cycles.insert(cycle_key_str);
                            if let Some(fixture_def) = fixture_defs.get(dep) {
                                cycles.push(FixtureCycle {
                                    cycle_path,
                                    fixture: fixture_def.clone(),
                                });
                            }
                        }
                    } else if !visited.contains(dep) {
                        // Explore this dependency
                        stack.push((dep.clone(), 0, path.clone()));
                    }
                } else {
                    // Done with this node
                    visited.insert(current.clone());
                    rec_stack.remove(&current);
                }
            }
        }

        cycles
    }

    /// Detect cycles for fixtures in a specific file.
    /// Returns cycles where the first fixture in the cycle is defined in the given file.
    /// Uses cached cycle detection results for efficiency.
    pub fn detect_fixture_cycles_in_file(
        &self,
        file_path: &Path,
    ) -> Vec<super::types::FixtureCycle> {
        let all_cycles = self.detect_fixture_cycles();
        all_cycles
            .iter()
            .filter(|cycle| cycle.fixture.file_path == file_path)
            .cloned()
            .collect()
    }

    // ============ Scope Validation ============

    /// Detect scope mismatches where a broader-scoped fixture depends on a narrower-scoped fixture.
    /// For example, a session-scoped fixture depending on a function-scoped fixture.
    /// Returns mismatches for fixtures defined in the given file.
    pub fn detect_scope_mismatches_in_file(
        &self,
        file_path: &Path,
    ) -> Vec<super::types::ScopeMismatch> {
        use super::types::ScopeMismatch;

        let mut mismatches = Vec::new();

        // Get fixtures defined in this file
        let Some(fixture_names) = self.file_definitions.get(file_path) else {
            return mismatches;
        };

        for fixture_name in fixture_names.iter() {
            // Get the fixture definition
            let Some(definitions) = self.definitions.get(fixture_name) else {
                continue;
            };

            // Find the definition in this file
            let Some(fixture_def) = definitions.iter().find(|d| d.file_path == file_path) else {
                continue;
            };

            // Check each dependency
            for dep_name in &fixture_def.dependencies {
                // Find the dependency's definition (use resolution logic to get correct one)
                if let Some(dep_definitions) = self.definitions.get(dep_name) {
                    // Find best matching definition for the dependency
                    // Use the first one (most local) - matches cycle detection behavior
                    if let Some(dep_def) = dep_definitions.first() {
                        // Check if scope mismatch: fixture has broader scope than dependency
                        // FixtureScope is ordered: Function < Class < Module < Package < Session
                        if fixture_def.scope > dep_def.scope {
                            mismatches.push(ScopeMismatch {
                                fixture: fixture_def.clone(),
                                dependency: dep_def.clone(),
                            });
                        }
                    }
                }
            }
        }

        mismatches
    }

    /// Resolve a fixture by name for a given file using priority rules.
    ///
    /// Returns the best matching FixtureDefinition based on pytest's
    /// fixture shadowing rules: same file > conftest hierarchy > third-party.
    pub fn resolve_fixture_for_file(
        &self,
        file_path: &Path,
        fixture_name: &str,
    ) -> Option<FixtureDefinition> {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Same file
        if let Some(def) = definitions.iter().find(|d| d.file_path == file_path) {
            return Some(def.clone());
        }

        // Priority 2: conftest.py in parent directories (closest first)
        let file_path = self.get_canonical_path(file_path.to_path_buf());
        let mut best_conftest: Option<&FixtureDefinition> = None;
        let mut best_depth = usize::MAX;

        for def in definitions.iter() {
            if def.is_third_party {
                continue;
            }
            if def.file_path.ends_with("conftest.py") {
                if let Some(parent) = def.file_path.parent() {
                    if file_path.starts_with(parent) {
                        let depth = parent.components().count();
                        if depth > best_depth {
                            // Deeper = closer conftest
                            best_conftest = Some(def);
                            best_depth = depth;
                        } else if best_conftest.is_none() {
                            best_conftest = Some(def);
                            best_depth = depth;
                        }
                    }
                }
            }
        }

        if let Some(def) = best_conftest {
            return Some(def.clone());
        }

        // Priority 3: Third-party (site-packages)
        if let Some(def) = definitions.iter().find(|d| d.is_third_party) {
            return Some(def.clone());
        }

        // Fallback: first definition
        definitions.first().cloned()
    }

    /// Find the name of the function/fixture containing a given line.
    ///
    /// Used for call hierarchy to identify callers.
    pub fn find_containing_function(&self, file_path: &Path, line: usize) -> Option<String> {
        let content = self.get_file_content(file_path)?;

        // Use cached AST to avoid re-parsing
        let parsed = self.get_parsed_ast(file_path, &content)?;

        if let rustpython_parser::ast::Mod::Module(module) = parsed.as_ref() {
            // Use cached line index for position calculations
            let line_index = self.get_line_index(file_path, &content);

            for stmt in &module.body {
                if let Some(name) = self.find_function_containing_line(stmt, line, &line_index) {
                    return Some(name);
                }
            }
        }

        None
    }

    /// Recursively search for a function containing the given line.
    fn find_function_containing_line(
        &self,
        stmt: &Stmt,
        target_line: usize,
        line_index: &[usize],
    ) -> Option<String> {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                let start_line =
                    self.get_line_from_offset(func_def.range.start().to_usize(), line_index);
                let end_line =
                    self.get_line_from_offset(func_def.range.end().to_usize(), line_index);

                if target_line >= start_line && target_line <= end_line {
                    return Some(func_def.name.to_string());
                }
            }
            Stmt::AsyncFunctionDef(func_def) => {
                let start_line =
                    self.get_line_from_offset(func_def.range.start().to_usize(), line_index);
                let end_line =
                    self.get_line_from_offset(func_def.range.end().to_usize(), line_index);

                if target_line >= start_line && target_line <= end_line {
                    return Some(func_def.name.to_string());
                }
            }
            Stmt::ClassDef(class_def) => {
                // Check methods inside the class
                for class_stmt in &class_def.body {
                    if let Some(name) =
                        self.find_function_containing_line(class_stmt, target_line, line_index)
                    {
                        return Some(name);
                    }
                }
            }
            _ => {}
        }
        None
    }
}
