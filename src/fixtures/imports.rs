//! Fixture import resolution.
//!
//! This module handles tracking and resolving fixtures that are imported
//! into conftest.py or test files via `from X import *` or explicit imports.
//!
//! When a conftest.py has `from .pytest_fixtures import *`, all fixtures
//! defined in that module become available as if they were defined in the
//! conftest.py itself.

use super::FixtureDatabase;
use once_cell::sync::Lazy;
use rustpython_parser::ast::Stmt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info};

/// Static HashSet of standard library module names for O(1) lookup.
static STDLIB_MODULES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "os",
        "sys",
        "re",
        "json",
        "typing",
        "collections",
        "functools",
        "itertools",
        "pathlib",
        "datetime",
        "time",
        "math",
        "random",
        "copy",
        "io",
        "abc",
        "contextlib",
        "dataclasses",
        "enum",
        "logging",
        "unittest",
        "asyncio",
        "concurrent",
        "multiprocessing",
        "threading",
        "subprocess",
        "shutil",
        "tempfile",
        "glob",
        "fnmatch",
        "pickle",
        "sqlite3",
        "urllib",
        "http",
        "email",
        "html",
        "xml",
        "socket",
        "ssl",
        "select",
        "signal",
        "struct",
        "codecs",
        "textwrap",
        "string",
        "difflib",
        "inspect",
        "dis",
        "traceback",
        "warnings",
        "weakref",
        "types",
        "importlib",
        "pkgutil",
        "pprint",
        "reprlib",
        "numbers",
        "decimal",
        "fractions",
        "statistics",
        "hashlib",
        "hmac",
        "secrets",
        "base64",
        "binascii",
        "zlib",
        "gzip",
        "bz2",
        "lzma",
        "zipfile",
        "tarfile",
        "csv",
        "configparser",
        "argparse",
        "getopt",
        "getpass",
        "platform",
        "errno",
        "ctypes",
        "__future__",
    ]
    .into_iter()
    .collect()
});

/// Represents a fixture import in a Python file.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for debugging and potential future features
pub struct FixtureImport {
    /// The module path being imported from (e.g., ".pytest_fixtures" or "pytest_fixtures")
    pub module_path: String,
    /// Whether this is a star import (`from X import *`)
    pub is_star_import: bool,
    /// Specific names imported (empty for star imports)
    pub imported_names: Vec<String>,
    /// The file that contains this import
    pub importing_file: PathBuf,
    /// Line number of the import statement
    pub line: usize,
}

impl FixtureDatabase {
    /// Extract fixture imports from a module's statements.
    /// Returns a list of imports that could potentially bring in fixtures.
    pub(crate) fn extract_fixture_imports(
        &self,
        stmts: &[Stmt],
        file_path: &Path,
        line_index: &[usize],
    ) -> Vec<FixtureImport> {
        let mut imports = Vec::new();

        for stmt in stmts {
            if let Stmt::ImportFrom(import_from) = stmt {
                // Skip imports from standard library or well-known non-fixture modules
                let mut module = import_from
                    .module
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_default();

                // Add leading dots for relative imports
                // level indicates how many parent directories to go up:
                // level=1 means "from . import" (current package)
                // level=2 means "from .. import" (parent package)
                if let Some(ref level) = import_from.level {
                    let dots = ".".repeat(level.to_usize());
                    module = dots + &module;
                }

                // Skip obvious non-fixture imports
                if self.is_standard_library_module(&module) {
                    continue;
                }

                let line =
                    self.get_line_from_offset(import_from.range.start().to_usize(), line_index);

                // Check if this is a star import
                let is_star = import_from
                    .names
                    .iter()
                    .any(|alias| alias.name.as_str() == "*");

                if is_star {
                    imports.push(FixtureImport {
                        module_path: module,
                        is_star_import: true,
                        imported_names: Vec::new(),
                        importing_file: file_path.to_path_buf(),
                        line,
                    });
                } else {
                    // Collect specific imported names
                    let names: Vec<String> = import_from
                        .names
                        .iter()
                        .map(|alias| alias.asname.as_ref().unwrap_or(&alias.name).to_string())
                        .collect();

                    if !names.is_empty() {
                        imports.push(FixtureImport {
                            module_path: module,
                            is_star_import: false,
                            imported_names: names,
                            importing_file: file_path.to_path_buf(),
                            line,
                        });
                    }
                }
            }
        }

        imports
    }

