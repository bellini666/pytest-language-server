//! Undeclared fixture detection in function bodies.
//!
//! This module scans function bodies for references to fixtures that
//! are not declared as function parameters.

use super::types::UndeclaredFixture;
use super::FixtureDatabase;
use rustpython_parser::ast::{Expr, Stmt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::info;

/// Context for scanning function bodies for undeclared fixtures.
/// This reduces the number of arguments passed to recursive functions.
pub(crate) struct BodyScanContext<'a> {
    pub file_path: &'a PathBuf,
    pub line_index: &'a [usize],
    pub declared_params: &'a HashSet<String>,
    pub local_vars: &'a HashMap<String, usize>,
    pub function_name: &'a str,
    pub function_line: usize,
}

impl FixtureDatabase {
    /// Scan a function body for undeclared fixture usages.
    /// An undeclared fixture is a reference to a fixture that exists in the database
    /// but is not declared as a parameter of the current function.
    pub(crate) fn scan_function_body_for_undeclared_fixtures(
        &self,
        body: &[Stmt],
        file_path: &PathBuf,
        line_index: &[usize],
        declared_params: &HashSet<String>,
        function_name: &str,
        function_line: usize,
    ) {
        // First, collect all local variable names with their definition line numbers
        let mut local_vars = HashMap::new();
        self.collect_local_variables(body, line_index, &mut local_vars);

        // Also add imported names to local_vars (they shouldn't be flagged as undeclared fixtures)
        if let Some(imports) = self.imports.get(file_path) {
            for import in imports.iter() {
                local_vars.insert(import.clone(), 0);
            }
        }

        let ctx = BodyScanContext {
            file_path,
            line_index,
            declared_params,
            local_vars: &local_vars,
            function_name,
            function_line,
        };

        // Walk through the function body and find all Name references
        for stmt in body {
            self.visit_stmt_for_names(stmt, &ctx);
        }
    }

