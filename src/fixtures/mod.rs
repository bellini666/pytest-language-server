//! Fixture database and analysis module.
//!
//! This module provides the core functionality for managing pytest fixtures:
//! - Scanning workspaces for fixture definitions
//! - Analyzing Python files for fixtures and their usages
//! - Resolving fixture definitions based on pytest's priority rules
//! - Providing completion context for fixture suggestions

mod analyzer;
pub(crate) mod cli;
mod resolver;
mod scanner;
pub mod types;

#[allow(unused_imports)] // ParamInsertionInfo re-exported for public API via lib.rs
pub use types::{
    CompletionContext, FixtureDefinition, FixtureUsage, ParamInsertionInfo, UndeclaredFixture,
};

use dashmap::DashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

/// The central database for fixture definitions and usages.
///
/// Uses `DashMap` for lock-free concurrent access during workspace scanning.
#[derive(Debug)]
pub struct FixtureDatabase {
    /// Map from fixture name to all its definitions (can be in multiple conftest.py files).
    pub definitions: Arc<DashMap<String, Vec<FixtureDefinition>>>,
    /// Map from file path to fixtures used in that file.
    pub usages: Arc<DashMap<PathBuf, Vec<FixtureUsage>>>,
    /// Cache of file contents for analyzed files (uses Arc for efficient sharing).
    pub file_cache: Arc<DashMap<PathBuf, Arc<String>>>,
    /// Map from file path to undeclared fixtures used in function bodies.
    pub undeclared_fixtures: Arc<DashMap<PathBuf, Vec<UndeclaredFixture>>>,
    /// Map from file path to imported names in that file.
    pub imports: Arc<DashMap<PathBuf, HashSet<String>>>,
    /// Cache of canonical paths to avoid repeated filesystem calls.
    pub canonical_path_cache: Arc<DashMap<PathBuf, PathBuf>>,
}

impl Default for FixtureDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl FixtureDatabase {
    /// Create a new empty fixture database.
    pub fn new() -> Self {
        Self {
            definitions: Arc::new(DashMap::new()),
            usages: Arc::new(DashMap::new()),
            file_cache: Arc::new(DashMap::new()),
            undeclared_fixtures: Arc::new(DashMap::new()),
            imports: Arc::new(DashMap::new()),
            canonical_path_cache: Arc::new(DashMap::new()),
        }
    }

    /// Get canonical path with caching to avoid repeated filesystem calls.
    /// Falls back to original path if canonicalization fails.
    pub(crate) fn get_canonical_path(&self, path: PathBuf) -> PathBuf {
        // Check cache first
        if let Some(cached) = self.canonical_path_cache.get(&path) {
            return cached.value().clone();
        }

        // Attempt canonicalization
        let canonical = path.canonicalize().unwrap_or_else(|_| {
            debug!("Could not canonicalize path {:?}, using as-is", path);
            path.clone()
        });

        // Store in cache for future lookups
        self.canonical_path_cache.insert(path, canonical.clone());
        canonical
    }

    /// Get file content from cache or read from filesystem.
    /// Returns None if file cannot be read.
    pub(crate) fn get_file_content(&self, file_path: &Path) -> Option<Arc<String>> {
        if let Some(cached) = self.file_cache.get(file_path) {
            Some(Arc::clone(cached.value()))
        } else {
            std::fs::read_to_string(file_path).ok().map(Arc::new)
        }
    }
}
