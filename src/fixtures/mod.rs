//! Fixture database and analysis module.
//!
//! This module provides the core functionality for managing pytest fixtures:
//! - Scanning workspaces for fixture definitions
//! - Analyzing Python files for fixtures and their usages
//! - Resolving fixture definitions based on pytest's priority rules
//! - Providing completion context for fixture suggestions

mod analyzer;
pub(crate) mod cli;
pub mod decorators; // Public for testing
mod resolver;
mod scanner;
mod string_utils;
pub mod types;

#[allow(unused_imports)] // ParamInsertionInfo re-exported for public API via lib.rs
pub use types::{
    CompletionContext, FixtureDefinition, FixtureUsage, ParamInsertionInfo, UndeclaredFixture,
};

use dashmap::DashMap;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

/// Cache entry for line indices: (content_hash, line_index).
/// The content hash is used to invalidate the cache when file content changes.
type LineIndexCacheEntry = (u64, Arc<Vec<usize>>);

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
    /// Cache of line indices (byte offsets) for files to avoid recomputation.
    /// Stores (content_hash, line_index) to invalidate when content changes.
    pub line_index_cache: Arc<DashMap<PathBuf, LineIndexCacheEntry>>,
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
            line_index_cache: Arc::new(DashMap::new()),
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

    /// Get or compute line index for a file, with content-hash-based caching.
    /// Returns Arc to avoid cloning the potentially large Vec.
    /// The cache is invalidated when the content hash changes.
    pub(crate) fn get_line_index(&self, file_path: &Path, content: &str) -> Arc<Vec<usize>> {
        let content_hash = Self::hash_content(content);

        // Check cache first - only use if content hash matches
        if let Some(cached) = self.line_index_cache.get(file_path) {
            let (cached_hash, cached_index) = cached.value();
            if *cached_hash == content_hash {
                return Arc::clone(cached_index);
            }
        }

        // Build line index
        let line_index = Self::build_line_index(content);
        let arc_index = Arc::new(line_index);

        // Store in cache with content hash
        self.line_index_cache.insert(
            file_path.to_path_buf(),
            (content_hash, Arc::clone(&arc_index)),
        );

        arc_index
    }

    /// Compute a hash of the content for cache invalidation.
    fn hash_content(content: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }
}
