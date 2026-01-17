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
mod docstring;
mod imports;
mod resolver;
mod scanner;
pub(crate) mod string_utils; // pub(crate) for inlay_hint provider access
pub mod types;
mod undeclared;

#[allow(unused_imports)] // ParamInsertionInfo re-exported for public API via lib.rs
pub use types::{
    CompletionContext, FixtureCycle, FixtureDefinition, FixtureScope, FixtureUsage,
    ParamInsertionInfo, ScopeMismatch, UndeclaredFixture,
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

/// Cache entry for parsed AST: (content_hash, ast).
/// The content hash is used to invalidate the cache when file content changes.
type AstCacheEntry = (u64, Arc<rustpython_parser::ast::Mod>);

/// Cache entry for fixture cycles: (definitions_version, cycles).
/// The version is incremented when definitions change to invalidate the cache.
type CycleCacheEntry = (u64, Arc<Vec<types::FixtureCycle>>);

/// Cache entry for available fixtures: (definitions_version, fixtures).
/// The version is incremented when definitions change to invalidate the cache.
type AvailableFixturesCacheEntry = (u64, Arc<Vec<FixtureDefinition>>);

/// Cache entry for imported fixtures: (content_hash, definitions_version, imported_fixture_names).
/// Invalidated when either the file content or fixture definitions change.
type ImportedFixturesCacheEntry = (u64, u64, Arc<HashSet<String>>);

/// Maximum number of files to keep in the file content cache.
/// When exceeded, the oldest entries are evicted to prevent unbounded memory growth.
const MAX_FILE_CACHE_SIZE: usize = 2000;

/// The central database for fixture definitions and usages.
///
/// Uses `DashMap` for lock-free concurrent access during workspace scanning.
#[derive(Debug)]
pub struct FixtureDatabase {
    /// Map from fixture name to all its definitions (can be in multiple conftest.py files).
    pub definitions: Arc<DashMap<String, Vec<FixtureDefinition>>>,
    /// Reverse index: file path -> fixture names defined in that file.
    /// Used for efficient cleanup when a file is re-analyzed.
    pub file_definitions: Arc<DashMap<PathBuf, HashSet<String>>>,
    /// Map from file path to fixtures used in that file.
    pub usages: Arc<DashMap<PathBuf, Vec<FixtureUsage>>>,
    /// Reverse index: fixture name -> (file_path, usage) pairs.
    /// Used for efficient O(1) lookup in find_references_for_definition.
    pub usage_by_fixture: Arc<DashMap<String, Vec<(PathBuf, FixtureUsage)>>>,
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
    /// Cache of parsed AST for files to avoid re-parsing.
    /// Stores (content_hash, ast) to invalidate when content changes.
    pub ast_cache: Arc<DashMap<PathBuf, AstCacheEntry>>,
    /// Version counter for definitions, incremented on each change.
    /// Used to invalidate cycle detection cache and available fixtures cache.
    pub definitions_version: Arc<std::sync::atomic::AtomicU64>,
    /// Cache of detected fixture cycles.
    /// Stores (definitions_version, cycles) to invalidate when definitions change.
    pub cycle_cache: Arc<DashMap<(), CycleCacheEntry>>,
    /// Cache of available fixtures per file.
    /// Stores (definitions_version, fixtures) to invalidate when definitions change.
    pub available_fixtures_cache: Arc<DashMap<PathBuf, AvailableFixturesCacheEntry>>,
    /// Cache of imported fixtures per file.
    /// Stores (content_hash, definitions_version, fixture_names) for invalidation.
    pub imported_fixtures_cache: Arc<DashMap<PathBuf, ImportedFixturesCacheEntry>>,
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
            file_definitions: Arc::new(DashMap::new()),
            usages: Arc::new(DashMap::new()),
            usage_by_fixture: Arc::new(DashMap::new()),
            file_cache: Arc::new(DashMap::new()),
            undeclared_fixtures: Arc::new(DashMap::new()),
            imports: Arc::new(DashMap::new()),
            canonical_path_cache: Arc::new(DashMap::new()),
            line_index_cache: Arc::new(DashMap::new()),
            ast_cache: Arc::new(DashMap::new()),
            definitions_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            cycle_cache: Arc::new(DashMap::new()),
            available_fixtures_cache: Arc::new(DashMap::new()),
            imported_fixtures_cache: Arc::new(DashMap::new()),
        }
    }

    /// Increment the definitions version to invalidate cycle cache.
    /// Called whenever fixture definitions are modified.
    pub(crate) fn invalidate_cycle_cache(&self) {
        self.definitions_version
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

    /// Get or parse AST for a file, with content-hash-based caching.
    /// Returns Arc to avoid cloning the potentially large AST.
    /// The cache is invalidated when the content hash changes.
    pub(crate) fn get_parsed_ast(
        &self,
        file_path: &Path,
        content: &str,
    ) -> Option<Arc<rustpython_parser::ast::Mod>> {
        let content_hash = Self::hash_content(content);

        // Check cache first - only use if content hash matches
        if let Some(cached) = self.ast_cache.get(file_path) {
            let (cached_hash, cached_ast) = cached.value();
            if *cached_hash == content_hash {
                return Some(Arc::clone(cached_ast));
            }
        }

        // Parse the content
        let parsed = rustpython_parser::parse(content, rustpython_parser::Mode::Module, "").ok()?;
        let arc_ast = Arc::new(parsed);

        // Store in cache with content hash
        self.ast_cache.insert(
            file_path.to_path_buf(),
            (content_hash, Arc::clone(&arc_ast)),
        );

        Some(arc_ast)
    }

    /// Compute a hash of the content for cache invalidation.
    fn hash_content(content: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// Remove all cached data for a file.
    /// Called when a file is closed or deleted to prevent unbounded memory growth.
    pub fn cleanup_file_cache(&self, file_path: &Path) {
        // Use canonical path for consistent cleanup
        let canonical = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.to_path_buf());

        debug!("Cleaning up cache for file: {:?}", canonical);

        // Remove from line_index_cache
        self.line_index_cache.remove(&canonical);

        // Remove from ast_cache
        self.ast_cache.remove(&canonical);

        // Remove from file_cache
        self.file_cache.remove(&canonical);

        // Remove from available_fixtures_cache (this file's cached available fixtures)
        self.available_fixtures_cache.remove(&canonical);

        // Remove from imported_fixtures_cache
        self.imported_fixtures_cache.remove(&canonical);

        // Note: We don't remove from canonical_path_cache because:
        // 1. It's keyed by original path, not canonical path
        // 2. Path->canonical mappings are stable and small
        // 3. They may be needed again if file is reopened

        // Note: We don't remove definitions/usages here because:
        // 1. They might be needed for cross-file references
        // 2. They're cleaned up on next analyze_file call anyway
    }

    /// Evict entries from caches if they exceed the maximum size.
    /// Called periodically to prevent unbounded memory growth in very large workspaces.
    /// Most LSPs rely on did_close cleanup for open files; this is a safety net for
    /// workspace scan files that accumulate over time.
    pub(crate) fn evict_cache_if_needed(&self) {
        // Only evict if significantly over limit to avoid frequent eviction
        if self.file_cache.len() > MAX_FILE_CACHE_SIZE {
            debug!(
                "File cache size ({}) exceeds limit ({}), evicting entries",
                self.file_cache.len(),
                MAX_FILE_CACHE_SIZE
            );

            // Remove ~25% of entries to avoid frequent re-eviction
            let to_remove_count = self.file_cache.len() / 4;
            let to_remove: Vec<PathBuf> = self
                .file_cache
                .iter()
                .take(to_remove_count)
                .map(|entry| entry.key().clone())
                .collect();

            for path in to_remove {
                self.file_cache.remove(&path);
                // Also clean related caches for consistency
                self.line_index_cache.remove(&path);
                self.ast_cache.remove(&path);
                self.available_fixtures_cache.remove(&path);
                self.imported_fixtures_cache.remove(&path);
            }

            debug!(
                "Cache eviction complete, new size: {}",
                self.file_cache.len()
            );
        }
    }
}
