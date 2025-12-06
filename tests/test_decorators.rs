//! Unit tests for decorator analysis utilities.

use pytest_language_server::fixtures::decorators;
use rustpython_parser::{parse, Mode};

#[test]
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