    /// Check if a module is a standard library module that can't contain fixtures.
    /// Uses a static HashSet for O(1) lookup instead of linear array search.
    fn is_standard_library_module(&self, module: &str) -> bool {
        let first_part = module.split('.').next().unwrap_or(module);
        STDLIB_MODULES.contains(first_part)
    }

    /// Resolve a module path to a file path.
    /// Handles both relative imports (starting with .) and absolute imports.
    pub(crate) fn resolve_module_to_file(
        &self,
        module_path: &str,
        importing_file: &Path,
    ) -> Option<PathBuf> {
        debug!(
            "Resolving module '{}' from file {:?}",
            module_path, importing_file
        );

        let parent_dir = importing_file.parent()?;

        if module_path.starts_with('.') {
            // Relative import
            self.resolve_relative_import(module_path, parent_dir)
        } else {
            // Absolute import - search in the same directory tree
            self.resolve_absolute_import(module_path, parent_dir)
        }
    }

    /// Resolve a relative import like `.pytest_fixtures` or `..utils`.
    fn resolve_relative_import(&self, module_path: &str, base_dir: &Path) -> Option<PathBuf> {
        let mut current_dir = base_dir.to_path_buf();
        let mut chars = module_path.chars().peekable();

        // Count leading dots to determine how many directories to go up
        while chars.peek() == Some(&'.') {
            chars.next();
            if chars.peek() != Some(&'.') {
                // Single dot - stay in current directory
                break;
            }
            // Additional dots - go up one directory
            current_dir = current_dir.parent()?.to_path_buf();
        }

        let remaining: String = chars.collect();
        if remaining.is_empty() {
            // Import from __init__.py of current/parent package
            let init_path = current_dir.join("__init__.py");
            if init_path.exists() {
                return Some(init_path);
            }
            return None;
        }

        self.find_module_file(&remaining, &current_dir)
    }

    /// Resolve an absolute import by searching up the directory tree.
    fn resolve_absolute_import(&self, module_path: &str, start_dir: &Path) -> Option<PathBuf> {
        let mut current_dir = start_dir.to_path_buf();

        loop {
            if let Some(path) = self.find_module_file(module_path, &current_dir) {
                return Some(path);
            }

            // Go up one directory
            match current_dir.parent() {
                Some(parent) => current_dir = parent.to_path_buf(),
                None => break,
            }
        }

        None
    }

    /// Find a module file given a dotted path and base directory.
    fn find_module_file(&self, module_path: &str, base_dir: &Path) -> Option<PathBuf> {
        let parts: Vec<&str> = module_path.split('.').collect();
        let mut current_path = base_dir.to_path_buf();

        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;

            if is_last {
                // Last part - could be a module file or a package
                let py_file = current_path.join(format!("{}.py", part));
                if py_file.exists() {
                    return Some(py_file);
                }

                // Also check if the file is in the cache (for test files that don't exist on disk)
                let canonical_py_file = self.get_canonical_path(py_file.clone());
                if self.file_cache.contains_key(&canonical_py_file) {
                    return Some(py_file);
                }

                // Check if it's a package with __init__.py
                let package_init = current_path.join(part).join("__init__.py");
                if package_init.exists() {
                    return Some(package_init);
                }

                // Also check if the package __init__.py is in the cache
                let canonical_package_init = self.get_canonical_path(package_init.clone());
                if self.file_cache.contains_key(&canonical_package_init) {
                    return Some(package_init);
                }
            } else {
                // Not the last part - must be a directory
                current_path = current_path.join(part);
                if !current_path.is_dir() {
                    return None;
                }
            }
        }