    /// Collect all local variable names from a function body.
    /// Records the line number where each variable is defined for scope checking.
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn collect_local_variables(
        &self,
        body: &[Stmt],
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
                    self.collect_local_variables(&for_stmt.body, line_index, local_vars);
                }
                Stmt::AsyncFor(for_stmt) => {
                    let line =
                        self.get_line_from_offset(for_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    self.collect_names_from_expr(&for_stmt.target, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                    self.collect_local_variables(&for_stmt.body, line_index, local_vars);
                }
                Stmt::While(while_stmt) => {
                    self.collect_local_variables(&while_stmt.body, line_index, local_vars);
                }
                Stmt::If(if_stmt) => {
                    self.collect_local_variables(&if_stmt.body, line_index, local_vars);
                    self.collect_local_variables(&if_stmt.orelse, line_index, local_vars);
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
                    self.collect_local_variables(&with_stmt.body, line_index, local_vars);
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
                    self.collect_local_variables(&with_stmt.body, line_index, local_vars);
                }
                Stmt::Try(try_stmt) => {
                    self.collect_local_variables(&try_stmt.body, line_index, local_vars);
                    self.collect_local_variables(&try_stmt.orelse, line_index, local_vars);
                    self.collect_local_variables(&try_stmt.finalbody, line_index, local_vars);
                }
                _ => {}
            }
        }
    }

    /// Visit a statement and check for undeclared fixture references.
    fn visit_stmt_for_names(&self, stmt: &Stmt, ctx: &BodyScanContext) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                self.visit_expr_for_names(&expr_stmt.value, ctx);
            }
            Stmt::Assign(assign) => {
                self.visit_expr_for_names(&assign.value, ctx);
            }
            Stmt::AugAssign(aug_assign) => {
                self.visit_expr_for_names(&aug_assign.value, ctx);
            }
            Stmt::Return(ret) => {
                if let Some(ref value) = ret.value {
                    self.visit_expr_for_names(value, ctx);
                }
            }
            Stmt::If(if_stmt) => {
                self.visit_expr_for_names(&if_stmt.test, ctx);
                for stmt in &if_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
                for stmt in &if_stmt.orelse {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::While(while_stmt) => {
                self.visit_expr_for_names(&while_stmt.test, ctx);
                for stmt in &while_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::For(for_stmt) => {
                self.visit_expr_for_names(&for_stmt.iter, ctx);
                for stmt in &for_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::With(with_stmt) => {
                for item in &with_stmt.items {
                    self.visit_expr_for_names(&item.context_expr, ctx);
                }
                for stmt in &with_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::AsyncFor(for_stmt) => {
                self.visit_expr_for_names(&for_stmt.iter, ctx);
                for stmt in &for_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::AsyncWith(with_stmt) => {
                for item in &with_stmt.items {
                    self.visit_expr_for_names(&item.context_expr, ctx);
                }
                for stmt in &with_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::Assert(assert_stmt) => {
                self.visit_expr_for_names(&assert_stmt.test, ctx);
                if let Some(ref msg) = assert_stmt.msg {
                    self.visit_expr_for_names(msg, ctx);
                }
            }
            _ => {}
        }
    }

    /// Visit an expression and check for undeclared fixture references.
    #[allow(clippy::only_used_in_recursion)]
    fn visit_expr_for_names(&self, expr: &Expr, ctx: &BodyScanContext) {
        match expr {
            Expr::Name(name) => {
                let name_str = name.id.as_str();
                let line = self.get_line_from_offset(name.range.start().to_usize(), ctx.line_index);

                let is_local_var_in_scope = ctx
                    .local_vars
                    .get(name_str)
                    .map(|def_line| *def_line < line)
                    .unwrap_or(false);

                if !ctx.declared_params.contains(name_str)
                    && !is_local_var_in_scope
                    && self.is_available_fixture(ctx.file_path, name_str)
                {
                    let start_char = self.get_char_position_from_offset(
                        name.range.start().to_usize(),
                        ctx.line_index,
                    );
                    let end_char = self
                        .get_char_position_from_offset(name.range.end().to_usize(), ctx.line_index);

                    info!(
                        "Found undeclared fixture usage: {} at {:?}:{}:{} in function {}",
                        name_str, ctx.file_path, line, start_char, ctx.function_name
                    );

                    let undeclared = UndeclaredFixture {
                        name: name_str.to_string(),
                        file_path: ctx.file_path.clone(),
                        line,
                        start_char,
                        end_char,
                        function_name: ctx.function_name.to_string(),
                        function_line: ctx.function_line,
                    };

                    self.undeclared_fixtures
                        .entry(ctx.file_path.clone())
                        .or_default()
                        .push(undeclared);
                }
            }
            Expr::Call(call) => {
                self.visit_expr_for_names(&call.func, ctx);
                for arg in &call.args {
                    self.visit_expr_for_names(arg, ctx);
                }
            }
            Expr::Attribute(attr) => {
                self.visit_expr_for_names(&attr.value, ctx);
            }
            Expr::BinOp(binop) => {
                self.visit_expr_for_names(&binop.left, ctx);
                self.visit_expr_for_names(&binop.right, ctx);
            }
            Expr::UnaryOp(unaryop) => {
                self.visit_expr_for_names(&unaryop.operand, ctx);
            }
            Expr::Compare(compare) => {
                self.visit_expr_for_names(&compare.left, ctx);
                for comparator in &compare.comparators {
                    self.visit_expr_for_names(comparator, ctx);
                }
            }
            Expr::Subscript(subscript) => {
                self.visit_expr_for_names(&subscript.value, ctx);
                self.visit_expr_for_names(&subscript.slice, ctx);
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.visit_expr_for_names(elt, ctx);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.visit_expr_for_names(elt, ctx);
                }
            }
            Expr::Dict(dict) => {
                for k in dict.keys.iter().flatten() {
                    self.visit_expr_for_names(k, ctx);
                }
                for value in &dict.values {
                    self.visit_expr_for_names(value, ctx);
                }
            }
            Expr::Await(await_expr) => {
                self.visit_expr_for_names(&await_expr.value, ctx);
            }
            _ => {}
        }
    }

    /// Check if a fixture is available at the given file location.
    /// A fixture is available if it's in the same file, a conftest.py in a parent directory,
    /// or from a third-party package.
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
                if def.is_third_party {
                    return true;
                }

                // Check if it's from a pytest11 entry point plugin
                if def.is_plugin {
                    return true;
                }
            }
        }
        false
    }
}
