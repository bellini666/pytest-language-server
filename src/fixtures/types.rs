//! Data structures for fixture definitions, usages, and related types.

use std::path::PathBuf;

/// Pytest fixture scope, ordered from narrowest to broadest.
/// A fixture with a broader scope cannot depend on a fixture with a narrower scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixtureScope {
    /// Function scope (default) - created once per test function
    #[default]
    Function = 0,
    /// Class scope - created once per test class
    Class = 1,
    /// Module scope - created once per test module
    Module = 2,
    /// Package scope - created once per test package
    Package = 3,
    /// Session scope - created once per test session
    Session = 4,
}

impl FixtureScope {
    /// Parse scope from a string (as used in @pytest.fixture(scope="..."))
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "function" => Some(Self::Function),
            "class" => Some(Self::Class),
            "module" => Some(Self::Module),
            "package" => Some(Self::Package),
            "session" => Some(Self::Session),
            _ => None,
        }
    }

    /// Get display name for the scope
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Class => "class",
            Self::Module => "module",
            Self::Package => "package",
            Self::Session => "session",
        }
    }
}

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
    pub dependencies: Vec<String>, // Names of fixtures this fixture depends on (via parameters)
    pub scope: FixtureScope,  // The fixture's scope (function, class, module, package, session)
    pub yield_line: Option<usize>, // Line number of the yield statement (for generator fixtures)
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

/// A circular dependency between fixtures.
#[derive(Debug, Clone)]
pub struct FixtureCycle {
    /// The chain of fixtures forming the cycle (e.g., ["A", "B", "C", "A"]).
    pub cycle_path: Vec<String>,
    /// The fixture where the cycle was detected (first fixture in the cycle).
    pub fixture: FixtureDefinition,
}

/// A scope mismatch where a broader-scoped fixture depends on a narrower-scoped fixture.
#[derive(Debug, Clone)]
pub struct ScopeMismatch {
    /// The fixture with broader scope that has the invalid dependency.
    pub fixture: FixtureDefinition,
    /// The dependency fixture with narrower scope.
    pub dependency: FixtureDefinition,
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