        None
    }

    /// Get fixtures that are re-exported from a file via imports.
    /// This handles `from .module import *` patterns that bring fixtures into scope.
    ///
    /// Results are cached with content-hash and definitions-version based invalidation.
    /// Returns fixture names that are available in `file_path` via imports.
    pub fn get_imported_fixtures(
        &self,
        file_path: &Path,
        visited: &mut HashSet<PathBuf>,
    ) -> HashSet<String> {
        let canonical_path = self.get_canonical_path(file_path.to_path_buf());

        // Prevent circular imports
        if visited.contains(&canonical_path) {
            debug!("Circular import detected for {:?}, skipping", file_path);
            return HashSet::new();
        }
        visited.insert(canonical_path.clone());

        // Get the file content first (needed for cache validation)
        let Some(content) = self.get_file_content(&canonical_path) else {
            return HashSet::new();
        };

        let content_hash = Self::hash_content(&content);
        let current_version = self
            .definitions_version
            .load(std::sync::atomic::Ordering::SeqCst);

        // Check cache - valid if both content hash and definitions version match
        if let Some(cached) = self.imported_fixtures_cache.get(&canonical_path) {
            let (cached_content_hash, cached_version, cached_fixtures) = cached.value();
            if *cached_content_hash == content_hash && *cached_version == current_version {
                debug!("Cache hit for imported fixtures in {:?}", canonical_path);
                return cached_fixtures.as_ref().clone();
            }
        }

        // Compute imported fixtures
        let imported_fixtures = self.compute_imported_fixtures(&canonical_path, &content, visited);

        // Store in cache
        self.imported_fixtures_cache.insert(
            canonical_path.clone(),
            (
                content_hash,
                current_version,
                Arc::new(imported_fixtures.clone()),
            ),
        );

        info!(
            "Found {} imported fixtures for {:?}: {:?}",
            imported_fixtures.len(),
            file_path,
            imported_fixtures
        );

        imported_fixtures
    }

    /// Internal method to compute imported fixtures without caching.
    fn compute_imported_fixtures(
        &self,
        canonical_path: &Path,
        content: &str,
        visited: &mut HashSet<PathBuf>,
    ) -> HashSet<String> {
        let mut imported_fixtures = HashSet::new();

        let Some(parsed) = self.get_parsed_ast(canonical_path, content) else {
            return imported_fixtures;
        };

        let line_index = self.get_line_index(canonical_path, content);

        if let rustpython_parser::ast::Mod::Module(module) = parsed.as_ref() {
            let imports = self.extract_fixture_imports(&module.body, canonical_path, &line_index);

            for import in imports {
                // Resolve the import to a file path
                let Some(resolved_path) =
                    self.resolve_module_to_file(&import.module_path, canonical_path)
                else {
                    debug!(
                        "Could not resolve module '{}' from {:?}",
                        import.module_path, canonical_path
                    );
                    continue;
                };

                let resolved_canonical = self.get_canonical_path(resolved_path);

                debug!(
                    "Resolved import '{}' to {:?}",
                    import.module_path, resolved_canonical
                );

                if import.is_star_import {
                    // Star import: get all fixtures from the resolved file
                    // First, get fixtures defined directly in that file
                    if let Some(file_fixtures) = self.file_definitions.get(&resolved_canonical) {
                        for fixture_name in file_fixtures.iter() {
                            imported_fixtures.insert(fixture_name.clone());
                        }
                    }

                    // Also recursively get fixtures imported into that file
                    let transitive = self.get_imported_fixtures(&resolved_canonical, visited);
                    imported_fixtures.extend(transitive);
                } else {
                    // Explicit import: only include the specified names if they are fixtures
                    for name in &import.imported_names {
                        if self.definitions.contains_key(name) {
                            imported_fixtures.insert(name.clone());
                        }
                    }
                }
            }
        }

        imported_fixtures
    }

    /// Check if a fixture is available in a file via imports.
    /// This is used in resolution to check conftest.py files that import fixtures.
    pub fn is_fixture_imported_in_file(&self, fixture_name: &str, file_path: &Path) -> bool {
        let mut visited = HashSet::new();
        let imported = self.get_imported_fixtures(file_path, &mut visited);
        imported.contains(fixture_name)
    }
}
