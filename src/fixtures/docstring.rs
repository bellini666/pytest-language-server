//! Docstring and return type extraction from Python AST.
//!
//! This module handles extracting documentation and type information
//! from Python function definitions.

use super::FixtureDatabase;
use rustpython_parser::ast::{Expr, Stmt};

impl FixtureDatabase {
    /// Extract docstring from a function body.
    /// The docstring is the first statement if it's a string literal.
    pub(crate) fn extract_docstring(&self, body: &[Stmt]) -> Option<String> {
        if let Some(Stmt::Expr(expr_stmt)) = body.first() {
            if let Expr::Constant(constant) = &*expr_stmt.value {
                if let rustpython_parser::ast::Constant::Str(s) = &constant.value {
                    return Some(super::string_utils::format_docstring(s.to_string()));
                }
            }
        }
        None
    }

    /// Extract return type from a function's return annotation.
    /// For yield fixtures (generators), extracts the yielded type from Generator[T, ...].
    pub(crate) fn extract_return_type(
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

    /// Check if a function body contains yield statements.
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn contains_yield(&self, body: &[Stmt]) -> bool {
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

    /// Extract the yielded type from a Generator/Iterator type annotation.
    /// For Generator[T, None, None] or Iterator[T], returns T.
    fn extract_yielded_type(
        &self,
        expr: &rustpython_parser::ast::Expr,
        content: &str,
    ) -> Option<String> {
        if let Expr::Subscript(subscript) = expr {
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

    /// Convert a Python type expression AST node to a string representation.
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn expr_to_string(
        &self,
        expr: &rustpython_parser::ast::Expr,
        content: &str,
    ) -> String {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Analyze a Python snippet defining a single fixture and return its
    /// recorded `return_type`.
    fn fixture_return_type(source: &str) -> Option<String> {
        let db = FixtureDatabase::new();
        let path = std::env::temp_dir().join("pls_docstring_unit/conftest.py");
        db.analyze_file(path, source);
        db.definitions
            .get("fx")
            .and_then(|defs| defs.value().first().cloned())
            .and_then(|d| d.return_type)
    }

    #[test]
    fn test_return_type_simple_name() {
        assert_eq!(
            fixture_return_type("import pytest\n@pytest.fixture\ndef fx() -> int:\n    return 1\n"),
            Some("int".to_string())
        );
    }

    #[test]
    fn test_return_type_attribute() {
        // Attribute → `pathlib.Path`
        assert_eq!(
            fixture_return_type(
                "import pytest\nimport pathlib\n@pytest.fixture\ndef fx() -> pathlib.Path:\n    return pathlib.Path()\n"
            ),
            Some("pathlib.Path".to_string())
        );
    }

    #[test]
    fn test_return_type_subscript() {
        // `list[int]` → "list[int]"
        assert_eq!(
            fixture_return_type(
                "import pytest\n@pytest.fixture\ndef fx() -> list[int]:\n    return []\n"
            ),
            Some("list[int]".to_string())
        );
    }

    #[test]
    fn test_return_type_nested_subscript() {
        assert_eq!(
            fixture_return_type(
                "import pytest\n@pytest.fixture\ndef fx() -> dict[str, list[int]]:\n    return {}\n"
            ),
            Some("dict[str, list[int]]".to_string())
        );
    }

    #[test]
    fn test_return_type_bitor_union() {
        // `int | str` → "int | str" (covers the BinOp BitOr branch).
        assert_eq!(
            fixture_return_type(
                "import pytest\n@pytest.fixture\ndef fx() -> int | str:\n    return 1\n"
            ),
            Some("int | str".to_string())
        );
    }

    #[test]
    fn test_return_type_generator_tuple_slice_extracts_first_arg() {
        // `Generator[int, None, None]` with yield → extract `int`.
        let ret = fixture_return_type(
            "import pytest\nfrom typing import Generator\n@pytest.fixture\ndef fx() -> Generator[int, None, None]:\n    yield 1\n",
        );
        assert_eq!(ret, Some("int".to_string()));
    }

    #[test]
    fn test_return_type_iterator_single_slice() {
        // `Iterator[str]` with yield → extract `str` (non-Tuple slice branch).
        let ret = fixture_return_type(
            "import pytest\nfrom typing import Iterator\n@pytest.fixture\ndef fx() -> Iterator[str]:\n    yield \"x\"\n",
        );
        assert_eq!(ret, Some("str".to_string()));
    }

    #[test]
    fn test_return_type_non_subscript_generator_falls_through() {
        // Plain `Generator` without subscript → falls through to expr_to_string.
        let ret = fixture_return_type(
            "import pytest\nfrom typing import Generator\n@pytest.fixture\ndef fx() -> Generator:\n    yield 1\n",
        );
        assert_eq!(ret, Some("Generator".to_string()));
    }

    #[test]
    fn test_extract_docstring_picks_up_first_string() {
        let db = FixtureDatabase::new();
        let path = PathBuf::from("/tmp/pls_docstring_unit/conftest_doc.py");
        db.analyze_file(
            path,
            "import pytest\n@pytest.fixture\ndef fx():\n    \"\"\"The docstring.\"\"\"\n    return 1\n",
        );
        let doc = db
            .definitions
            .get("fx")
            .and_then(|defs| defs.value().first().cloned())
            .and_then(|d| d.docstring);
        assert!(doc.is_some(), "docstring should be captured");
        assert!(doc.unwrap().contains("The docstring"));
    }

    #[test]
    fn test_no_return_type_when_annotation_missing() {
        let ret = fixture_return_type("import pytest\n@pytest.fixture\ndef fx():\n    return 1\n");
        assert!(ret.is_none());
    }
}
