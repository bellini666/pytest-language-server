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

/// Collect names bound by walrus (`:=`) expressions anywhere inside `expr`.
fn collect_walrus_targets(expr: &Expr, names: &mut HashSet<String>) {
    if let Expr::NamedExpr(named) = expr {
        if let Expr::Name(name) = named.target.as_ref() {
            names.insert(name.id.to_string());
        }
        collect_walrus_targets(&named.value, names);
        return;
    }
    // Recurse into the common containers a walrus realistically appears in.
    match expr {
        Expr::BoolOp(e) => e
            .values
            .iter()
            .for_each(|v| collect_walrus_targets(v, names)),
        Expr::BinOp(e) => {
            collect_walrus_targets(&e.left, names);
            collect_walrus_targets(&e.right, names);
        }
        Expr::UnaryOp(e) => collect_walrus_targets(&e.operand, names),
        Expr::Compare(e) => {
            collect_walrus_targets(&e.left, names);
            e.comparators
                .iter()
                .for_each(|c| collect_walrus_targets(c, names));
        }
        Expr::Call(e) => e.args.iter().for_each(|a| collect_walrus_targets(a, names)),
        Expr::Tuple(e) => e.elts.iter().for_each(|v| collect_walrus_targets(v, names)),
        Expr::IfExp(e) => {
            collect_walrus_targets(&e.test, names);
            collect_walrus_targets(&e.body, names);
            collect_walrus_targets(&e.orelse, names);
        }
        _ => {}
    }
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
                    collect_walrus_targets(&assign.value, &mut temp_names);
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
                    let line =
                        self.get_line_from_offset(while_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    collect_walrus_targets(&while_stmt.test, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
                    self.collect_local_variables(&while_stmt.body, line_index, local_vars);
                }
                Stmt::If(if_stmt) => {
                    let line =
                        self.get_line_from_offset(if_stmt.range.start().to_usize(), line_index);
                    let mut temp_names = HashSet::new();
                    collect_walrus_targets(&if_stmt.test, &mut temp_names);
                    for name in temp_names {
                        local_vars.insert(name, line);
                    }
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
                for stmt in &while_stmt.orelse {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::For(for_stmt) => {
                self.visit_expr_for_names(&for_stmt.iter, ctx);
                for stmt in &for_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
                for stmt in &for_stmt.orelse {
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
            Stmt::AnnAssign(ann_assign) => {
                if let Some(ref value) = ann_assign.value {
                    self.visit_expr_for_names(value, ctx);
                }
            }
            Stmt::Raise(raise_stmt) => {
                if let Some(ref exc) = raise_stmt.exc {
                    self.visit_expr_for_names(exc, ctx);
                }
                if let Some(ref cause) = raise_stmt.cause {
                    self.visit_expr_for_names(cause, ctx);
                }
            }
            Stmt::Try(try_stmt) => {
                for stmt in &try_stmt.body {
                    self.visit_stmt_for_names(stmt, ctx);
                }
                for handler in &try_stmt.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(h) = handler;
                    for stmt in &h.body {
                        self.visit_stmt_for_names(stmt, ctx);
                    }
                }
                for stmt in &try_stmt.orelse {
                    self.visit_stmt_for_names(stmt, ctx);
                }
                for stmt in &try_stmt.finalbody {
                    self.visit_stmt_for_names(stmt, ctx);
                }
            }
            Stmt::Match(match_stmt) => {
                self.visit_expr_for_names(&match_stmt.subject, ctx);
                for case in &match_stmt.cases {
                    if let Some(ref guard) = case.guard {
                        self.visit_expr_for_names(guard, ctx);
                    }
                    for stmt in &case.body {
                        self.visit_stmt_for_names(stmt, ctx);
                    }
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
            Expr::BoolOp(bool_op) => {
                for value in &bool_op.values {
                    self.visit_expr_for_names(value, ctx);
                }
            }
            Expr::IfExp(if_exp) => {
                self.visit_expr_for_names(&if_exp.test, ctx);
                self.visit_expr_for_names(&if_exp.body, ctx);
                self.visit_expr_for_names(&if_exp.orelse, ctx);
            }
            Expr::NamedExpr(named) => {
                // Only the value is a read; the walrus target is a binding.
                self.visit_expr_for_names(&named.value, ctx);
            }
            Expr::Starred(starred) => {
                self.visit_expr_for_names(&starred.value, ctx);
            }
            Expr::JoinedStr(joined) => {
                for value in &joined.values {
                    self.visit_expr_for_names(value, ctx);
                }
            }
            Expr::FormattedValue(formatted) => {
                self.visit_expr_for_names(&formatted.value, ctx);
            }
            Expr::Set(set) => {
                for elt in &set.elts {
                    self.visit_expr_for_names(elt, ctx);
                }
            }
            Expr::Slice(slice) => {
                for part in [&slice.lower, &slice.upper, &slice.step]
                    .into_iter()
                    .flatten()
                {
                    self.visit_expr_for_names(part, ctx);
                }
            }
            // Comprehensions: only the iterables are visited — elements and
            // conditions typically reference comprehension-local loop targets,
            // which would produce false positives without scope tracking.
            Expr::ListComp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx);
                }
            }
            Expr::SetComp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx);
                }
            }
            Expr::GeneratorExp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx);
                }
            }
            Expr::DictComp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Seed a conftest with a single fixture and analyze a test file that
    /// references it in some shape. Returns the undeclared fixtures detected
    /// in the test file.
    fn analyze_with_conftest(test_body: &str) -> Vec<UndeclaredFixture> {
        let db = FixtureDatabase::new();
        let base = std::env::temp_dir().join("pls_undeclared_unit");

        let conftest_path = base.join("conftest.py");
        db.analyze_file(
            conftest_path,
            "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
        );

        let test_path = base.join("test_example.py");
        let content = format!("def test_one():\n{}\n", test_body);
        db.analyze_file(test_path.clone(), &content);

        db.get_undeclared_fixtures(&test_path)
    }

    #[test]
    fn test_with_statement_binding_shadows_outer_name() {
        // `my_fixture` is bound by the `with ... as my_fixture:` statement,
        // so a later read of it is a local variable, not an undeclared fixture.
        let undeclared =
            analyze_with_conftest("    with open(\"x\") as my_fixture:\n        _ = my_fixture\n");
        assert!(
            undeclared.iter().all(|u| u.name != "my_fixture"),
            "with-binding should suppress undeclared flag, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_for_loop_target_captured_as_local() {
        // `my_fixture` is the loop variable → local, not undeclared.
        let undeclared =
            analyze_with_conftest("    for my_fixture in []:\n        _ = my_fixture\n");
        assert!(
            undeclared.iter().all(|u| u.name != "my_fixture"),
            "for-loop target should be a local, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_imported_name_not_flagged_as_undeclared_fixture() {
        // Imports are tracked per file: if `my_fixture` is imported in the
        // test file, it should *not* be flagged even though it is also a
        // fixture defined in conftest.
        let db = FixtureDatabase::new();
        let base = std::env::temp_dir().join("pls_undeclared_unit_imported");

        let conftest_path = base.join("conftest.py");
        db.analyze_file(
            conftest_path,
            "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
        );

        let test_path = base.join("test_example.py");
        db.analyze_file(
            test_path.clone(),
            "from helpers import my_fixture\n\ndef test_one():\n    _ = my_fixture\n",
        );

        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert!(
            undeclared.iter().all(|u| u.name != "my_fixture"),
            "imported name should not be flagged, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_undeclared_flagged_in_assignment_rhs() {
        // Baseline: referencing `my_fixture` on the RHS of an assignment
        // without declaring it as a parameter *is* flagged.
        let undeclared = analyze_with_conftest("    x = my_fixture\n");
        assert!(
            undeclared.iter().any(|u| u.name == "my_fixture"),
            "baseline undeclared detection failed, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_undeclared_flagged_inside_dict_value() {
        // Dict literal value should still be walked.
        let undeclared = analyze_with_conftest("    x = {\"k\": my_fixture}\n");
        assert!(
            undeclared.iter().any(|u| u.name == "my_fixture"),
            "fixture inside dict value should be flagged, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_declared_parameter_suppresses_flag() {
        // If the fixture is declared as a parameter, it's not undeclared.
        let db = FixtureDatabase::new();
        let base = std::env::temp_dir().join("pls_undeclared_unit_declared");

        let conftest_path = base.join("conftest.py");
        db.analyze_file(
            conftest_path,
            "import pytest\n\n@pytest.fixture\ndef my_fixture():\n    return 1\n",
        );

        let test_path = base.join("test_example.py");
        db.analyze_file(
            test_path.clone(),
            "def test_one(my_fixture):\n    _ = my_fixture\n",
        );
        let undeclared = db.get_undeclared_fixtures(&test_path);
        assert!(
            undeclared.iter().all(|u| u.name != "my_fixture"),
            "declared parameter should suppress flag, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_undeclared_flagged_in_fstring() {
        let undeclared = analyze_with_conftest("    x = f\"{my_fixture}\"\n");
        assert!(
            undeclared.iter().any(|u| u.name == "my_fixture"),
            "fixture inside f-string should be flagged, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_undeclared_flagged_in_ternary_and_boolop() {
        let undeclared = analyze_with_conftest("    x = 1 if my_fixture else 2\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest("    x = my_fixture or None\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }

    #[test]
    fn test_undeclared_flagged_in_ann_assign_and_raise() {
        let undeclared = analyze_with_conftest("    x: int = my_fixture\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest("    raise ValueError(my_fixture)\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }

    #[test]
    fn test_undeclared_flagged_in_try_and_comprehension_iterable() {
        let undeclared = analyze_with_conftest(
            "    try:\n        _ = my_fixture\n    except KeyError:\n        pass\n",
        );
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest("    x = [i for i in my_fixture]\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }

    #[test]
    fn test_walrus_target_not_flagged() {
        // `(my_fixture := 5)` binds a local; neither the binding nor a later
        // read of it should be flagged as an undeclared fixture.
        let undeclared =
            analyze_with_conftest("    if (my_fixture := 5):\n        _ = my_fixture\n");
        assert!(
            undeclared.iter().all(|u| u.name != "my_fixture"),
            "walrus binding should suppress undeclared flag, got {:?}",
            undeclared
        );
    }

    #[test]
    fn test_undeclared_flagged_in_match_and_orelse_blocks() {
        let undeclared =
            analyze_with_conftest("    match my_fixture:\n        case _:\n            pass\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared =
            analyze_with_conftest("    match 1:\n        case _:\n            _ = my_fixture\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest(
            "    for i in []:\n        pass\n    else:\n        _ = my_fixture\n",
        );
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest(
            "    while False:\n        pass\n    else:\n        _ = my_fixture\n",
        );
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }

    #[test]
    fn test_undeclared_flagged_in_raise_from_and_finally() {
        let undeclared = analyze_with_conftest("    raise ValueError() from my_fixture\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared =
            analyze_with_conftest("    try:\n        pass\n    finally:\n        _ = my_fixture\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest(
            "    try:\n        pass\n    except ValueError:\n        pass\n    else:\n        _ = my_fixture\n",
        );
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }

    #[test]
    fn test_undeclared_flagged_in_set_slice_and_starred() {
        let undeclared = analyze_with_conftest("    x = {my_fixture}\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest("    x = [1, 2][my_fixture:]\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));

        let undeclared = analyze_with_conftest("    x = [*my_fixture]\n");
        assert!(undeclared.iter().any(|u| u.name == "my_fixture"));
    }

    #[test]
    fn test_walrus_targets_collected_through_containers() {
        // Walrus bindings nested in calls, comparisons, boolean/unary/binary
        // ops, tuples and ternaries all register as locals. The target shadows
        // a known fixture name so the test fails if collection stops working
        // (an uncollected `my_fixture` read would be flagged as undeclared).
        for body in [
            "    x = f((my_fixture := 1))\n    _ = my_fixture\n",
            "    x = (my_fixture := 1) < 2\n    _ = my_fixture\n",
            "    x = not (my_fixture := 1)\n    _ = my_fixture\n",
            "    x = (my_fixture := 1) + 1\n    _ = my_fixture\n",
            "    x = ((my_fixture := 1), 2)\n    _ = my_fixture\n",
            "    x = 1 if (my_fixture := 2) else 3\n    _ = my_fixture\n",
        ] {
            let undeclared = analyze_with_conftest(body);
            assert!(
                undeclared.iter().all(|u| u.name != "my_fixture"),
                "walrus target should be a local in {body:?}, got {undeclared:?}"
            );
        }
    }

    #[test]
    fn test_is_available_fixture_same_file() {
        let db = FixtureDatabase::new();
        let conftest_path = PathBuf::from("/tmp/pls_avail/conftest.py");
        db.analyze_file(
            conftest_path.clone(),
            "import pytest\n\n@pytest.fixture\ndef same_file_fixture():\n    return 1\n",
        );
        assert!(db.is_available_fixture(&conftest_path, "same_file_fixture"));
        assert!(!db.is_available_fixture(&conftest_path, "nonexistent_fixture"));
    }

    #[test]
    fn test_is_available_fixture_third_party() {
        use crate::fixtures::types::FixtureDefinition;

        let db = FixtureDatabase::new();
        db.definitions.insert(
            "third_party_fixture".to_string(),
            vec![FixtureDefinition {
                name: "third_party_fixture".to_string(),
                file_path: PathBuf::from("/site-packages/pkg/fixtures.py"),
                is_third_party: true,
                ..Default::default()
            }],
        );
        // Not in same file nor conftest parent, but flagged third_party → available.
        let consumer = PathBuf::from("/tmp/pls_avail/test_foo.py");
        assert!(db.is_available_fixture(&consumer, "third_party_fixture"));
    }
}
