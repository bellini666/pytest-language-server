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
use rustpython_parser::{parse, Mode};
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

    /// Find the closest fixture definition based on pytest priority rules
    pub(crate) fn find_closest_definition(
        &self,
        file_path: &Path,
        fixture_name: &str,
    ) -> Option<FixtureDefinition> {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Same file (highest priority)
        debug!(
            "Checking for fixture {} in same file: {:?}",
            fixture_name, file_path
        );

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

        // Priority 2: Search upward through conftest.py files
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
                    info!(
                        "Found fixture {} in conftest.py: {:?}",
                        fixture_name, conftest_path
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

        debug!(
            "No fixture {} found in scope for {:?}",
            fixture_name, file_path
        );
        None
    }

    /// Find the closest definition, excluding a specific definition
    pub(crate) fn find_closest_definition_excluding(
        &self,
        file_path: &Path,
        fixture_name: &str,
        exclude: Option<&FixtureDefinition>,
    ) -> Option<FixtureDefinition> {
        let definitions = self.definitions.get(fixture_name)?;

        // Priority 1: Same file, excluding the specified definition
        debug!(
            "Checking for fixture {} in same file: {:?} (excluding: {:?})",
            fixture_name, file_path, exclude
        );

        if let Some(last_def) = definitions
            .iter()
            .filter(|def| {
                if def.file_path != file_path {
                    return false;
                }
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
                "Found fixture {} in same file at line {} (excluding specified)",
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

            for def in definitions.iter() {
                if def.file_path == conftest_path {
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

            match current_dir.parent() {
                Some(parent) => current_dir = parent,
                None => break,
            }
        }

        // Priority 3: Third-party fixtures
        debug!(
            "No fixture {} found in conftest hierarchy (excluding specified), checking third-party",
            fixture_name
        );
        for def in definitions.iter() {
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

        debug!(
            "No fixture {} found in scope for {:?} (excluding specified)",
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

    /// Find all references that resolve to a specific fixture definition
    pub fn find_references_for_definition(
        &self,
        definition: &FixtureDefinition,
    ) -> Vec<FixtureUsage> {
        info!(
            "Finding references for specific definition: {} at {:?}:{}",
            definition.name, definition.file_path, definition.line
        );

        let mut matching_references = Vec::new();

        for entry in self.usages.iter() {
            let file_path = entry.key();
            let usages = entry.value();

            for usage in usages.iter() {
                if usage.name == definition.name {
                    let fixture_def_at_line =
                        self.get_fixture_definition_at_line(file_path, usage.line);

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

    /// Get all available fixtures for a given file
    pub fn get_available_fixtures(&self, file_path: &Path) -> Vec<FixtureDefinition> {
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

        // Priority 2: Fixtures in conftest.py files
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

        let parsed = parse(&content, Mode::Module, "").ok()?;

        if let rustpython_parser::ast::Mod::Module(module) = parsed {
            // First check if we're inside a decorator
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
        args: &rustpython_parser::ast::Arguments,
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
}
