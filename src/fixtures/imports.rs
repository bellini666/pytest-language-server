//! Fixture import resolution.
//!
//! This module handles tracking and resolving fixtures that are imported
//! into conftest.py or test files via `from X import *` or explicit imports.
//!
//! When a conftest.py has `from .pytest_fixtures import *`, all fixtures
//! defined in that module become available as if they were defined in the
//! conftest.py itself.

use super::types::TypeImportSpec;
use super::FixtureDatabase;
use once_cell::sync::Lazy;
use rustpython_parser::ast::{Expr, Stmt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tracing::{debug, info, warn};

/// Runtime stdlib module names populated from the venv's Python binary via
/// `sys.stdlib_module_names` (Python ≥ 3.10).  When set, this takes
/// precedence over the static [`STDLIB_MODULES`] fallback list in
/// [`is_stdlib_module`].
///
/// Set at most once per process lifetime by [`try_init_stdlib_from_python`].
static RUNTIME_STDLIB_MODULES: OnceLock<HashSet<String>> = OnceLock::new();

/// Built-in fallback list of standard library module names for O(1) lookup.
///
/// Used when [`RUNTIME_STDLIB_MODULES`] has not been populated (no venv
/// found, Python < 3.10, or the Python binary could not be executed).
/// Intentionally conservative — it is better to misclassify an unknown
/// third-party module as stdlib (and skip inserting a redundant import)
/// than to misclassify a stdlib module as third-party.
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

    /// Extract module paths from `pytest_plugins` variable assignments.
    ///
    /// Handles both regular and annotated assignments:
    /// - `pytest_plugins = "module"` (single string)
    /// - `pytest_plugins = ["module_a", "module_b"]` (list)
    /// - `pytest_plugins = ("module_a", "module_b")` (tuple)
    /// - `pytest_plugins: list[str] = ["module_a"]` (annotated)
    ///
    /// If multiple assignments exist, only the last one is used (matching pytest semantics).
    pub(crate) fn extract_pytest_plugins(&self, stmts: &[Stmt]) -> Vec<String> {
        let mut modules = Vec::new();

        for stmt in stmts {
            let value = match stmt {
                Stmt::Assign(assign) => {
                    let is_pytest_plugins = assign.targets.iter().any(|target| {
                        matches!(target, Expr::Name(name) if name.id.as_str() == "pytest_plugins")
                    });
                    if !is_pytest_plugins {
                        continue;
                    }
                    assign.value.as_ref()
                }
                Stmt::AnnAssign(ann_assign) => {
                    let is_pytest_plugins = matches!(
                        ann_assign.target.as_ref(),
                        Expr::Name(name) if name.id.as_str() == "pytest_plugins"
                    );
                    if !is_pytest_plugins {
                        continue;
                    }
                    match ann_assign.value.as_ref() {
                        Some(v) => v.as_ref(),
                        None => continue,
                    }
                }
                _ => continue,
            };

            // Last assignment wins: clear previous values
            modules.clear();

            match value {
                Expr::Constant(c) => {
                    if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                        modules.push(s.to_string());
                    }
                }
                Expr::List(list) => {
                    for elt in &list.elts {
                        if let Expr::Constant(c) = elt {
                            if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                                modules.push(s.to_string());
                            }
                        }
                    }
                }
                Expr::Tuple(tuple) => {
                    for elt in &tuple.elts {
                        if let Expr::Constant(c) = elt {
                            if let rustpython_parser::ast::Constant::Str(s) = &c.value {
                                modules.push(s.to_string());
                            }
                        }
                    }
                }
                _ => {
                    debug!("Ignoring dynamic pytest_plugins value (not a string/list/tuple)");
                }
            }
        }

        modules
    }

    /// Check if a module is a standard library module that can't contain fixtures.
    /// Uses a static HashSet for O(1) lookup instead of linear array search.
    fn is_standard_library_module(&self, module: &str) -> bool {
        is_stdlib_module(module)
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

    /// Resolve an absolute import by searching up the directory tree,
    /// then falling back to site-packages paths for venv plugin modules.
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

        // Fallback: search in site-packages paths (for venv plugin pytest_plugins)
        for sp in self.site_packages_paths.lock().unwrap().iter() {
            if let Some(path) = self.find_module_file(module_path, sp) {
                return Some(path);
            }
        }

        // Fallback: search in editable install source roots
        for install in self.editable_install_roots.lock().unwrap().iter() {
            if let Some(path) = self.find_module_file(module_path, &install.source_root) {
                return Some(path);
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

            // Process pytest_plugins variable (treated like star imports)
            let plugin_modules = self.extract_pytest_plugins(&module.body);
            for module_path in plugin_modules {
                let Some(resolved_path) = self.resolve_module_to_file(&module_path, canonical_path)
                else {
                    debug!(
                        "Could not resolve pytest_plugins module '{}' from {:?}",
                        module_path, canonical_path
                    );
                    continue;
                };

                let resolved_canonical = self.get_canonical_path(resolved_path);

                debug!(
                    "Resolved pytest_plugins '{}' to {:?}",
                    module_path, resolved_canonical
                );

                if let Some(file_fixtures) = self.file_definitions.get(&resolved_canonical) {
                    for fixture_name in file_fixtures.iter() {
                        imported_fixtures.insert(fixture_name.clone());
                    }
                }

                let transitive = self.get_imported_fixtures(&resolved_canonical, visited);
                imported_fixtures.extend(transitive);
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

/// Check whether `module` (possibly dotted, e.g. `"collections.abc"`) belongs
/// to the Python standard library.  Only the top-level package name is tested.
///
/// Checks [`RUNTIME_STDLIB_MODULES`] first (populated by
/// [`try_init_stdlib_from_python`] when a venv with Python ≥ 3.10 is found),
/// then falls back to the built-in [`STDLIB_MODULES`] list.
///
/// Exposed as a free function so that the code-action provider can classify
/// import statements without access to a `FixtureDatabase` instance.
pub(crate) fn is_stdlib_module(module: &str) -> bool {
    let first_part = module.split('.').next().unwrap_or(module);
    if let Some(runtime) = RUNTIME_STDLIB_MODULES.get() {
        runtime.contains(first_part)
    } else {
        STDLIB_MODULES.contains(first_part)
    }
}

/// Try to locate the Python interpreter inside a virtual environment.
///
/// Checks the standard Unix (`bin/python3`, `bin/python`) and Windows
/// (`Scripts/python3.exe`, `Scripts/python.exe`) layouts in that order.
/// Returns the first path that resolves to an existing regular file (or
/// symlink to one).
fn find_venv_python(venv_path: &Path) -> Option<PathBuf> {
    // Unix / macOS layout
    for name in &["python3", "python"] {
        let candidate = venv_path.join("bin").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // Windows layout
    for name in &["python3.exe", "python.exe"] {
        let candidate = venv_path.join("Scripts").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Attempt to populate [`RUNTIME_STDLIB_MODULES`] by querying the Python
/// interpreter found inside `venv_path`.
///
/// Runs:
/// ```text
/// python -I -c "import sys; print('\n'.join(sorted(sys.stdlib_module_names)))"
/// ```
///
/// `sys.stdlib_module_names` was added in Python 3.10.  For older interpreters
/// the command exits with a non-zero status and this function returns `false`,
/// leaving [`is_stdlib_module`] to use the static fallback list.
///
/// The `OnceLock` guarantees that the runtime list is set at most once per
/// process lifetime.  Subsequent calls return `true` immediately when the
/// lock is already populated.
///
/// Returns `true` if the runtime list is now available (either just populated
/// or already set by a previous call), `false` otherwise.
pub(crate) fn try_init_stdlib_from_python(venv_path: &Path) -> bool {
    // Already initialised — nothing to do.
    if RUNTIME_STDLIB_MODULES.get().is_some() {
        return true;
    }

    let Some(python) = find_venv_python(venv_path) else {
        debug!(
            "try_init_stdlib_from_python: no Python binary found in {:?}",
            venv_path
        );
        return false;
    };

    debug!(
        "try_init_stdlib_from_python: querying stdlib module names via {:?}",
        python
    );

    // -I (isolated): ignore PYTHONPATH, user site, PYTHONSTARTUP — we only
    // need a pristine `sys` module, nothing else.
    let output = match std::process::Command::new(&python)
        .args([
            "-I",
            "-c",
            "import sys; print('\\n'.join(sorted(sys.stdlib_module_names)))",
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            warn!(
                "try_init_stdlib_from_python: failed to run {:?}: {}",
                python, e
            );
            return false;
        }
    };

    if !output.status.success() {
        // Most likely Python < 3.10 — AttributeError on sys.stdlib_module_names.
        debug!(
            "try_init_stdlib_from_python: Python exited with {:?} \
             (Python < 3.10 or other error) — using built-in stdlib list",
            output.status.code()
        );
        return false;
    }

    let stdout = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(e) => {
            warn!(
                "try_init_stdlib_from_python: Python output is not valid UTF-8: {}",
                e
            );
            return false;
        }
    };

    let modules: HashSet<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();

    if modules.is_empty() {
        warn!("try_init_stdlib_from_python: Python returned an empty module list");
        return false;
    }

    info!(
        "try_init_stdlib_from_python: loaded {} stdlib module names from {:?}",
        modules.len(),
        python
    );

    // Ignore the error — another thread may have raced us; either way the
    // OnceLock now contains a valid set.
    let _ = RUNTIME_STDLIB_MODULES.set(modules);
    true
}

impl FixtureDatabase {
    /// Convert a file path to a dotted Python module path string.
    ///
    /// Walks upward from the file's parent directory, accumulating package
    /// components as long as each directory contains an `__init__.py` file.
    /// Stops at the first directory that is not a package.
    ///
    /// **Note:** This function checks the filesystem (`__init__.py` existence)
    /// at call time.  Results are captured in `FixtureDefinition::return_type_imports`
    /// during analysis — if `__init__.py` files are added or removed after
    /// analysis, re-analysis of the fixture file is required for the module
    /// path to update.
    ///
    /// Examples (assuming `tests/` has `__init__.py` but `project/` does not):
    /// - `/project/tests/conftest.py`      →  `"tests.conftest"`
    /// - `/project/tests/__init__.py`      →  `"tests"`   (package root, stem dropped)
    /// - `/tmp/conftest.py`                →  `"conftest"`   (no __init__.py found)
    /// - `/project/tests/helpers/utils.py` →  `"tests.helpers.utils"` (nested package)
    pub(crate) fn file_path_to_module_path(file_path: &Path) -> Option<String> {
        let stem = file_path.file_stem()?.to_str()?;
        // `__init__.py` *is* the package — its stem must not be added as a
        // component.  The parent-directory traversal loop below will push the
        // directory name (e.g. `pkg/sub/__init__.py` → `"pkg.sub"`).
        // Any other file gets its stem as the first component
        // (e.g. `pkg/sub/module.py` → `"pkg.sub.module"`).
        let mut components = if stem == "__init__" {
            vec![]
        } else {
            vec![stem.to_string()]
        };
        let mut current = file_path.parent()?;

        loop {
            if current.join("__init__.py").exists() {
                let name = current.file_name().and_then(|n| n.to_str())?;
                components.push(name.to_string());
                match current.parent() {
                    Some(parent) => current = parent,
                    None => break,
                }
            } else {
                break;
            }
        }

        if components.is_empty() {
            return None;
        }

        components.reverse();
        Some(components.join("."))
    }

    /// Resolve a relative import (e.g. `from .models import X` where level=1,
    /// module="models") to an absolute dotted module path string suitable for
    /// use in any file (not just the fixture's package).
    ///
    /// Returns `None` when the path cannot be resolved (e.g. level goes above
    /// the filesystem root).
    fn resolve_relative_module_to_string(
        &self,
        module: &str,
        level: usize,
        fixture_file: &Path,
    ) -> Option<String> {
        // Navigate up `level` directories from the fixture file's own directory.
        // level=1 means "current package" (.models), level=2 means "parent" (..models).
        let mut base = fixture_file.parent()?;
        for _ in 1..level {
            base = base.parent()?;
        }

        // Build the theoretical target file path (may or may not exist on disk).
        let target = if module.is_empty() {
            // `from . import X` — target is the package __init__.py itself.
            base.join("__init__.py")
        } else {
            // Replace dots in sub-module path with path separators.
            let rel_path = module.replace('.', "/");
            base.join(format!("{}.py", rel_path))
        };

        // Convert that file path to a dotted module path string.
        Self::file_path_to_module_path(&target)
    }

    /// Build a map from imported name → `TypeImportSpec` for all import
    /// statements in `stmts`.
    ///
    /// Unlike `extract_fixture_imports`, this function processes **all** imports
    /// (including stdlib such as `pathlib` and `typing`) because type annotations
    /// may reference any imported name.  Relative imports are resolved to their
    /// absolute form so the resulting `import_statement` strings are valid in any
    /// file, not just in the fixture's own package.
    ///
    /// Covers all four Python import styles:
    ///
    /// | Source statement                    | check_name  | import_statement               |
    /// |-------------------------------------|-------------|-------------------------------|
    /// | `import pathlib`                    | `"pathlib"` | `"import pathlib"`             |
    /// | `import pathlib as pl`              | `"pl"`      | `"import pathlib as pl"`       |
    /// | `from pathlib import Path`          | `"Path"`    | `"from pathlib import Path"`   |
    /// | `from pathlib import Path as P`     | `"P"`       | `"from pathlib import Path as P"` |
    pub(crate) fn build_name_to_import_map(
        &self,
        stmts: &[Stmt],
        fixture_file: &Path,
    ) -> HashMap<String, TypeImportSpec> {
        let mut map = HashMap::new();

        for stmt in stmts {
            match stmt {
                Stmt::Import(import_stmt) => {
                    for alias in &import_stmt.names {
                        let module = alias.name.to_string();
                        let (check_name, import_statement) = if let Some(ref asname) = alias.asname
                        {
                            let asname_str = asname.to_string();
                            (
                                asname_str.clone(),
                                format!("import {} as {}", module, asname_str),
                            )
                        } else {
                            (module.clone(), format!("import {}", module))
                        };
                        map.insert(
                            check_name.clone(),
                            TypeImportSpec {
                                check_name,
                                import_statement,
                            },
                        );
                    }
                }

                Stmt::ImportFrom(import_from) => {
                    let level = import_from
                        .level
                        .as_ref()
                        .map(|l| l.to_usize())
                        .unwrap_or(0);
                    let raw_module = import_from
                        .module
                        .as_ref()
                        .map(|m| m.to_string())
                        .unwrap_or_default();

                    // Resolve relative imports to absolute module paths.
                    let abs_module = if level > 0 {
                        match self.resolve_relative_module_to_string(
                            &raw_module,
                            level,
                            fixture_file,
                        ) {
                            Some(m) => m,
                            None => {
                                debug!(
                                    "Could not resolve relative import '.{}' from {:?}, skipping",
                                    raw_module, fixture_file
                                );
                                continue;
                            }
                        }
                    } else {
                        raw_module
                    };

                    for alias in &import_from.names {
                        if alias.name.as_str() == "*" {
                            continue; // Star imports don't bind individual names here.
                        }
                        let name = alias.name.to_string();
                        let (check_name, import_statement) = if let Some(ref asname) = alias.asname
                        {
                            let asname_str = asname.to_string();
                            (
                                asname_str.clone(),
                                format!("from {} import {} as {}", abs_module, name, asname_str),
                            )
                        } else {
                            (name.clone(), format!("from {} import {}", abs_module, name))
                        };
                        map.insert(
                            check_name.clone(),
                            TypeImportSpec {
                                check_name,
                                import_statement,
                            },
                        );
                    }
                }

                _ => {}
            }
        }

        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temp directory tree and return a guard that deletes it on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(name);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ── find_venv_python ───────────────────────────────────────────────────

    /// Write an empty file at `path`, creating parent directories as needed.
    fn touch(path: &std::path::Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn test_find_venv_python_unix_python3() {
        let dir = TempDir::new("fvp_unix_py3");
        touch(&dir.path().join("bin/python3"));
        let result = find_venv_python(dir.path());
        assert_eq!(result, Some(dir.path().join("bin/python3")));
    }

    #[test]
    fn test_find_venv_python_unix_python_fallback() {
        // Only `python` present (no `python3`).
        let dir = TempDir::new("fvp_unix_py");
        touch(&dir.path().join("bin/python"));
        let result = find_venv_python(dir.path());
        assert_eq!(result, Some(dir.path().join("bin/python")));
    }

    #[test]
    fn test_find_venv_python_unix_prefers_python3_over_python() {
        let dir = TempDir::new("fvp_unix_prefer");
        touch(&dir.path().join("bin/python3"));
        touch(&dir.path().join("bin/python"));
        let result = find_venv_python(dir.path());
        assert_eq!(
            result,
            Some(dir.path().join("bin/python3")),
            "python3 should be preferred over python"
        );
    }

    #[test]
    fn test_find_venv_python_windows_style() {
        let dir = TempDir::new("fvp_win_py");
        touch(&dir.path().join("Scripts/python.exe"));
        let result = find_venv_python(dir.path());
        assert_eq!(result, Some(dir.path().join("Scripts/python.exe")));
    }

    #[test]
    fn test_find_venv_python_windows_prefers_python3_exe() {
        let dir = TempDir::new("fvp_win_prefer");
        touch(&dir.path().join("Scripts/python3.exe"));
        touch(&dir.path().join("Scripts/python.exe"));
        let result = find_venv_python(dir.path());
        assert_eq!(
            result,
            Some(dir.path().join("Scripts/python3.exe")),
            "python3.exe should be preferred over python.exe"
        );
    }

    #[test]
    fn test_find_venv_python_not_found() {
        let dir = TempDir::new("fvp_empty");
        assert_eq!(find_venv_python(dir.path()), None);
    }

    #[test]
    fn test_find_venv_python_wrong_layout() {
        // Python binary at the venv root — not in bin/ or Scripts/.
        let dir = TempDir::new("fvp_wrong_layout");
        touch(&dir.path().join("python3"));
        assert_eq!(find_venv_python(dir.path()), None);
    }

    #[test]
    fn test_try_init_stdlib_no_python_returns_false_or_already_set() {
        // An empty venv directory has no Python binary → should return false
        // without panicking.  If RUNTIME_STDLIB_MODULES was already populated
        // by a prior test (OnceLock is set once per process) the function
        // returns true; either way is_stdlib_module must remain correct.
        let dir = TempDir::new("fvp_no_python");
        let _ = try_init_stdlib_from_python(dir.path());
        assert!(is_stdlib_module("os"), "os must always be stdlib");
        assert!(is_stdlib_module("sys"), "sys must always be stdlib");
        assert!(!is_stdlib_module("pytest"), "pytest is not stdlib");
        assert!(!is_stdlib_module("flask"), "flask is not stdlib");
    }

    // ── file_path_to_module_path ────────────────────────────────────────────

    #[test]
    fn test_module_path_regular_file_no_package() {
        // File in a plain directory (no __init__.py) → just the stem.
        let dir = TempDir::new("fptmp_plain");
        let file = dir.path().join("conftest.py");
        fs::write(&file, "").unwrap();
        // No __init__.py in the directory, so the result is just "conftest".
        assert_eq!(
            FixtureDatabase::file_path_to_module_path(&file),
            Some("conftest".to_string())
        );
    }

    #[test]
    fn test_module_path_regular_file_in_package() {
        // pkg/__init__.py exists → file inside pkg resolves to "pkg.module".
        let dir = TempDir::new("fptmp_pkg");
        let pkg = dir.path().join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("__init__.py"), "").unwrap();
        let file = pkg.join("module.py");
        fs::write(&file, "").unwrap();
        assert_eq!(
            FixtureDatabase::file_path_to_module_path(&file),
            Some("pkg.module".to_string())
        );
    }

    #[test]
    fn test_module_path_init_file_is_package_root() {
        // pkg/__init__.py itself → resolves to "pkg", NOT "pkg.__init__".
        // This is the regression test for the `from . import X` bug fix.
        let dir = TempDir::new("fptmp_init");
        let pkg = dir.path().join("pkg");
        fs::create_dir_all(&pkg).unwrap();
        let init = pkg.join("__init__.py");
        fs::write(&init, "").unwrap();
        assert_eq!(
            FixtureDatabase::file_path_to_module_path(&init),
            Some("pkg".to_string())
        );
    }

    #[test]
    fn test_module_path_nested_init_file() {
        // pkg/sub/__init__.py → resolves to "pkg.sub", NOT "pkg.sub.__init__".
        let dir = TempDir::new("fptmp_nested_init");
        let pkg = dir.path().join("pkg");
        let sub = pkg.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(pkg.join("__init__.py"), "").unwrap();
        let init = sub.join("__init__.py");
        fs::write(&init, "").unwrap();
        assert_eq!(
            FixtureDatabase::file_path_to_module_path(&init),
            Some("pkg.sub".to_string())
        );
    }

    #[test]
    fn test_module_path_nested_package() {
        // pkg/sub/module.py with both __init__.py files → "pkg.sub.module".
        let dir = TempDir::new("fptmp_nested");
        let pkg = dir.path().join("pkg");
        let sub = pkg.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(pkg.join("__init__.py"), "").unwrap();
        fs::write(sub.join("__init__.py"), "").unwrap();
        let file = sub.join("module.py");
        fs::write(&file, "").unwrap();
        assert_eq!(
            FixtureDatabase::file_path_to_module_path(&file),
            Some("pkg.sub.module".to_string())
        );
    }

    #[test]
    fn test_module_path_conftest_in_package() {
        // pkg/conftest.py → "pkg.conftest".
        let dir = TempDir::new("fptmp_conftest_pkg");
        let pkg = dir.path().join("mypkg");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("__init__.py"), "").unwrap();
        let file = pkg.join("conftest.py");
        fs::write(&file, "").unwrap();
        assert_eq!(
            FixtureDatabase::file_path_to_module_path(&file),
            Some("mypkg.conftest".to_string())
        );
    }
}
