//! Undeclared fixture detection in function bodies.
//!
//! This module scans function bodies for references to fixtures that
//! are not declared as function parameters.

use super::types::UndeclaredFixture;
use super::FixtureDatabase;
use rustpython_parser::ast::{
    ExceptHandler, Expr, Pattern, Stmt, StmtAsyncFor, StmtAsyncFunctionDef, StmtAsyncWith,
    StmtClassDef, StmtFor, StmtFunctionDef, StmtImport, StmtImportFrom, StmtTry, StmtTryStar,
    StmtWith,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::info;

/// Context for scanning function bodies for undeclared fixtures.
/// This reduces the number of arguments passed to recursive functions.
pub(crate) struct BodyScanContext<'a> {
    pub file_path: &'a PathBuf,
    pub line_index: &'a [usize],
    pub declared_params: &'a HashSet<String>,
    pub function_name: &'a str,
    pub function_line: usize,
}

/// Add the names a `match` pattern captures to `bound`.
fn bind_pattern_names(pattern: &Pattern, bound: &mut HashSet<String>) {
    let (patterns, captured): (Vec<&Pattern>, Option<&str>) = match pattern {
        Pattern::MatchSequence(p) => (p.patterns.iter().collect(), None),
        Pattern::MatchMapping(p) => (
            p.patterns.iter().collect(),
            p.rest.as_ref().map(|name| name.as_str()),
        ),
        Pattern::MatchClass(p) => (p.patterns.iter().chain(&p.kwd_patterns).collect(), None),
        Pattern::MatchStar(p) => (Vec::new(), p.name.as_ref().map(|name| name.as_str())),
        Pattern::MatchAs(p) => (
            p.pattern.iter().map(|inner| inner.as_ref()).collect(),
            p.name.as_ref().map(|name| name.as_str()),
        ),
        Pattern::MatchOr(p) => (p.patterns.iter().collect(), None),
        Pattern::MatchValue(_) | Pattern::MatchSingleton(_) => (Vec::new(), None),
    };

    if let Some(name) = captured {
        bound.insert(name.to_string());
    }
    for inner in patterns {
        bind_pattern_names(inner, bound);
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
        // Imported names shouldn't be flagged as undeclared fixtures
        let mut bound: HashSet<String> = self
            .imports
            .get(file_path)
            .map(|imports| imports.iter().cloned().collect())
            .unwrap_or_default();

        let ctx = BodyScanContext {
            file_path,
            line_index,
            declared_params,
            function_name,
            function_line,
        };

        self.visit_body_for_names(body, &ctx, &mut bound);
    }

    fn visit_body_for_names(
        &self,
        body: &[Stmt],
        ctx: &BodyScanContext,
        bound: &mut HashSet<String>,
    ) {
        for stmt in body {
            self.visit_stmt_for_names(stmt, ctx, bound);
        }
    }

    /// Check a statement for undeclared fixture reads, then add what it binds to `bound`.
    ///
    /// `bound` holds the names that may be bound at this point. A name bound on only
    /// some paths through a branch counts as bound after it, and a sibling branch never
    /// sees it.
    fn visit_stmt_for_names(
        &self,
        stmt: &Stmt,
        ctx: &BodyScanContext,
        bound: &mut HashSet<String>,
    ) {
        match stmt {
            Stmt::Expr(expr_stmt) => {
                self.visit_expr_for_names(&expr_stmt.value, ctx, bound);
            }
            Stmt::Assign(assign) => {
                self.visit_expr_for_names(&assign.value, ctx, bound);
                for target in &assign.targets {
                    self.collect_names_from_expr(target, bound);
                }
            }
            Stmt::AugAssign(aug_assign) => {
                self.visit_expr_for_names(&aug_assign.value, ctx, bound);
                self.collect_names_from_expr(&aug_assign.target, bound);
            }
            Stmt::AnnAssign(ann_assign) => {
                if let Some(ref value) = ann_assign.value {
                    self.visit_expr_for_names(value, ctx, bound);
                }
                self.collect_names_from_expr(&ann_assign.target, bound);
            }
            Stmt::Return(ret) => {
                if let Some(ref value) = ret.value {
                    self.visit_expr_for_names(value, ctx, bound);
                }
            }
            Stmt::If(if_stmt) => {
                self.visit_expr_for_names(&if_stmt.test, ctx, bound);

                let mut orelse_bound = bound.clone();
                self.visit_body_for_names(&if_stmt.body, ctx, bound);
                self.visit_body_for_names(&if_stmt.orelse, ctx, &mut orelse_bound);
                bound.extend(orelse_bound);
            }
            Stmt::While(while_stmt) => {
                self.visit_expr_for_names(&while_stmt.test, ctx, bound);
                self.visit_body_for_names(&while_stmt.body, ctx, bound);
                self.visit_body_for_names(&while_stmt.orelse, ctx, bound);
            }
            Stmt::For(StmtFor {
                target,
                iter,
                body,
                orelse,
                ..
            })
            | Stmt::AsyncFor(StmtAsyncFor {
                target,
                iter,
                body,
                orelse,
                ..
            }) => {
                self.visit_expr_for_names(iter, ctx, bound);
                self.collect_names_from_expr(target, bound);
                self.visit_body_for_names(body, ctx, bound);
                self.visit_body_for_names(orelse, ctx, bound);
            }
            Stmt::With(StmtWith { items, body, .. })
            | Stmt::AsyncWith(StmtAsyncWith { items, body, .. }) => {
                for item in items {
                    self.visit_expr_for_names(&item.context_expr, ctx, bound);
                    if let Some(ref optional_vars) = item.optional_vars {
                        self.collect_names_from_expr(optional_vars, bound);
                    }
                }
                self.visit_body_for_names(body, ctx, bound);
            }
            Stmt::Assert(assert_stmt) => {
                self.visit_expr_for_names(&assert_stmt.test, ctx, bound);
                if let Some(ref msg) = assert_stmt.msg {
                    self.visit_expr_for_names(msg, ctx, bound);
                }
            }
            Stmt::Raise(raise_stmt) => {
                if let Some(ref exc) = raise_stmt.exc {
                    self.visit_expr_for_names(exc, ctx, bound);
                }
                if let Some(ref cause) = raise_stmt.cause {
                    self.visit_expr_for_names(cause, ctx, bound);
                }
            }
            Stmt::Try(StmtTry {
                body,
                handlers,
                orelse,
                finalbody,
                ..
            })
            | Stmt::TryStar(StmtTryStar {
                body,
                handlers,
                orelse,
                finalbody,
                ..
            }) => {
                self.visit_body_for_names(body, ctx, bound);
                let after_body = bound.clone();

                for handler in handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    // Several `except*` handlers can run for one exception group.
                    let mut handler_bound = if matches!(stmt, Stmt::TryStar(_)) {
                        bound.clone()
                    } else {
                        after_body.clone()
                    };
                    if let Some(ref name) = h.name {
                        handler_bound.insert(name.to_string());
                    }

                    self.visit_body_for_names(&h.body, ctx, &mut handler_bound);

                    // Python unbinds `except E as name` when the handler ends.
                    if let Some(ref name) = h.name {
                        if !after_body.contains(name.as_str()) {
                            handler_bound.remove(name.as_str());
                        }
                    }
                    bound.extend(handler_bound);
                }

                let mut orelse_bound = after_body;
                self.visit_body_for_names(orelse, ctx, &mut orelse_bound);
                bound.extend(orelse_bound);

                self.visit_body_for_names(finalbody, ctx, bound);
            }
            Stmt::Match(match_stmt) => {
                self.visit_expr_for_names(&match_stmt.subject, ctx, bound);
                let mut next_case_bound = bound.clone();

                for case in &match_stmt.cases {
                    let mut case_bound = next_case_bound.clone();
                    bind_pattern_names(&case.pattern, &mut case_bound);
                    if let Some(ref guard) = case.guard {
                        self.visit_expr_for_names(guard, ctx, &mut case_bound);
                        // Captures stay bound when the guard fails and matching continues.
                        next_case_bound.clone_from(&case_bound);
                    }
                    self.visit_body_for_names(&case.body, ctx, &mut case_bound);
                    bound.extend(case_bound);
                }
            }
            Stmt::Import(StmtImport { names, .. })
            | Stmt::ImportFrom(StmtImportFrom { names, .. }) => {
                for alias in names {
                    let name = match alias.asname {
                        Some(ref asname) => asname.as_str(),
                        None => alias.name.split('.').next().unwrap_or(&alias.name),
                    };
                    bound.insert(name.to_string());
                }
            }
            Stmt::FunctionDef(StmtFunctionDef { name, .. })
            | Stmt::AsyncFunctionDef(StmtAsyncFunctionDef { name, .. })
            | Stmt::ClassDef(StmtClassDef { name, .. }) => {
                bound.insert(name.to_string());
            }
            _ => {}
        }
    }

    /// Visit an expression and check for undeclared fixture references.
    #[allow(clippy::only_used_in_recursion)]
    fn visit_expr_for_names(
        &self,
        expr: &Expr,
        ctx: &BodyScanContext,
        bound: &mut HashSet<String>,
    ) {
        match expr {
            Expr::Name(name) => {
                let name_str = name.id.as_str();
                let line = self.get_line_from_offset(name.range.start().to_usize(), ctx.line_index);

                if !ctx.declared_params.contains(name_str)
                    && !bound.contains(name_str)
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
                self.visit_expr_for_names(&call.func, ctx, bound);
                for arg in &call.args {
                    self.visit_expr_for_names(arg, ctx, bound);
                }
                for keyword in &call.keywords {
                    self.visit_expr_for_names(&keyword.value, ctx, bound);
                }
            }
            Expr::Attribute(attr) => {
                self.visit_expr_for_names(&attr.value, ctx, bound);
            }
            Expr::BinOp(binop) => {
                self.visit_expr_for_names(&binop.left, ctx, bound);
                self.visit_expr_for_names(&binop.right, ctx, bound);
            }
            Expr::UnaryOp(unaryop) => {
                self.visit_expr_for_names(&unaryop.operand, ctx, bound);
            }
            Expr::Compare(compare) => {
                self.visit_expr_for_names(&compare.left, ctx, bound);
                for comparator in &compare.comparators {
                    self.visit_expr_for_names(comparator, ctx, bound);
                }
            }
            Expr::Subscript(subscript) => {
                self.visit_expr_for_names(&subscript.value, ctx, bound);
                self.visit_expr_for_names(&subscript.slice, ctx, bound);
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.visit_expr_for_names(elt, ctx, bound);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.visit_expr_for_names(elt, ctx, bound);
                }
            }
            Expr::Dict(dict) => {
                for (key, value) in dict.keys.iter().zip(&dict.values) {
                    if let Some(key) = key {
                        self.visit_expr_for_names(key, ctx, bound);
                    }
                    self.visit_expr_for_names(value, ctx, bound);
                }
            }
            Expr::Await(await_expr) => {
                self.visit_expr_for_names(&await_expr.value, ctx, bound);
            }
            Expr::BoolOp(bool_op) => {
                for value in &bool_op.values {
                    self.visit_expr_for_names(value, ctx, bound);
                }
            }
            Expr::IfExp(if_exp) => {
                self.visit_expr_for_names(&if_exp.test, ctx, bound);

                let mut orelse_bound = bound.clone();
                self.visit_expr_for_names(&if_exp.body, ctx, bound);
                self.visit_expr_for_names(&if_exp.orelse, ctx, &mut orelse_bound);
                bound.extend(orelse_bound);
            }
            Expr::NamedExpr(named) => {
                // Only the value is a read; the walrus target is a binding.
                self.visit_expr_for_names(&named.value, ctx, bound);
                self.collect_names_from_expr(&named.target, bound);
            }
            Expr::Starred(starred) => {
                self.visit_expr_for_names(&starred.value, ctx, bound);
            }
            Expr::JoinedStr(joined) => {
                for value in &joined.values {
                    self.visit_expr_for_names(value, ctx, bound);
                }
            }
            Expr::FormattedValue(formatted) => {
                self.visit_expr_for_names(&formatted.value, ctx, bound);
            }
            Expr::Set(set) => {
                for elt in &set.elts {
                    self.visit_expr_for_names(elt, ctx, bound);
                }
            }
            Expr::Slice(slice) => {
                for part in [&slice.lower, &slice.upper, &slice.step]
                    .into_iter()
                    .flatten()
                {
                    self.visit_expr_for_names(part, ctx, bound);
                }
            }
            // Comprehensions: only the iterables are visited — elements and
            // conditions typically reference comprehension-local loop targets,
            // which would produce false positives without scope tracking.
            Expr::ListComp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx, bound);
                }
            }
            Expr::SetComp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx, bound);
                }
            }
            Expr::GeneratorExp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx, bound);
                }
            }
            Expr::DictComp(comp) => {
                for generator in &comp.generators {
                    self.visit_expr_for_names(&generator.iter, ctx, bound);
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
    fn test_local_bindings_shadow_fixture_after_first_binding() {
        let cases = [
            (
                "read between assignments",
                "    my_fixture = 1\n    _ = my_fixture\n    my_fixture = 2\n",
            ),
            (
                "assigned in except body",
                "    try:\n        pass\n    except LookupError:\n        my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "except as name",
                "    try:\n        pass\n    except LookupError as my_fixture:\n        _ = my_fixture\n",
            ),
            (
                "except as name on the handler line",
                "    try:\n        pass\n    except LookupError as my_fixture: _ = my_fixture\n",
            ),
            (
                "assigned in except star body",
                "    try:\n        pass\n    except* LookupError:\n        my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "assigned in try, read in handler",
                "    try:\n        my_fixture = 1\n    except LookupError:\n        _ = my_fixture\n",
            ),
            (
                "assigned in try, read in finally",
                "    try:\n        my_fixture = 1\n    finally:\n        _ = my_fixture\n",
            ),
            (
                "assigned in try else, read after",
                "    try:\n        pass\n    except LookupError:\n        pass\n    else:\n        my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "assigned in one branch, read after",
                "    if c:\n        my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "assigned in for else",
                "    for i in []:\n        pass\n    else:\n        my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "assigned in while else",
                "    while c:\n        pass\n    else:\n        my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "assigned in match body",
                "    match c:\n        case 1:\n            my_fixture = 1\n    _ = my_fixture\n",
            ),
            (
                "match capture",
                "    match c:\n        case my_fixture:\n            _ = my_fixture\n",
            ),
            (
                "match as pattern",
                "    match c:\n        case int() as my_fixture:\n            _ = my_fixture\n",
            ),
            (
                "match sequence star",
                "    match c:\n        case [1, *my_fixture]:\n            _ = my_fixture\n",
            ),
            (
                "match mapping rest",
                "    match c:\n        case {\"a\": 1, **my_fixture}:\n            _ = my_fixture\n",
            ),
            (
                "match class keyword pattern",
                "    match c:\n        case C(a=my_fixture):\n            _ = my_fixture\n",
            ),
            (
                "match or pattern",
                "    match c:\n        case [my_fixture] | (my_fixture, 1):\n            _ = my_fixture\n",
            ),
            (
                "match capture used in guard",
                "    match c:\n        case my_fixture if my_fixture:\n            pass\n",
            ),
            (
                "walrus in expression statement",
                "    print(my_fixture := 1)\n    _ = my_fixture\n",
            ),
            (
                "walrus in assert",
                "    assert (my_fixture := 1).real\n    _ = my_fixture\n",
            ),
            (
                "walrus in annotated assignment",
                "    x: int = (my_fixture := 1)\n    _ = my_fixture\n",
            ),
            (
                "walrus in augmented assignment",
                "    x += (my_fixture := 1)\n    _ = my_fixture\n",
            ),
            (
                "walrus in for iterable",
                "    for i in (my_fixture := []):\n        pass\n    _ = my_fixture\n",
            ),
            (
                "walrus in with item",
                "    with (my_fixture := open(\"x\")):\n        _ = my_fixture\n",
            ),
            (
                "walrus nested in list",
                "    x = [(my_fixture := 1)]\n    _ = my_fixture\n",
            ),
            (
                "assigned in except star handler, read in a later one",
                "    try:\n        pass\n    except* ValueError:\n        my_fixture = 1\n    except* TypeError:\n        _ = my_fixture\n",
            ),
            (
                "capture read in the case after a guarded one",
                "    match c:\n        case my_fixture if c:\n            pass\n        case _:\n            _ = my_fixture\n",
            ),
            (
                "walrus in dict value, read in the next key",
                "    _ = {\"a\": (my_fixture := 1), my_fixture: 2}\n",
            ),
            (
                "walrus in ternary branch, read after",
                "    x = (my_fixture := 1) if c else 2\n    _ = my_fixture\n",
            ),
            (
                "walrus read on the same line",
                "    if (my_fixture := 1) and my_fixture:\n        pass\n",
            ),
            (
                "from import alias in body",
                "    from mymod import other as my_fixture\n    _ = my_fixture\n",
            ),
            (
                "import on the same line as the read",
                "    import my_fixture; _ = my_fixture\n",
            ),
            (
                "dotted import in body",
                "    import my_fixture.sub\n    _ = my_fixture\n",
            ),
            (
                "nested def",
                "    def my_fixture():\n        return 2\n    _ = my_fixture()\n",
            ),
            (
                "nested async def",
                "    async def my_fixture():\n        return 2\n    _ = my_fixture()\n",
            ),
            (
                "nested class",
                "    class my_fixture:\n        pass\n    _ = my_fixture()\n",
            ),
        ];

        let flagged: Vec<&str> = cases
            .iter()
            .filter(|(_, body)| {
                analyze_with_conftest(body)
                    .iter()
                    .any(|u| u.name == "my_fixture")
            })
            .map(|(label, _)| *label)
            .collect();

        assert!(flagged.is_empty(), "wrongly flagged: {:?}", flagged);
    }

    #[test]
    fn test_read_no_binding_reaches_stays_flagged() {
        let cases = [
            (
                "before two assignments",
                "    _ = my_fixture\n    my_fixture = 1\n    my_fixture = 2\n",
                2,
            ),
            (
                "before import in body",
                "    _ = my_fixture\n    from mymod import my_fixture\n",
                2,
            ),
            (
                "before nested def",
                "    _ = my_fixture\n    def my_fixture():\n        return 2\n",
                2,
            ),
            (
                "assignment reading the fixture",
                "    my_fixture = my_fixture\n",
                2,
            ),
            (
                "multi-line assignment reading the fixture",
                "    my_fixture = wrap(\n        my_fixture,\n    )\n",
                3,
            ),
            ("call keyword value", "    _ = f(kw=my_fixture)\n", 2),
            (
                "other if branch, assigned later",
                "    if c:\n        my_fixture = 1\n    else:\n        _ = my_fixture\n        my_fixture = 2\n",
                5,
            ),
            (
                "other if branch, imported",
                "    if c:\n        import my_fixture\n    else:\n        _ = my_fixture\n",
                5,
            ),
            (
                "other except handler",
                "    try:\n        pass\n    except ValueError:\n        my_fixture = 1\n    except TypeError:\n        _ = my_fixture\n",
                7,
            ),
            (
                "case after a guarded case's body",
                "    match c:\n        case 1 if c:\n            my_fixture = 1\n        case 2:\n            _ = my_fixture\n",
                6,
            ),
            (
                "dict value before the walrus in the next key",
                "    _ = {\"a\": my_fixture, (my_fixture := 1): 2}\n",
                2,
            ),
            (
                "other ternary branch",
                "    x = (my_fixture := 1) if c else my_fixture\n",
                2,
            ),
            (
                "other match case",
                "    match c:\n        case 1:\n            my_fixture = 1\n        case 2:\n            _ = my_fixture\n",
                6,
            ),
            (
                "after the handler that named it",
                "    try:\n        pass\n    except LookupError as my_fixture:\n        pass\n    _ = my_fixture\n",
                6,
            ),
            (
                "try else after assignment in handler",
                "    try:\n        pass\n    except LookupError:\n        my_fixture = 1\n    else:\n        _ = my_fixture\n",
                7,
            ),
            (
                "inside try star body",
                "    try:\n        _ = my_fixture\n    except* LookupError:\n        pass\n",
                3,
            ),
            (
                "inside async for else",
                "    async for i in []:\n        pass\n    else:\n        _ = my_fixture\n",
                5,
            ),
        ];

        for (label, body, line) in cases {
            let lines: Vec<usize> = analyze_with_conftest(body)
                .iter()
                .filter(|u| u.name == "my_fixture")
                .map(|u| u.line)
                .collect();

            assert_eq!(lines, vec![line], "{}", label);
        }
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
            "    x = f(kw=(my_fixture := 1))\n    _ = my_fixture\n",
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
