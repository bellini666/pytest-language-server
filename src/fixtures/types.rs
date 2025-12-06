//! Data structures for fixture definitions, usages, and related types.

use std::path::PathBuf;

/// A fixture definition extracted from a Python file.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureDefinition {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize, // Character position where the fixture name starts (on the line)
    pub end_char: usize,   // Character position where the fixture name ends (on the line)
    pub docstring: Option<String>,
    pub return_type: Option<String>, // The return type annotation (for generators, the yielded type)
    pub is_third_party: bool, // Whether this fixture is from a third-party package (site-packages)
}

/// A fixture usage (reference) in a Python file.
#[derive(Debug, Clone)]
pub struct FixtureUsage {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize, // Character position where this usage starts (on the line)
    pub end_char: usize,   // Character position where this usage ends (on the line)
}

/// An undeclared fixture used in a function body without being declared as a parameter.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for debugging and future features
pub struct UndeclaredFixture {
    pub name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub function_name: String, // Name of the test/fixture function where this is used
    pub function_line: usize,  // Line where the function is defined
}

/// Context for code completion.
#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    /// Inside a function signature (parameter list) - suggest fixtures as parameters.
    FunctionSignature {
        function_name: String,
        function_line: usize,
        is_fixture: bool,
        declared_params: Vec<String>,
    },
    /// Inside a function body - suggest fixtures with auto-add to parameters.
    FunctionBody {
        function_name: String,
        function_line: usize,
        is_fixture: bool,
        declared_params: Vec<String>,
    },
    /// Inside @pytest.mark.usefixtures("...") decorator - suggest fixture names as strings.
    UsefixuturesDecorator,
    /// Inside @pytest.mark.parametrize(..., indirect=...) - suggest fixture names as strings.
    ParametrizeIndirect,
}

/// Information about where to insert a new parameter in a function signature.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInsertionInfo {
    /// Line number (1-indexed) where the function signature is.
    pub line: usize,
    /// Character position where the new parameter should be inserted.
    pub char_pos: usize,
    /// Whether a comma needs to be added before the new parameter.
    pub needs_comma: bool,
}
