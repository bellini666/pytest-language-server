//! Decorator analysis utilities for pytest fixtures.
//!
//! This module contains shared logic for recognizing and extracting information
//! from pytest decorators like @pytest.fixture, @pytest.mark.usefixtures, etc.

use rustpython_parser::ast::Expr;

/// Check if an expression is a @pytest.fixture or @pytest_asyncio.fixture decorator
pub fn is_fixture_decorator(expr: &Expr) -> bool {
    match expr {
        Expr::Name(name) => name.id.as_str() == "fixture",
        Expr::Attribute(attr) => {
            if let Expr::Name(value) = &*attr.value {
                (value.id.as_str() == "pytest" || value.id.as_str() == "pytest_asyncio")
                    && attr.attr.as_str() == "fixture"
            } else {
                false
            }
        }
        Expr::Call(call) => is_fixture_decorator(&call.func),
        _ => false,
    }
}

/// Extracts the fixture name from a decorator's `name=` argument if present.
pub fn extract_fixture_name_from_decorator(expr: &Expr) -> Option<String> {
    let Expr::Call(call) = expr else { return None };
    if !is_fixture_decorator(&call.func) {
        return None;
    }

    call.keywords
        .iter()
        .filter(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "name"))
        .find_map(|kw| match &kw.value {
            Expr::Constant(c) => match &c.value {
                rustpython_parser::ast::Constant::Str(s) => Some(s.to_string()),
                _ => None,
            },
            _ => None,
        })
}

/// Checks if an expression is a pytest.mark.* decorator with the given marker name.
/// This is a helper function to avoid duplicating the decorator matching logic.
fn is_pytest_mark_decorator(expr: &Expr, marker_name: &str) -> bool {
    match expr {
        Expr::Call(call) => is_pytest_mark_decorator(&call.func, marker_name),
        Expr::Attribute(attr) => {
            if attr.attr.as_str() != marker_name {
                return false;
            }
            match &*attr.value {
                Expr::Attribute(inner_attr) => {
                    if inner_attr.attr.as_str() != "mark" {
                        return false;
                    }
                    matches!(&*inner_attr.value, Expr::Name(name) if name.id.as_str() == "pytest")
                }
                Expr::Name(name) => name.id.as_str() == "mark",
                _ => false,
            }
        }
        _ => false,
    }
}

/// Checks if an expression is a pytest.mark.usefixtures decorator.
pub fn is_usefixtures_decorator(expr: &Expr) -> bool {
    is_pytest_mark_decorator(expr, "usefixtures")
}

/// Returns the range of a string literal's content, without the surrounding
/// prefix (`r`, `b`, `u`, `f`) and quote run (`'`/`"`, single or triple).
///
/// `literal` is the literal's exact source text and `range` its full range;
/// falls back to the full range when the text doesn't look like a string.
fn literal_content_range(
    literal: &str,
    range: rustpython_parser::text_size::TextRange,
) -> rustpython_parser::text_size::TextRange {
    use rustpython_parser::text_size::{TextRange, TextSize};

    let bytes = literal.as_bytes();
    let mut prefix = 0;
    while prefix < bytes.len()
        && matches!(
            bytes[prefix],
            b'r' | b'R' | b'b' | b'B' | b'u' | b'U' | b'f' | b'F'
        )
    {
        prefix += 1;
    }
    let Some(&quote) = bytes.get(prefix) else {
        return range;
    };
    if quote != b'"' && quote != b'\'' {
        return range;
    }
    let quote_len =
        if bytes.len() >= prefix + 3 && bytes[prefix + 1] == quote && bytes[prefix + 2] == quote {
            3
        } else {
            1
        };

    let content_offset = prefix + quote_len;
    let content_end = literal.len().saturating_sub(quote_len);
    if content_offset > content_end {
        return range;
    }
    TextRange::new(
        range.start() + TextSize::from(content_offset as u32),
        range.start() + TextSize::from(content_end as u32),
    )
}

/// Extracts fixture names from @pytest.mark.usefixtures("fix1", "fix2", ...) decorator.
///
/// Each name is paired with the range of the string literal's *content*
/// (quotes and any string prefix excluded), so the range covers exactly the
/// fixture name. `content` is the full source of the file the decorator is in.
pub fn extract_usefixtures_names(
    expr: &Expr,
    content: &str,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    let Expr::Call(call) = expr else {
        return vec![];
    };
    if !is_usefixtures_decorator(&call.func) {
        return vec![];
    }

    call.args
        .iter()
        .filter_map(|arg| {
            if let Expr::Constant(c) = arg {
                if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                    let literal = content
                        .get(c.range.start().to_usize()..c.range.end().to_usize())
                        .unwrap_or("");
                    return Some((s.to_string(), literal_content_range(literal, c.range)));
                }
            }
            None
        })
        .collect()
}

