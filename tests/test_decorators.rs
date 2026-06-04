//! Unit tests for decorator analysis utilities.
//!
//! All tests have a 30-second timeout to prevent hangs from blocking CI.

use ntest::timeout;
use pytest_language_server::fixtures::decorators;
use rustpython_parser::{parse, Mode};

#[test]
#[timeout(30000)]
fn test_is_fixture_decorator_simple() {
    let code = "@fixture\ndef my_fixture(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_fixture_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_is_fixture_decorator_pytest_dot() {
    let code = "@pytest.fixture\ndef my_fixture(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_fixture_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_is_fixture_decorator_with_args() {
    let code = "@pytest.fixture(scope='session')\ndef my_fixture(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_fixture_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_is_fixture_decorator_pytest_asyncio() {
    // Test @pytest_asyncio.fixture (no parens)
    let code = "@pytest_asyncio.fixture\nasync def my_fixture(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::AsyncFunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_fixture_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_is_fixture_decorator_pytest_asyncio_with_args() {
    // Test @pytest_asyncio.fixture(scope='session')
    let code = "@pytest_asyncio.fixture(scope='session')\nasync def my_fixture(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::AsyncFunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_fixture_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_not_fixture_decorator() {
    let code = "@property\ndef my_prop(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            assert!(!decorators::is_fixture_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_custom_fixture_name() {
    let code = "@pytest.fixture(name='custom')\ndef my_fixture(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            let name = decorators::extract_fixture_name_from_decorator(&func_def.decorator_list[0]);
            assert_eq!(name, Some("custom".to_string()));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_is_usefixtures_decorator() {
    let code = "@pytest.mark.usefixtures('f1')\ndef test_x(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_usefixtures_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_usefixtures() {
    let code = "@pytest.mark.usefixtures('f1', 'f2')\ndef test_x(): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            let names = decorators::extract_usefixtures_names(&func_def.decorator_list[0]);
            assert_eq!(names.len(), 2);
            assert_eq!(names[0].0, "f1");
            assert_eq!(names[1].0, "f2");
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_usefixtures_from_expr_direct_call() {
    let code = "pytestmark = pytest.mark.usefixtures('f1', 'f2')";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::Assign(assign) = &module.body[0] {
            let names = decorators::extract_usefixtures_from_expr(&assign.value);
            assert_eq!(names.len(), 2);
            assert_eq!(names[0].0, "f1");
            assert_eq!(names[1].0, "f2");
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_usefixtures_from_expr_list() {
    let code = "pytestmark = [pytest.mark.usefixtures('f1'), pytest.mark.skip, pytest.mark.usefixtures('f2')]";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::Assign(assign) = &module.body[0] {
            let names = decorators::extract_usefixtures_from_expr(&assign.value);
            assert_eq!(names.len(), 2);
            assert_eq!(names[0].0, "f1");
            assert_eq!(names[1].0, "f2");
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_usefixtures_from_expr_tuple() {
    let code = "pytestmark = (pytest.mark.usefixtures('f1'), pytest.mark.usefixtures('f2'))";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::Assign(assign) = &module.body[0] {
            let names = decorators::extract_usefixtures_from_expr(&assign.value);
            assert_eq!(names.len(), 2);
            assert_eq!(names[0].0, "f1");
            assert_eq!(names[1].0, "f2");
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_usefixtures_from_expr_no_usefixtures() {
    let code = "pytestmark = [pytest.mark.skip, pytest.mark.slow]";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::Assign(assign) = &module.body[0] {
            let names = decorators::extract_usefixtures_from_expr(&assign.value);
            assert_eq!(names.len(), 0);
        }
    }
}

#[test]
#[timeout(30000)]
fn test_is_parametrize_decorator() {
    let code = "@pytest.mark.parametrize('x', [1])\ndef test_x(x): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            assert!(decorators::is_parametrize_decorator(
                &func_def.decorator_list[0]
            ));
        }
    }
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_indirect() {
    let code = "@pytest.mark.parametrize('f1', ['a'], indirect=True)\ndef test_x(f1): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();

    if let rustpython_parser::ast::Mod::Module(module) = parsed {
        if let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] {
            let fixtures =
                decorators::extract_parametrize_indirect_fixtures(&func_def.decorator_list[0]);
            assert_eq!(fixtures.len(), 1);
            assert_eq!(fixtures[0].0, "f1");
        }
    }
}

/// Parse a single decorated function and return `(name, source_slice)` pairs from
/// `extract_parametrize_argnames`, where `source_slice` is the exact substring the returned
/// range points at — so tests can confirm ranges land on the identifier, not quotes/whitespace.
fn argnames_with_slices(code: &str) -> Vec<(String, String)> {
    let parsed = parse(code, Mode::Module, "").unwrap();
    let rustpython_parser::ast::Mod::Module(module) = parsed else {
        panic!("expected module");
    };
    let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] else {
        panic!("expected function def");
    };
    func_def
        .decorator_list
        .iter()
        .flat_map(|dec| decorators::extract_parametrize_argnames(dec, code))
        .map(|(name, range)| {
            let slice = code[range.start().to_usize()..range.end().to_usize()].to_string();
            (name, slice)
        })
        .collect()
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_single() {
    let got = argnames_with_slices("@pytest.mark.parametrize('x', [1])\ndef test_x(x): pass");
    assert_eq!(got, vec![("x".to_string(), "x".to_string())]);
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_comma_no_space() {
    let got =
        argnames_with_slices("@pytest.mark.parametrize('a,b', [(1, 2)])\ndef test_x(a, b): pass");
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string())
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_comma_with_spaces() {
    // The ranges must skip the surrounding whitespace and land on the identifiers.
    let got = argnames_with_slices(
        "@pytest.mark.parametrize('a,  b ', [(1, 2)])\ndef test_x(a, b): pass",
    );
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string())
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_list() {
    let got = argnames_with_slices(
        "@pytest.mark.parametrize(['a', 'b'], [(1, 2)])\ndef test_x(a, b): pass",
    );
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string())
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_tuple() {
    let got = argnames_with_slices(
        "@pytest.mark.parametrize(('a', 'b'), [(1, 2)])\ndef test_x(a, b): pass",
    );
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string())
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_keyword() {
    let got = argnames_with_slices(
        "@pytest.mark.parametrize(argnames='x', argvalues=[1])\ndef test_x(x): pass",
    );
    assert_eq!(got, vec![("x".to_string(), "x".to_string())]);
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_stacked() {
    let code = "@pytest.mark.parametrize('a', [1])\n@pytest.mark.parametrize('b', [2])\ndef test_x(a, b): pass";
    let got = argnames_with_slices(code);
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string())
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_not_parametrize() {
    let got = argnames_with_slices("@pytest.mark.usefixtures('x')\ndef test_x(): pass");
    assert!(got.is_empty());
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_triple_quoted() {
    // The range must skip all three opening quotes and land on the identifier.
    let got = argnames_with_slices(
        "@pytest.mark.parametrize('''a, b''', [(1, 2)])\ndef test_x(a, b): pass",
    );
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "b".to_string())
        ]
    );
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_raw_string_prefix() {
    // The `r` prefix must be skipped along with the quote.
    let got =
        argnames_with_slices("@pytest.mark.parametrize(r\"foo\", [1])\ndef test_x(foo): pass");
    assert_eq!(got, vec![("foo".to_string(), "foo".to_string())]);
}

#[test]
#[timeout(30000)]
fn test_extract_parametrize_argnames_rejects_non_identifier() {
    // Implicitly concatenated literals can't be cleanly located, so nothing is returned rather
    // than a corrupting range.
    let got = argnames_with_slices("@pytest.mark.parametrize('a' 'b', [1])\ndef test_x(ab): pass");
    assert!(got.is_empty());
}

#[test]
#[timeout(30000)]
fn test_indirect_names_keyword_argnames() {
    let code = "@pytest.mark.parametrize(argnames='a,b', argvalues=[(1, 2)], indirect=True)\ndef test_x(a, b): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();
    let rustpython_parser::ast::Mod::Module(module) = parsed else {
        panic!("expected module");
    };
    let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] else {
        panic!("expected function def");
    };
    let dec = &func_def.decorator_list[0];
    let names: Vec<String> = decorators::extract_parametrize_argnames(dec, code)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let indirect = decorators::extract_parametrize_indirect_names(dec, &names);
    assert!(indirect.contains("a"));
    assert!(indirect.contains("b"));
}

#[test]
#[timeout(30000)]
fn test_indirect_names_partial_list() {
    let code = "@pytest.mark.parametrize('a,b', [(1, 2)], indirect=['a'])\ndef test_x(a, b): pass";
    let parsed = parse(code, Mode::Module, "").unwrap();
    let rustpython_parser::ast::Mod::Module(module) = parsed else {
        panic!("expected module");
    };
    let rustpython_parser::ast::Stmt::FunctionDef(func_def) = &module.body[0] else {
        panic!("expected function def");
    };
    let dec = &func_def.decorator_list[0];
    let names: Vec<String> = decorators::extract_parametrize_argnames(dec, code)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let indirect = decorators::extract_parametrize_indirect_names(dec, &names);
    assert!(indirect.contains("a"));
    assert!(!indirect.contains("b"));
}

/// Returns the indirect-name set for the first decorator of a single decorated function.
fn indirect_names(code: &str) -> std::collections::HashSet<String> {
    let parsed = parse(code, Mode::Module, "").unwrap();
    let rustpython_parser::ast::Mod::Module(module) = parsed else {
        panic!("expected module");
    };
    let dec = match &module.body[0] {
        rustpython_parser::ast::Stmt::FunctionDef(f) => &f.decorator_list[0],
        rustpython_parser::ast::Stmt::AsyncFunctionDef(f) => &f.decorator_list[0],
        _ => panic!("expected function def"),
    };
    let names: Vec<String> = decorators::extract_parametrize_argnames(dec, code)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    decorators::extract_parametrize_indirect_names(dec, &names)
}

#[test]
#[timeout(30000)]
fn test_indirect_names_tuple_form() {
    let names = indirect_names(
        "@pytest.mark.parametrize('a,b', [(1, 2)], indirect=('a', 'b'))\ndef test_x(a, b): pass",
    );
    assert!(names.contains("a"));
    assert!(names.contains("b"));
}

#[test]
#[timeout(30000)]
fn test_indirect_names_positional() {
    // indirect passed as the third positional argument.
    let names = indirect_names("@pytest.mark.parametrize('a', [1], True)\ndef test_x(a): pass");
    assert!(names.contains("a"));
}

#[test]
#[timeout(30000)]
fn test_indirect_names_absent_or_false() {
    assert!(indirect_names("@pytest.mark.parametrize('a', [1])\ndef test_x(a): pass").is_empty());
    assert!(indirect_names(
        "@pytest.mark.parametrize('a', [1], indirect=False)\ndef test_x(a): pass"
    )
    .is_empty());
}

#[test]
#[timeout(30000)]
fn test_argnames_non_string_forms_ignored() {
    // A non-string/list/tuple argnames expression (e.g. a variable) yields nothing.
    assert!(
        argnames_with_slices("@pytest.mark.parametrize(NAMES, [1])\ndef test_x(a): pass")
            .is_empty()
    );
    // List elements that are not string literals are skipped, whether they are non-constant
    // expressions or non-string constants.
    let got =
        argnames_with_slices("@pytest.mark.parametrize([NAME, 'b'], [1])\ndef test_x(a, b): pass");
    assert_eq!(got, vec![("b".to_string(), "b".to_string())]);
    let got =
        argnames_with_slices("@pytest.mark.parametrize([1, 'b'], [1])\ndef test_x(a, b): pass");
    assert_eq!(got, vec![("b".to_string(), "b".to_string())]);
}

#[test]
#[timeout(30000)]
fn test_indirect_names_ignores_non_string_list_elements() {
    // Non-string entries in an indirect list are ignored.
    let names = indirect_names(
        "@pytest.mark.parametrize('a,b', [(1, 2)], indirect=[1, other])\ndef test_x(a, b): pass",
    );
    assert!(names.is_empty());
}
