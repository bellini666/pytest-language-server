//! # pytest-language-server
//!
//! A Language Server Protocol (LSP) implementation for pytest fixtures.
//!
//! This crate provides IDE features for pytest fixture development:
//!
//! - **Go to Definition**: Jump to fixture definitions from test functions
//! - **Find References**: Find all usages of a fixture across the codebase
//! - **Hover Documentation**: View fixture docstrings and signatures
//! - **Code Completion**: Auto-complete fixture names in function signatures
//! - **Diagnostics**: Detect undeclared fixtures, scope mismatches, and circular dependencies
//! - **Code Actions**: Quick fixes to add missing fixture parameters
//! - **Code Lens**: Usage counts above fixture definitions
//! - **Inlay Hints**: Show fixture return types inline
//! - **Call Hierarchy**: Navigate fixture dependency graphs
//!
//! ## Architecture
//!
//! The crate is organized into two main modules:
//!
//! - [`fixtures`]: Core fixture analysis engine with [`FixtureDatabase`] as the central data structure
//! - [`config`]: Configuration file support for `pyproject.toml` settings
//!
//! ## Usage
//!
//! The primary entry point is [`FixtureDatabase`], which provides methods for:
//!
//! - Scanning workspaces for fixture definitions
//! - Analyzing Python files for fixtures and their usages
//! - Resolving fixture definitions based on pytest's priority rules
//! - Providing completion context for fixture suggestions
//!
//! ```no_run
//! use pytest_language_server::FixtureDatabase;
//! use std::path::Path;
//!
//! let db = FixtureDatabase::new();
//! db.scan_workspace(Path::new("./tests"));
//!
//! // Find a fixture definition
//! if let Some(def) = db.find_fixture_definition(Path::new("test_file.py"), 10, 15) {
//!     println!("Found fixture: {} at line {}", def.name, def.line);
//! }
//! ```
//!
//! ## Fixture Resolution
//!
//! The LSP correctly implements pytest's fixture priority/shadowing rules:
//!
//! 1. **Same file**: Fixtures defined in the same file have highest priority
//! 2. **Closest conftest.py**: Walk up directory tree looking for conftest.py
//! 3. **Third-party**: Fixtures from site-packages (50+ plugins supported)

pub mod config;
pub mod fixtures;
pub mod providers;

pub use config::Config;
pub use fixtures::{
    CompletionContext, FixtureCycle, FixtureDatabase, FixtureDefinition, FixtureScope,
    FixtureUsage, ParamInsertionInfo, ScopeMismatch, TypeImportSpec, UndeclaredFixture,
};

// Expose decorators module for testing
#[cfg(test)]
pub use fixtures::decorators;

// Expose Backend for integration testing
pub use providers::Backend;