/// Extracts fixture names from usefixtures calls within any expression,
/// including nested structures like lists and tuples.
/// This handles patterns like:
///   pytestmark = pytest.mark.usefixtures("fix1")
///   pytestmark = [pytest.mark.usefixtures("fix1"), pytest.mark.skip]
///   pytestmark = (pytest.mark.usefixtures("fix1"), pytest.mark.usefixtures("fix2"))
pub fn extract_usefixtures_from_expr(
    expr: &Expr,
    content: &str,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    match expr {
        // Direct call: pytest.mark.usefixtures("fix1", "fix2")
        Expr::Call(_) => extract_usefixtures_names(expr, content),
        // List: [pytest.mark.usefixtures("fix1"), ...]
        Expr::List(list) => list
            .elts
            .iter()
            .flat_map(|e| extract_usefixtures_from_expr(e, content))
            .collect(),
        // Tuple: (pytest.mark.usefixtures("fix1"), ...)
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .flat_map(|e| extract_usefixtures_from_expr(e, content))
            .collect(),
        _ => vec![],
    }
}

/// Checks if an expression is a pytest.mark.parametrize decorator.
pub fn is_parametrize_decorator(expr: &Expr) -> bool {
    is_pytest_mark_decorator(expr, "parametrize")
}

/// Returns true if `name` is a plain Python identifier (the only thing a parametrize argname can
/// legally be). Used to reject anything we couldn't cleanly locate in the source, e.g. implicitly
/// concatenated string literals, so a rename never corrupts the file.
/// Unicode-aware to match `is_valid_python_identifier` in the rename provider.
fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// Splits the *source text* of a string literal into argnames, each paired with the precise
/// [`TextRange`] of its identifier token.
///
/// Working from the source (rather than the parsed value) keeps the ranges correct regardless of
/// quote style: `range_start` is the offset of the literal's opening prefix/quote, so the actual
/// content offset is derived by skipping any string prefix (`r`, `b`, `u`, `f`) and the quote run
/// (`'`, `"`, or their triple variants).
fn split_argnames_from_source(
    literal: &str,
    range_start: rustpython_parser::text_size::TextSize,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    use rustpython_parser::text_size::{TextRange, TextSize};

    let bytes = literal.as_bytes();
    let mut prefix = 0;
    while prefix < bytes.len()
        && matches!(
            bytes[prefix],
            b'r' | b'R' | b'b' | b'B' | b'u' | b'U' | b'f' | b'F'
        )
    {
        prefix += 1;
    }
    let Some(&quote) = bytes.get(prefix) else {
        return vec![];
    };
    if quote != b'"' && quote != b'\'' {
        return vec![];
    }
    let quote_len =
        if bytes.len() >= prefix + 3 && bytes[prefix + 1] == quote && bytes[prefix + 2] == quote {
            3
        } else {
            1
        };

    let content_offset = prefix + quote_len;
    let content_end = literal.len().saturating_sub(quote_len);
    if content_offset > content_end {
        return vec![];
    }
    let inner = &literal[content_offset..content_end];

    let mut result = Vec::new();
    let mut offset = content_offset;
    for segment in inner.split(',') {
        let leading_ws = segment.len() - segment.trim_start().len();
        let trimmed = segment.trim();
        if is_plain_identifier(trimmed) {
            let start = range_start + TextSize::from((offset + leading_ws) as u32);
            let end = start + TextSize::from(trimmed.len() as u32);
            result.push((trimmed.to_string(), TextRange::new(start, end)));
        }
        offset += segment.len() + 1; // +1 for the comma separator
    }
    result
}

/// Extracts the declared parameter names from a `@pytest.mark.parametrize(...)` decorator, each
/// paired with the precise [`TextRange`] of its name token.
///
/// Handles every argnames form pytest accepts: a single name, a comma-separated string
/// (`"a,b"` / `"a, b"`), a list or tuple of strings, and `argnames=` passed as a keyword.
/// `content` is the full source of the file the decorator came from, used to read each string
/// literal's exact text.
pub fn extract_parametrize_argnames(
    expr: &Expr,
    content: &str,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    let Expr::Call(call) = expr else {
        return vec![];
    };
    if !is_parametrize_decorator(&call.func) {
        return vec![];
    }

    let argnames = call.args.first().or_else(|| {
        call.keywords
            .iter()
            .find(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "argnames"))
            .map(|kw| &kw.value)
    });

    let Some(argnames) = argnames else {
        return vec![];
    };

    match argnames {
        Expr::Constant(_) => parametrize_name_element_ranges(argnames, content),
        Expr::List(list) => list
            .elts
            .iter()
            .flat_map(|elt| parametrize_name_element_ranges(elt, content))
            .collect(),
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .flat_map(|elt| parametrize_name_element_ranges(elt, content))
            .collect(),
        _ => vec![],
    }
}

