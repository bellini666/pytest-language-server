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

/// Extracts fixture names from @pytest.mark.usefixtures("fix1", "fix2", ...) decorator.
pub fn extract_usefixtures_names(
    expr: &Expr,
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
                    return Some((s.to_string(), c.range));
                }
            }
            None
        })
        .collect()
}

/// Checks if an expression is a pytest.mark.parametrize decorator.
pub fn is_parametrize_decorator(expr: &Expr) -> bool {
    is_pytest_mark_decorator(expr, "parametrize")
}

/// Extracts fixture names from @pytest.mark.parametrize when indirect=True.
pub fn extract_parametrize_indirect_fixtures(
    expr: &Expr,
) -> Vec<(String, rustpython_parser::text_size::TextRange)> {
    let Expr::Call(call) = expr else {
        return vec![];
    };
    if !is_parametrize_decorator(&call.func) {
        return vec![];
    }

    let indirect_value = call.keywords.iter().find_map(|kw| {
        if kw.arg.as_ref().is_some_and(|a| a.as_str() == "indirect") {
            Some(&kw.value)
        } else {
            None
        }
    });

    let Some(indirect) = indirect_value else {
        return vec![];
    };

    let Some(first_arg) = call.args.first() else {
        return vec![];
    };

    let Expr::Constant(param_const) = first_arg else {
        return vec![];
    };

    let rustpython_parser::ast::Constant::Str(param_str) = &param_const.value else {
        return vec![];
    };

    let param_names: Vec<&str> = param_str.split(',').map(|s| s.trim()).collect();

    match indirect {
        Expr::Constant(c) => {
            if matches!(c.value, rustpython_parser::ast::Constant::Bool(true)) {
                return param_names
                    .into_iter()
                    .map(|name| (name.to_string(), param_const.range))
                    .collect();
            }
        }
        Expr::List(list) => {
            return list
                .elts
                .iter()
                .filter_map(|elt| {
                    if let Expr::Constant(c) = elt {
                        if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                            if param_names.contains(&s.as_str()) {
                                return Some((s.to_string(), c.range));
                            }
                        }
                    }
                    None
                })
                .collect();
        }
        _ => {}
    }

    vec![]
}