/// Extracts name/range pairs from a single string-literal element of an argnames spec.
fn parametrize_name_element_ranges(
    elt: &Expr,
    content: &str,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    let Expr::Constant(c) = elt else {
        return vec![];
    };
    if !matches!(c.value, rustpython_parser::ast::Constant::Str(_)) {
        return vec![];
    }
    let start = c.range.start().to_usize();
    let end = c.range.end().to_usize();
    let Some(literal) = content.get(start..end) else {
        return vec![];
    };
    split_argnames_from_source(literal, c.range.start())
}

/// Returns the subset of `argnames` that a `@pytest.mark.parametrize(...)` decorator marks as
/// indirect (`indirect=True` marks all of them; `indirect=[names]` marks the listed ones).
///
/// `indirect` may be passed by keyword or as the third positional argument.
pub fn extract_parametrize_indirect_names(
    expr: &Expr,
    argnames: &[String],
) -> std::collections::HashSet<String> {
    use std::collections::HashSet;

    let Expr::Call(call) = expr else {
        return HashSet::new();
    };
    if !is_parametrize_decorator(&call.func) {
        return HashSet::new();
    }

    let indirect = call
        .keywords
        .iter()
        .find(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "indirect"))
        .map(|kw| &kw.value)
        .or_else(|| call.args.get(2));

    let Some(indirect) = indirect else {
        return HashSet::new();
    };

    match indirect {
        Expr::Constant(c) if matches!(c.value, rustpython_parser::ast::Constant::Bool(true)) => {
            argnames.iter().cloned().collect()
        }
        Expr::List(list) => collect_string_constants(&list.elts),
        Expr::Tuple(tuple) => collect_string_constants(&tuple.elts),
        _ => HashSet::new(),
    }
}

fn collect_string_constants(elts: &[Expr]) -> std::collections::HashSet<String> {
    elts.iter()
        .filter_map(|elt| match elt {
            Expr::Constant(c) => match &c.value {
                rustpython_parser::ast::Constant::Str(s) => Some(s.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Extracts fixture names from @pytest.mark.parametrize when they are marked
/// indirect (`indirect=True` or `indirect=[names]`, keyword or positional).
///
/// Each name is paired with the precise range of its token inside the
/// argnames literal (via [`extract_parametrize_argnames`]), so usages point
/// at the parameter name itself rather than a whole string literal.
pub fn extract_parametrize_indirect_fixtures(
    expr: &Expr,
    content: &str,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    let argnames = extract_parametrize_argnames(expr, content);
    if argnames.is_empty() {
        return vec![];
    }

    let names: Vec<String> = argnames.iter().map(|(name, _)| name.clone()).collect();
    let indirect = extract_parametrize_indirect_names(expr, &names);

    argnames
        .into_iter()
        .filter(|(name, _)| indirect.contains(name))
        .collect()
}

/// Extracts whether autouse=True is set on a @pytest.fixture decorator.
/// Returns false if no autouse keyword is specified or if autouse=False.
pub fn extract_fixture_autouse(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else { return false };
    if !is_fixture_decorator(&call.func) {
        return false;
    }

    call.keywords
        .iter()
        .filter(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "autouse"))
        .any(|kw| matches!(&kw.value, Expr::Constant(c) if matches!(c.value, rustpython_parser::ast::Constant::Bool(true))))
}

/// Extracts the scope from a @pytest.fixture(scope="...") decorator.
/// Returns None if no scope is specified (defaults to "function" at call site).
pub fn extract_fixture_scope(expr: &Expr) -> Option<super::types::FixtureScope> {
    let Expr::Call(call) = expr else { return None };
    if !is_fixture_decorator(&call.func) {
        return None;
    }

    call.keywords
        .iter()
        .filter(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "scope"))
        .find_map(|kw| match &kw.value {
            Expr::Constant(c) => match &c.value {
                rustpython_parser::ast::Constant::Str(s) => super::types::FixtureScope::parse(s),
                _ => None,
            },
            _ => None,
        })
}
