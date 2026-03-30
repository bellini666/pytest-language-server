//! Import layout analysis for Python files.
//!
//! This module is the single source of truth for import-related operations
//! used across LSP providers (`code_action`, `inlay_hint`, and any future
//! consumers).  It has no dependency on LSP types such as `TextEdit` — those
//! belong in the provider layer.
//!
//! # Design
//!
//! [`parse_import_layout`] is the main entry point.  It tries to produce an
//! [`ImportLayout`] via the `rustpython-parser` AST, which correctly handles
//! multiline parenthesised imports, inline comments, and every other edge case
//! that confounds a naive line-scanner.  If the file has syntax errors (common
//! during active editing) it falls back to a simpler line-scan that mirrors
//! the behaviour of the original string-based `parse_import_groups` function.
//!
//! [`adapt_type_for_consumer`] rewrites a fixture's return-type annotation
//! string to match the consumer file's existing import style (dotted ↔ short).
//! It is used by both `code_action` (which also inserts the necessary import
//! statements) and `inlay_hint` (which only needs the display string).

use crate::fixtures::imports::is_stdlib_module;
use crate::fixtures::string_utils::replace_identifier;
use crate::fixtures::types::TypeImportSpec;
use rustpython_parser::ast::{Mod, Stmt};
use rustpython_parser::Mode;
use std::collections::HashMap;
use tracing::{info, warn};

// ─── Public types ─────────────────────────────────────────────────────────────

/// Which isort / import-sort group an import belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// `from __future__ import …` — must appear before all other imports.
    Future,
    /// Python standard-library import.
    Stdlib,
    /// Third-party or project-local import.
    ThirdParty,
}

/// A contiguous block of module-level import lines (separated from other
/// blocks by blank lines or comment lines), with a classification for the
/// whole group.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportGroup {
    /// 0-based index of the first line in this group.
    pub first_line: usize,
    /// 0-based index of the last line in this group (inclusive).
    pub last_line: usize,
    /// Classification based on the first import in the group.
    pub kind: ImportKind,
}

/// One name (or alias) inside a `from X import …` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedName {
    /// The imported identifier, e.g. `"Path"` or `"*"`.
    pub name: String,
    /// The alias, if present, e.g. `"P"` for `Path as P`.
    pub alias: Option<String>,
}

impl ImportedName {
    /// Format as it appears in source: `"Name"` or `"Name as Alias"`.
    pub fn as_import_str(&self) -> String {
        match &self.alias {
            Some(alias) => format!("{} as {}", self.name, alias),
            None => self.name.clone(),
        }
    }
}

/// A parsed `from X import a, b` (or multiline variant) statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFromImport {
    /// 0-based line of the `from` keyword.
    pub line: usize,
    /// 0-based last line of the statement (equals `line` for single-line imports).
    pub end_line: usize,
    /// Fully-qualified module path, including leading dots for relative imports.
    pub module: String,
    /// Names imported.
    ///
    /// **Note**: for multiline imports parsed via the string fallback (i.e. the
    /// file had syntax errors), this list will be empty because the individual
    /// names cannot be reliably extracted line-by-line.  The AST path always
    /// populates this field correctly.
    pub names: Vec<ImportedName>,
    /// `true` when the import spans multiple lines (parenthesised form).
    pub is_multiline: bool,
}

impl ParsedFromImport {
    /// Return each name formatted as an import string
    /// (`"Name"` or `"Name as Alias"`).
    pub fn name_strings(&self) -> Vec<String> {
        self.names.iter().map(|n| n.as_import_str()).collect()
    }

    /// Whether this is a star-import (`from X import *`).
    pub fn has_star(&self) -> bool {
        self.names.iter().any(|n| n.name == "*")
    }
}

/// A parsed `import X` or `import X as Y` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBareImport {
    /// 0-based line of the `import` keyword.
    pub line: usize,
    /// The fully-qualified module name, e.g. `"pathlib"` or `"os.path"`.
    pub module: String,
    /// The alias, if present (`as Y`).
    pub alias: Option<String>,
}

/// How the [`ImportLayout`] was derived — used for testing and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseSource {
    /// Derived from a successfully parsed AST.
    Ast,
    /// Derived from simple line scanning (file has syntax errors).
    StringFallback,
}

/// Complete import layout for the top-level import section of a Python file.
///
/// Obtained via [`parse_import_layout`].
pub struct ImportLayout {
    /// Classified groups of contiguous import lines.
    pub groups: Vec<ImportGroup>,
    /// All module-level `from X import …` statements, in source order.
    pub from_imports: Vec<ParsedFromImport>,
    /// All module-level `import X` statements, in source order.
    ///
    /// Exposed as part of the public API for future consumers; currently the
    /// providers use the `existing_imports` `HashSet` for deduplication and
    /// only read `from_imports` for merge decisions.
    #[allow(dead_code)]
    pub bare_imports: Vec<ParsedBareImport>,
    /// How this layout was produced — `Ast` or `StringFallback`.
    ///
    /// Read in unit tests to assert which parser path was exercised; not
    /// consumed by production code (the `warn!` in `parse_import_layout`
    /// already records the fallback case in the log).
    #[allow(dead_code)]
    pub source: ParseSource,
    /// Owned copy of each file line for `TextEdit` character-length lookups.
    lines: Vec<String>,
}

impl ImportLayout {
    fn new(
        groups: Vec<ImportGroup>,
        from_imports: Vec<ParsedFromImport>,
        bare_imports: Vec<ParsedBareImport>,
        source: ParseSource,
        content: &str,
    ) -> Self {
        let lines = content.lines().map(|l| l.to_string()).collect();
        Self {
            groups,
            from_imports,
            bare_imports,
            source,
            lines,
        }
    }

    /// Borrow all lines as `&str` slices (for passing to sort/insert helpers).
    pub fn line_strs(&self) -> Vec<&str> {
        self.lines.iter().map(|s| s.as_str()).collect()
    }

    /// Get a single 0-based line as `&str` (empty string if out of bounds).
    pub fn line(&self, idx: usize) -> &str {
        self.lines.get(idx).map(|s| s.as_str()).unwrap_or("")
    }

    /// Find a module-level `from <module> import …` entry that is *not* a
    /// star-import, regardless of whether it is single-line or multiline.
    ///
    /// Returns `None` if no matching entry exists or only star-imports match.
    /// Used by `emit_kind_import_edits` to decide whether to merge a new name
    /// into an existing import line or insert a fresh line.
    pub fn find_matching_from_import(&self, module: &str) -> Option<&ParsedFromImport> {
        self.from_imports
            .iter()
            .find(|fi| fi.module == module && !fi.has_star())
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Parse the import layout of a Python source file.
///
/// Tries AST-based parsing first (via `rustpython-parser`); falls back to a
/// simpler line-scan if the file has syntax errors (e.g. during active
/// editing).
///
/// The returned [`ImportLayout`] contains:
/// - `groups` — classified contiguous import blocks with line ranges,
/// - `from_imports` / `bare_imports` — individual import statements,
/// - `lines` — the file lines (for `TextEdit` character-length lookups).
pub fn parse_import_layout(content: &str) -> ImportLayout {
    match rustpython_parser::parse(content, Mode::Module, "") {
        Ok(ast) => parse_layout_from_ast(&ast, content),
        Err(e) => {
            warn!("AST parse failed ({e}), using string fallback for import layout");
            parse_layout_from_str(content)
        }
    }
}

/// Classify an import statement string as [`ImportKind::Future`],
/// [`ImportKind::Stdlib`], or [`ImportKind::ThirdParty`].
///
/// **Contract**: only the top-level package name of the *first* module in the
/// statement is examined.  This is intentional: the function is called from
/// `build_import_edits` exclusively with [`TypeImportSpec::import_statement`]
/// values, which are always **single-module** strings such as
/// `"from typing import Any"` or `"import pathlib"`.  Comma-separated
/// multi-module lines (`import os, pytest`) can never reach this function
/// through that path.
///
/// For group classification of raw file lines (the string-fallback parser) the
/// same first-module heuristic is used, matching isort's own group-assignment
/// behaviour.  In practice, mixed-kind lines are a style violation that tools
/// like isort/ruff would split anyway.
pub fn classify_import_statement(statement: &str) -> ImportKind {
    classify_module(top_level_module(statement).unwrap_or(""))
}

/// Sort key for an imported name, stripping any `as alias` suffix.
///
/// `"Path as P"` → `"Path"`, `"Path"` → `"Path"`.
pub fn import_sort_key(name: &str) -> &str {
    match name.find(" as ") {
        Some(pos) => name[..pos].trim(),
        None => name.trim(),
    }
}

/// Sort key for a full import line, following isort / ruff conventions:
///
/// 1. `import X` sorts **before** `from X import Y` (category `0` vs `1`).
/// 2. Within each category, alphabetical by module path (case-insensitive).
///
/// Returns `(category, lowercased_module)`.
pub fn import_line_sort_key(line: &str) -> (u8, String) {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("import ") {
        let module = rest.split_whitespace().next().unwrap_or("");
        (0, module.to_lowercase())
    } else if let Some(rest) = trimmed.strip_prefix("from ") {
        let module = rest.split(" import ").next().unwrap_or("").trim();
        (1, module.to_lowercase())
    } else {
        (2, String::new())
    }
}

/// Find the correct sorted insertion line within an existing import group,
/// so that the result stays isort-sorted (bare before from, alphabetical by
/// module within each sub-category).
///
/// Returns the 0-based line number for a point-insert.  When the new import
/// sorts after every existing line in the group the position is
/// `group.last_line + 1`.
pub fn find_sorted_insert_position(
    lines: &[&str],
    group: &ImportGroup,
    sort_key: &(u8, String),
) -> u32 {
    for (i, line) in lines
        .iter()
        .enumerate()
        .take(group.last_line + 1)
        .skip(group.first_line)
    {
        let existing_key = import_line_sort_key(line);
        if *sort_key < existing_key {
            return i as u32;
        }
    }
    (group.last_line + 1) as u32
}

// ─── adapt_type_for_consumer ──────────────────────────────────────────────────

/// Adapt a fixture's return-type annotation and import specs to the consumer
/// file's existing import context.
///
/// Two adaptations are performed:
///
/// 1. **Dotted → short** — when a fixture uses `import pathlib` (bare) producing
///    `pathlib.Path`, and the consumer already has `from pathlib import Path`,
///    the annotation is shortened to `Path` and the bare-import spec is dropped.
///
/// 2. **Short → dotted** — when a fixture uses `from pathlib import Path`
///    producing the short name `Path`, and the consumer already has
///    `import pathlib` (bare), the annotation is lengthened to `pathlib.Path`
///    and the from-import spec is dropped, respecting the consumer's style.
///
/// Returns `(adapted_type_string, remaining_import_specs)`.
///
/// Callers decide what to do with `remaining_import_specs`:
/// - `code_action` inserts them as new import statements.
/// - `inlay_hint` discards them (display only, no imports inserted).
pub fn adapt_type_for_consumer(
    return_type: &str,
    fixture_imports: &[TypeImportSpec],
    consumer_import_map: &HashMap<String, TypeImportSpec>,
) -> (String, Vec<TypeImportSpec>) {
    let mut adapted = return_type.to_string();
    let mut remaining = Vec::new();

    for spec in fixture_imports {
        if spec.import_statement.starts_with("import ") {
            // ── Case 1: bare-import spec → try dotted-to-short rewrite ───────
            let bare_module = spec
                .import_statement
                .strip_prefix("import ")
                .unwrap()
                .split(" as ")
                .next()
                .unwrap_or("")
                .trim();

            if bare_module.is_empty() {
                remaining.push(spec.clone());
                continue;
            }

            // Look for `check_name.Name` patterns in the type string.
            let prefix = format!("{}.", spec.check_name);
            if !adapted.contains(&prefix) {
                remaining.push(spec.clone());
                continue;
            }

            // Collect every `check_name.Name` occurrence and verify that the
            // consumer already imports `Name` from the same module.
            let mut rewrites: Vec<(String, String)> = Vec::new(); // (dotted, short)
            let mut all_rewritable = true;
            let mut pos = 0;

            while let Some(hit) = adapted[pos..].find(&prefix) {
                let abs = pos + hit;

                // Guard against partial matches (e.g. `mypathlib.X` matching `pathlib.`).
                if abs > 0 {
                    let prev = adapted.as_bytes()[abs - 1];
                    if prev.is_ascii_alphanumeric() || prev == b'_' {
                        pos = abs + prefix.len();
                        continue;
                    }
                }

                let name_start = abs + prefix.len();
                let rest = &adapted[name_start..];
                let name_end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                let name = &rest[..name_end];

                if name.is_empty() {
                    pos = name_start;
                    continue;
                }

                if let Some(consumer_spec) = consumer_import_map.get(name) {
                    let expected = format!("from {} import", bare_module);
                    if consumer_spec.import_statement.starts_with(&expected) {
                        let dotted = format!("{}.{}", spec.check_name, name);
                        if !rewrites.iter().any(|(d, _)| d == &dotted) {
                            rewrites.push((dotted, consumer_spec.check_name.clone()));
                        }
                    } else {
                        // Name imported from a different module — can't safely rewrite.
                        all_rewritable = false;
                        break;
                    }
                } else {
                    // Name not in consumer's import map — can't rewrite.
                    all_rewritable = false;
                    break;
                }

                pos = name_start + name_end;
            }

            if all_rewritable && !rewrites.is_empty() {
                for (dotted, short) in &rewrites {
                    adapted = adapted.replace(dotted.as_str(), short.as_str());
                }
                info!(
                    "Adapted type '{}' → '{}' (consumer already imports short names)",
                    return_type, adapted
                );
            } else {
                // Full-or-nothing: if any dotted name in the type string cannot
                // be safely rewritten, keep the bare-import spec as-is.
                remaining.push(spec.clone());
            }
        } else if let Some((module, name_part)) = split_from_import(&spec.import_statement) {
            // ── Case 2: from-import spec → try short-to-dotted rewrite ───────
            //
            // The fixture uses `from X import Y` so the type string contains
            // the short name `Y`.  If the consumer already has `import X`
            // (bare), we rewrite `Y` → `X.Y` and drop the from-import.

            // Handle `from X import Y as Z` — the original name is `Y`, the
            // check_name (used in the type string) is `Z`.
            let original_name = name_part.split(" as ").next().unwrap_or(name_part).trim();

            if let Some(consumer_module_name) =
                find_consumer_bare_import(consumer_import_map, module)
            {
                let dotted = format!("{}.{}", consumer_module_name, original_name);
                let new_adapted = replace_identifier(&adapted, &spec.check_name, &dotted);
                if new_adapted != adapted {
                    info!(
                        "Adapted type: '{}' → '{}' (consumer has bare import for '{}')",
                        spec.check_name, dotted, module
                    );
                    adapted = new_adapted;
                    // Drop the from-import spec — consumer's bare import covers it.
                } else {
                    // The check_name wasn't found as a standalone identifier in
                    // the type string (word-boundary mismatch).  Keep the spec.
                    remaining.push(spec.clone());
                }
            } else {
                remaining.push(spec.clone());
            }
        } else {
            remaining.push(spec.clone());
        }
    }

    (adapted, remaining)
}

/// Find the `check_name` used by the consumer for a bare `import X` of the
/// given module.  Returns `None` if the consumer does not have such an import.
pub(crate) fn find_consumer_bare_import<'a>(
    consumer_import_map: &'a HashMap<String, TypeImportSpec>,
    module: &str,
) -> Option<&'a str> {
    for spec in consumer_import_map.values() {
        if let Some(rest) = spec.import_statement.strip_prefix("import ") {
            let module_part = rest.split(" as ").next().unwrap_or("").trim();
            if module_part == module {
                return Some(&spec.check_name);
            }
        }
    }
    None
}

/// Returns `true` when we can safely merge new names into an existing
/// [`ParsedFromImport`] — i.e. it is not a star-import, and it either has
/// known names (AST path) or is single-line (both paths).
///
/// A multiline import from the string-fallback parser has `names.is_empty()`
/// because individual names cannot be reliably extracted line-by-line from a
/// file that failed to parse.  Merging into such an entry would lose existing
/// names, so we fall back to inserting a new line instead.
pub(crate) fn can_merge_into(fi: &ParsedFromImport) -> bool {
    !(fi.has_star() || fi.is_multiline && fi.names.is_empty())
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Classify a module name string as Future, Stdlib, or ThirdParty.
fn classify_module(module: &str) -> ImportKind {
    if module == "__future__" {
        ImportKind::Future
    } else if is_stdlib_module(module) {
        ImportKind::Stdlib
    } else {
        ImportKind::ThirdParty
    }
}

/// Return the more restrictive of two import kinds.
///
/// Priority order: `Future` > `ThirdParty` > `Stdlib`.  This is a **binary
/// reducer**, not the full algorithm.  The full N-module case is handled by
/// composing this function with [`Iterator::fold`]:
///
/// ```text
/// import os, sys, pytest, flask
///   fold(Stdlib, merge_kinds):
///     merge_kinds(Stdlib,      Stdlib)      // os      → Stdlib
///     merge_kinds(Stdlib,      Stdlib)      // sys     → Stdlib
///     merge_kinds(Stdlib,      ThirdParty)  // pytest  → ThirdParty
///     merge_kinds(ThirdParty,  ThirdParty)  // flask   → ThirdParty
///   result: ThirdParty  ✓
/// ```
///
/// See [`classify_import_line`] for the call site that applies this to an
/// arbitrary number of comma-separated modules.
fn merge_kinds(a: ImportKind, b: ImportKind) -> ImportKind {
    match (a, b) {
        (ImportKind::Future, _) | (_, ImportKind::Future) => ImportKind::Future,
        (ImportKind::ThirdParty, _) | (_, ImportKind::ThirdParty) => ImportKind::ThirdParty,
        _ => ImportKind::Stdlib,
    }
}

/// Classify an import **line** by inspecting *all* named modules, returning
/// the most restrictive [`ImportKind`] found.
///
/// For a comma-separated bare import such as `import os, pytest`, every
/// module is examined and `ThirdParty` wins over `Stdlib` (and `Future` wins
/// over both).  This prevents a mixed line from being misclassified as
/// `Stdlib` simply because the first module happens to be a stdlib package.
///
/// `from X import Y` lines have exactly one module, so the result is the
/// same as calling [`classify_module`] directly.
fn classify_import_line(line: &str) -> ImportKind {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("from ") {
        // `from X import Y` — exactly one module.
        let module = rest.split_whitespace().next().unwrap_or("");
        classify_module(module.split('.').next().unwrap_or(""))
    } else if let Some(rest) = trimmed.strip_prefix("import ") {
        // `import os, sys, pytest` — check every comma-separated module.
        rest.split(',')
            .filter_map(|part| {
                let name = part.split_whitespace().next()?;
                Some(classify_module(name.split('.').next().unwrap_or("")))
            })
            .fold(ImportKind::Stdlib, merge_kinds)
    } else {
        ImportKind::ThirdParty
    }
}

/// Extract the top-level package name from the **first** module in an import
/// line.  Only used for the `split_from_import` helper; all group-
/// classification code should use [`classify_import_line`] instead.
///
/// - `"from collections.abc import X"` → `Some("collections")`
/// - `"import os, sys"` → `Some("os")` (first module only)
/// - `"x = 1"` → `None`
fn top_level_module(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("from ") {
        let module = rest.split_whitespace().next()?;
        module.split('.').next()
    } else if let Some(rest) = trimmed.strip_prefix("import ") {
        let first = rest.split(',').next()?.trim();
        let first = first.split_whitespace().next()?;
        first.split('.').next()
    } else {
        None
    }
}

/// Split `"from X import Y"` into `Some(("X", "Y"))`, or return `None` for
/// bare `import X` statements and other non-matching strings.
fn split_from_import(statement: &str) -> Option<(&str, &str)> {
    let rest = statement.strip_prefix("from ")?;
    let (module, rest) = rest.split_once(" import ")?;
    let module = module.trim();
    let name = rest.trim();
    if module.is_empty() || name.is_empty() {
        None
    } else {
        Some((module, name))
    }
}

// ─── AST-based parser ─────────────────────────────────────────────────────────

fn parse_layout_from_ast(ast: &rustpython_parser::ast::Mod, content: &str) -> ImportLayout {
    let line_starts = build_line_starts(content);
    let offset_to_line = |offset: usize| -> usize {
        line_starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1)
    };

    let mut from_imports: Vec<ParsedFromImport> = Vec::new();
    let mut bare_imports: Vec<ParsedBareImport> = Vec::new();

    let body = match ast {
        Mod::Module(m) => &m.body,
        _ => return parse_layout_from_str(content),
    };

    for stmt in body {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                let start_byte = import_from.range.start().to_usize();
                let end_byte = import_from.range.end().to_usize();
                let line = offset_to_line(start_byte);
                // end() is exclusive; saturating_sub(1) gives the last byte of
                // the statement, which is always on the same logical line as `)`
                // for a multiline import.
                let end_line = offset_to_line(end_byte.saturating_sub(1));

                let mut module = import_from
                    .module
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_default();
                if let Some(ref level) = import_from.level {
                    let level_val = level.to_usize();
                    if level_val > 0 {
                        let dots = ".".repeat(level_val);
                        module = dots + &module;
                    }
                }

                let names: Vec<ImportedName> = import_from
                    .names
                    .iter()
                    .map(|alias| ImportedName {
                        name: alias.name.to_string(),
                        alias: alias.asname.as_ref().map(|a| a.to_string()),
                    })
                    .collect();

                let is_multiline = end_line > line;

                from_imports.push(ParsedFromImport {
                    line,
                    end_line,
                    module,
                    names,
                    is_multiline,
                });
            }
            Stmt::Import(import_stmt) => {
                let start_byte = import_stmt.range.start().to_usize();
                let line = offset_to_line(start_byte);
                for alias in &import_stmt.names {
                    bare_imports.push(ParsedBareImport {
                        line,
                        module: alias.name.to_string(),
                        alias: alias.asname.as_ref().map(|a| a.to_string()),
                    });
                }
            }
            _ => {}
        }
    }

    let groups = build_groups_from_ast(&from_imports, &bare_imports);
    ImportLayout::new(
        groups,
        from_imports,
        bare_imports,
        ParseSource::Ast,
        content,
    )
}

/// Build a byte-offset → 0-based-line-number lookup table from file content.
///
/// `result[i]` = byte offset of the start of line `i`.
fn build_line_starts(content: &str) -> Vec<usize> {
    let bytes = content.as_bytes();
    let mut starts = vec![0usize];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// A lightweight event representing one import statement's line span and
/// top-level module name, used only during group construction.
struct ImportEvent {
    first_line: usize,
    last_line: usize,
    top_module: String,
}

/// Group AST-derived import events into [`ImportGroup`]s.
///
/// Two imports are merged into the same group when the next import's first
/// line is at most one greater than the previous import's last line (i.e. no
/// blank line between them).  Comments in Python have no AST representation,
/// so a comment between two imports creates a natural gap that is treated as a
/// group boundary — matching the behaviour of the string-fallback parser.
fn build_groups_from_ast(
    from_imports: &[ParsedFromImport],
    bare_imports: &[ParsedBareImport],
) -> Vec<ImportGroup> {
    let mut events: Vec<ImportEvent> = Vec::new();

    for fi in from_imports {
        let top = fi
            .module
            .trim_start_matches('.')
            .split('.')
            .next()
            .unwrap_or("")
            .to_string();
        events.push(ImportEvent {
            first_line: fi.line,
            last_line: fi.end_line,
            top_module: top,
        });
    }

    for bi in bare_imports {
        // For `import os, sys` the AST yields two ParsedBareImport entries on
        // the same line.  We emit an event for *every* entry so that the
        // grouping step below can merge their kinds via `merge_kinds` — this
        // correctly classifies `import os, pytest` as ThirdParty rather than
        // Stdlib (first-module-wins).
        let top = bi.module.split('.').next().unwrap_or("").to_string();
        events.push(ImportEvent {
            first_line: bi.line,
            last_line: bi.line,
            top_module: top,
        });
    }

    events.sort_by_key(|e| e.first_line);

    let mut groups: Vec<ImportGroup> = Vec::new();
    for event in events {
        match groups.last_mut() {
            // Adjacent or overlapping — extend the current group and update its
            // kind to the most restrictive seen so far.  This handles both
            // normal consecutive imports (different lines) and same-line
            // comma-separated bare imports (same line, multiple events).
            Some(g) if event.first_line <= g.last_line + 1 => {
                g.last_line = g.last_line.max(event.last_line);
                g.kind = merge_kinds(g.kind, classify_module(&event.top_module));
            }
            // Gap (blank line or comment) — start a new group.
            _ => {
                let kind = classify_module(&event.top_module);
                groups.push(ImportGroup {
                    first_line: event.first_line,
                    last_line: event.last_line,
                    kind,
                });
            }
        }
    }

    groups
}

// ─── String-fallback parser ───────────────────────────────────────────────────

/// Line-scan fallback for files that fail to parse as valid Python.
///
/// Mirrors the behaviour of the original `parse_import_groups` function and
/// additionally populates `from_imports` and `bare_imports`.
fn parse_layout_from_str(content: &str) -> ImportLayout {
    let lines: Vec<&str> = content.lines().collect();
    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut from_imports: Vec<ParsedFromImport> = Vec::new();
    let mut bare_imports: Vec<ParsedBareImport> = Vec::new();

    let mut current_start: Option<usize> = None;
    let mut current_last: usize = 0;
    let mut current_kind = ImportKind::ThirdParty;
    let mut seen_any_import = false;
    let mut in_multiline = false;
    let mut multiline_start: usize = 0;
    let mut multiline_module: String = String::new();

    for (i, &line) in lines.iter().enumerate() {
        // ── Consume lines inside a parenthesised multiline import ────────────
        if in_multiline {
            current_last = i;
            let line_no_comment = line.split('#').next().unwrap_or("").trim_end();
            if line_no_comment.contains(')') {
                from_imports.push(ParsedFromImport {
                    line: multiline_start,
                    end_line: i,
                    module: multiline_module.clone(),
                    // Names cannot be reliably parsed from a fallback multiline;
                    // see can_merge_into() for how callers handle this.
                    names: vec![],
                    is_multiline: true,
                });
                in_multiline = false;
            }
            continue;
        }

        // ── Module-level (unindented) import line ────────────────────────────
        if line.starts_with("import ") || line.starts_with("from ") {
            seen_any_import = true;
            if current_start.is_none() {
                current_start = Some(i);
                current_kind = classify_import_line(line);
            } else {
                // A subsequent line in the same group: merge its kind so that a
                // mixed line (e.g. `import os, pytest`) cannot downgrade the
                // group's classification retroactively.
                current_kind = merge_kinds(current_kind, classify_import_line(line));
            }
            current_last = i;

            if let Some(rest) = line.strip_prefix("from ") {
                let module = rest
                    .split(" import ")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let line_no_comment = line.split('#').next().unwrap_or("").trim_end();
                if line_no_comment.contains('(') && !line_no_comment.contains(')') {
                    // Opening of a multiline import.
                    in_multiline = true;
                    multiline_start = i;
                    multiline_module = module;
                } else {
                    // Single-line from-import.
                    if let Some(names_raw) = rest.split(" import ").nth(1) {
                        let names_str = names_raw.split('#').next().unwrap_or("").trim_end();
                        let names: Vec<ImportedName> = names_str
                            .split(',')
                            .filter_map(|n| {
                                let n = n.trim();
                                if n.is_empty() {
                                    return None;
                                }
                                if let Some((name, alias)) = n.split_once(" as ") {
                                    Some(ImportedName {
                                        name: name.trim().to_string(),
                                        alias: Some(alias.trim().to_string()),
                                    })
                                } else {
                                    Some(ImportedName {
                                        name: n.to_string(),
                                        alias: None,
                                    })
                                }
                            })
                            .collect();
                        from_imports.push(ParsedFromImport {
                            line: i,
                            end_line: i,
                            module,
                            names,
                            is_multiline: false,
                        });
                    }
                }
            } else if let Some(rest) = line.strip_prefix("import ") {
                // Bare imports (possibly comma-separated, e.g. `import os, sys`).
                for part in rest.split(',') {
                    let part = part.trim();
                    let (module_str, alias) = if let Some((m, a)) = part.split_once(" as ") {
                        (m.trim().to_string(), Some(a.trim().to_string()))
                    } else {
                        let m = part.split_whitespace().next().unwrap_or(part);
                        (m.to_string(), None)
                    };
                    if !module_str.is_empty() {
                        bare_imports.push(ParsedBareImport {
                            line: i,
                            module: module_str,
                            alias,
                        });
                    }
                }
            }
            continue;
        }

        let trimmed = line.trim();

        // ── Blank line or comment — close current group, keep scanning ───────
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if let Some(start) = current_start.take() {
                groups.push(ImportGroup {
                    first_line: start,
                    last_line: current_last,
                    kind: current_kind,
                });
            }
            continue;
        }

        // ── Non-import, non-blank — stop if we've seen any import ────────────
        if seen_any_import {
            if let Some(start) = current_start.take() {
                groups.push(ImportGroup {
                    first_line: start,
                    last_line: current_last,
                    kind: current_kind,
                });
            }
            break;
        }
        // Before any import: preamble (module docstring, shebang, etc.) — keep going.
    }

    // Close final group if the file ends while still in an import section.
    if let Some(start) = current_start {
        groups.push(ImportGroup {
            first_line: start,
            last_line: current_last,
            kind: current_kind,
        });
    }

    ImportLayout::new(
        groups,
        from_imports,
        bare_imports,
        ParseSource::StringFallback,
        content,
    )
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::types::TypeImportSpec;
    use std::collections::HashMap;

    // ── helper ───────────────────────────────────────────────────────────

    fn spec(check_name: &str, import_statement: &str) -> TypeImportSpec {
        TypeImportSpec {
            check_name: check_name.to_string(),
            import_statement: import_statement.to_string(),
        }
    }

    /// Build an ImportLayout from a slice of lines joined with newlines.
    fn layout(lines: &[&str]) -> ImportLayout {
        parse_import_layout(&lines.join("\n"))
    }

    // ── classify_import_statement ─────────────────────────────────────────
    //
    // classify_import_statement uses first-module-wins (intentional: it is
    // only called from build_import_edits with single-module TypeImportSpec
    // strings).  Group classification of raw file lines goes through
    // classify_import_line, which checks *all* modules — see the
    // parse_layout group tests below for that behaviour.

    #[test]
    fn test_classify_future() {
        assert_eq!(
            classify_import_statement("from __future__ import annotations"),
            ImportKind::Future
        );
    }

    #[test]
    fn test_classify_stdlib() {
        assert_eq!(
            classify_import_statement("from typing import Any"),
            ImportKind::Stdlib
        );
        assert_eq!(
            classify_import_statement("import pathlib"),
            ImportKind::Stdlib
        );
        assert_eq!(
            classify_import_statement("from collections.abc import Sequence"),
            ImportKind::Stdlib
        );
    }

    #[test]
    fn test_classify_third_party() {
        assert_eq!(
            classify_import_statement("import pytest"),
            ImportKind::ThirdParty
        );
        assert_eq!(
            classify_import_statement("from myapp.db import Database"),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_classify_comma_separated_stdlib() {
        // Both modules are stdlib — unambiguous result.
        assert_eq!(
            classify_import_statement("import os, sys"),
            ImportKind::Stdlib
        );
    }

    #[test]
    fn test_classify_comma_separated_mixed_kinds_first_module_wins() {
        // classify_import_statement is called with single-module TypeImportSpec
        // strings only — it intentionally uses first-module-wins.  The mixed
        // case is handled correctly at the layout level by classify_import_line
        // (tested via test_parse_groups_mixed_bare_import_* below).
        assert_eq!(
            classify_import_statement("import os, pytest"),
            ImportKind::Stdlib // first-module-wins: os is stdlib
        );
        assert_eq!(
            classify_import_statement("import pytest, os"),
            ImportKind::ThirdParty // first-module-wins: pytest is third-party
        );
    }

    // ── merge_kinds ───────────────────────────────────────────────────────

    #[test]
    fn test_merge_kinds_future_wins_over_all() {
        assert_eq!(
            merge_kinds(ImportKind::Future, ImportKind::Stdlib),
            ImportKind::Future
        );
        assert_eq!(
            merge_kinds(ImportKind::Future, ImportKind::ThirdParty),
            ImportKind::Future
        );
        assert_eq!(
            merge_kinds(ImportKind::Stdlib, ImportKind::Future),
            ImportKind::Future
        );
    }

    #[test]
    fn test_merge_kinds_third_party_wins_over_stdlib() {
        assert_eq!(
            merge_kinds(ImportKind::ThirdParty, ImportKind::Stdlib),
            ImportKind::ThirdParty
        );
        assert_eq!(
            merge_kinds(ImportKind::Stdlib, ImportKind::ThirdParty),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_merge_kinds_same_kind_unchanged() {
        assert_eq!(
            merge_kinds(ImportKind::Stdlib, ImportKind::Stdlib),
            ImportKind::Stdlib
        );
        assert_eq!(
            merge_kinds(ImportKind::ThirdParty, ImportKind::ThirdParty),
            ImportKind::ThirdParty
        );
    }

    // ── classify_import_line ──────────────────────────────────────────────

    #[test]
    fn test_classify_import_line_all_stdlib() {
        assert_eq!(classify_import_line("import os, sys"), ImportKind::Stdlib);
    }

    #[test]
    fn test_classify_import_line_all_third_party() {
        assert_eq!(
            classify_import_line("import pytest, flask"),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_classify_import_line_mixed_stdlib_first() {
        // os is stdlib, pytest is third-party → ThirdParty wins regardless of order.
        assert_eq!(
            classify_import_line("import os, pytest"),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_classify_import_line_mixed_third_party_first() {
        assert_eq!(
            classify_import_line("import pytest, os"),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_classify_import_line_three_modules_mixed() {
        // fold applies merge_kinds once per module — arbitrary length works.
        // os → Stdlib, sys → Stdlib, pytest → ThirdParty: result is ThirdParty.
        assert_eq!(
            classify_import_line("import os, sys, pytest"),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_classify_import_line_four_modules_stdlib_only() {
        // All four stdlib → Stdlib.
        assert_eq!(
            classify_import_line("import os, sys, re, pathlib"),
            ImportKind::Stdlib
        );
    }

    #[test]
    fn test_classify_import_line_four_modules_third_party_last() {
        // Third-party appears last — fold must still catch it.
        assert_eq!(
            classify_import_line("import os, sys, re, pytest"),
            ImportKind::ThirdParty
        );
    }

    #[test]
    fn test_parse_groups_three_module_mixed_bare_import() {
        // Three modules on one line; third-party is last → group is ThirdParty.
        let l = layout(&["import os, sys, pytest", "", "def test(): pass"]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_classify_import_line_from_import_unaffected() {
        // from-imports have exactly one module; behaviour is unchanged.
        assert_eq!(
            classify_import_line("from typing import Any"),
            ImportKind::Stdlib
        );
        assert_eq!(
            classify_import_line("from flask import Flask"),
            ImportKind::ThirdParty
        );
    }

    // ── parse_import_layout — mixed bare imports ──────────────────────────

    #[test]
    fn test_parse_groups_mixed_bare_import_classified_as_third_party() {
        // `import os, pytest` contains a third-party module — the group must
        // be ThirdParty regardless of which module appears first.
        let l = layout(&["import os, pytest", "", "def test(): pass"]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_mixed_bare_import_order_independent() {
        // Result must be the same regardless of which module comes first.
        let l = layout(&["import pytest, os", "", "def test(): pass"]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_all_stdlib_bare_import_unchanged() {
        let l = layout(&["import os, sys", "", "def test(): pass"]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].kind, ImportKind::Stdlib);
    }

    #[test]
    fn test_parse_groups_fallback_mixed_bare_import() {
        // Same assertion must hold for the string-fallback path.
        let l = parse_import_layout("import os, pytest\ndef test(:\n    pass");
        assert_eq!(l.source, ParseSource::StringFallback);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
    }

    // ── parse_import_layout — parse source tracking ───────────────────────

    #[test]
    fn test_parse_layout_uses_ast_for_valid_python() {
        let l = layout(&["import os", "", "def test(): pass"]);
        assert_eq!(l.source, ParseSource::Ast);
    }

    #[test]
    fn test_parse_layout_falls_back_for_invalid_python() {
        let l = parse_import_layout("import os\ndef test(:\n    pass");
        assert_eq!(l.source, ParseSource::StringFallback);
    }

    // ── parse_import_layout — groups ──────────────────────────────────────

    #[test]
    fn test_parse_groups_stdlib_and_third_party() {
        let l = layout(&[
            "import time",
            "",
            "import pytest",
            "from vcc.framework import fixture",
            "",
            "LOGGING_TIME = 2",
        ]);
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 0);
        assert_eq!(l.groups[0].kind, ImportKind::Stdlib);
        assert_eq!(l.groups[1].first_line, 2);
        assert_eq!(l.groups[1].last_line, 3);
        assert_eq!(l.groups[1].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_single_third_party() {
        let l = layout(&["import pytest", "", "def test(): pass"]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 0);
    }

    #[test]
    fn test_parse_groups_no_imports() {
        let l = layout(&["def test(): pass"]);
        assert!(l.groups.is_empty());
    }

    #[test]
    fn test_parse_groups_empty_file() {
        let l = layout(&[]);
        assert!(l.groups.is_empty());
    }

    #[test]
    fn test_parse_groups_with_docstring_preamble() {
        let l = layout(&[
            r#""""Module docstring.""""#,
            "",
            "import pytest",
            "from pathlib import Path",
            "",
            "def test(): pass",
        ]);
        // pytest and pathlib are in the same contiguous block; group classified
        // by first import (pytest → ThirdParty).
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].first_line, 2);
        assert_eq!(l.groups[0].last_line, 3);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_ignores_indented_imports() {
        let l = layout(&[
            "import pytest",
            "",
            "def test():",
            "    from .utils import helper",
            "    import os",
        ]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 0);
    }

    #[test]
    fn test_parse_groups_future_then_stdlib_then_third_party() {
        let l = layout(&[
            "from __future__ import annotations",
            "",
            "import os",
            "import time",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ]);
        assert_eq!(l.groups.len(), 3);
        assert_eq!(l.groups[0].kind, ImportKind::Future);
        assert_eq!(l.groups[1].kind, ImportKind::Stdlib); // os, time
        assert_eq!(l.groups[2].kind, ImportKind::ThirdParty); // pytest
    }

    #[test]
    fn test_parse_groups_with_comments_between() {
        let l = layout(&[
            "import os",
            "# stdlib above, third-party below",
            "import pytest",
            "",
            "def test(): pass",
        ]);
        // Comment closes the first group, starts a new one.
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].kind, ImportKind::Stdlib);
        assert_eq!(l.groups[0].last_line, 0);
        assert_eq!(l.groups[1].kind, ImportKind::ThirdParty);
        assert_eq!(l.groups[1].first_line, 2);
    }

    #[test]
    fn test_parse_groups_comma_separated_import_is_stdlib() {
        let l = layout(&[
            "import os, sys",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ]);
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].kind, ImportKind::Stdlib);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 0);
        assert_eq!(l.groups[1].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_multiline_import_single_group() {
        let l = layout(&["from liba import (", "    moda,", "    modb", ")"]);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 3);
        assert_eq!(l.groups[0].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_multiline_import_followed_by_third_party() {
        let l = layout(&[
            "from liba import (",
            "    moda,",
            "    modb",
            ")",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ]);
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 3);
        assert_eq!(l.groups[1].first_line, 5);
        assert_eq!(l.groups[1].last_line, 5);
        assert_eq!(l.groups[1].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_multiline_stdlib_then_third_party() {
        let l = layout(&[
            "from typing import (",
            "    Any,",
            "    Optional,",
            ")",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ]);
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].kind, ImportKind::Stdlib);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 3);
        assert_eq!(l.groups[1].kind, ImportKind::ThirdParty);
        assert_eq!(l.groups[1].first_line, 5);
        assert_eq!(l.groups[1].last_line, 5);
    }

    #[test]
    fn test_parse_groups_inline_multiline_import() {
        let l = layout(&[
            "from typing import (Any,",
            "    Optional)",
            "",
            "import pytest",
        ]);
        assert_eq!(l.groups.len(), 2);
        assert_eq!(l.groups[0].kind, ImportKind::Stdlib);
        assert_eq!(l.groups[0].first_line, 0);
        assert_eq!(l.groups[0].last_line, 1);
        assert_eq!(l.groups[1].kind, ImportKind::ThirdParty);
        assert_eq!(l.groups[1].first_line, 3);
        assert_eq!(l.groups[1].last_line, 3);
    }

    // ── parse_import_layout — from_imports / bare_imports fields ─────────

    #[test]
    fn test_from_imports_single_line() {
        let l = layout(&["from typing import Any, Optional"]);
        assert_eq!(l.from_imports.len(), 1);
        let fi = &l.from_imports[0];
        assert_eq!(fi.module, "typing");
        assert_eq!(fi.line, 0);
        assert_eq!(fi.end_line, 0);
        assert!(!fi.is_multiline);
        assert_eq!(fi.name_strings(), vec!["Any", "Optional"]);
    }

    #[test]
    fn test_from_imports_with_alias() {
        let l = layout(&["from pathlib import Path as P"]);
        let fi = &l.from_imports[0];
        assert_eq!(fi.module, "pathlib");
        assert_eq!(fi.name_strings(), vec!["Path as P"]);
    }

    #[test]
    fn test_from_imports_multiline_has_correct_end_line() {
        let l = layout(&["from typing import (", "    Any,", "    Optional,", ")"]);
        assert_eq!(l.from_imports.len(), 1);
        let fi = &l.from_imports[0];
        assert_eq!(fi.line, 0);
        assert_eq!(fi.end_line, 3);
        assert!(fi.is_multiline);
        // AST path populates names; fallback path leaves them empty.
        // For valid Python (AST path), names must be present.
        if l.source == ParseSource::Ast {
            assert_eq!(fi.name_strings(), vec!["Any", "Optional"]);
        }
    }

    #[test]
    fn test_bare_imports_comma_separated() {
        let l = layout(&["import os, sys"]);
        assert_eq!(l.bare_imports.len(), 2);
        assert_eq!(l.bare_imports[0].module, "os");
        assert_eq!(l.bare_imports[1].module, "sys");
        // Both on line 0.
        assert_eq!(l.bare_imports[0].line, 0);
        assert_eq!(l.bare_imports[1].line, 0);
    }

    #[test]
    fn test_bare_import_with_alias() {
        let l = layout(&["import pathlib as pl"]);
        assert_eq!(l.bare_imports.len(), 1);
        assert_eq!(l.bare_imports[0].module, "pathlib");
        assert_eq!(l.bare_imports[0].alias, Some("pl".to_string()));
    }

    // ── ImportLayout::find_matching_from_import ───────────────────────────

    #[test]
    fn test_find_matching_found() {
        let l = layout(&[
            "import pytest",
            "from typing import Optional",
            "",
            "def test(): pass",
        ]);
        let fi = l.find_matching_from_import("typing");
        assert!(fi.is_some());
        assert_eq!(fi.unwrap().name_strings(), vec!["Optional"]);
    }

    #[test]
    fn test_find_matching_multiple_names() {
        let l = layout(&["from typing import Any, Optional, Union"]);
        let fi = l.find_matching_from_import("typing").unwrap();
        assert_eq!(fi.name_strings(), vec!["Any", "Optional", "Union"]);
    }

    #[test]
    fn test_find_matching_not_found() {
        let l = layout(&["import pytest", "from pathlib import Path"]);
        assert!(l.find_matching_from_import("typing").is_none());
    }

    #[test]
    fn test_find_matching_returns_multiline() {
        // New capability: multiline imports are now returned (not skipped).
        let l = layout(&["from typing import (", "    Any,", "    Optional,", ")"]);
        let fi = l.find_matching_from_import("typing");
        assert!(fi.is_some(), "multiline match should be returned");
        assert!(fi.unwrap().is_multiline);
    }

    #[test]
    fn test_find_matching_skips_star() {
        let l = layout(&["from typing import *"]);
        assert!(l.find_matching_from_import("typing").is_none());
    }

    #[test]
    fn test_find_matching_ignores_indented() {
        // Indented imports are not in module.body → not in from_imports.
        let l = layout(&[
            "import pytest",
            "",
            "def test():",
            "    from typing import Any",
        ]);
        assert!(l.find_matching_from_import("typing").is_none());
    }

    #[test]
    fn test_find_matching_with_inline_comment() {
        let l = layout(&["from typing import Any  # comment"]);
        let fi = l.find_matching_from_import("typing").unwrap();
        // Comment must NOT appear in name_strings.
        assert_eq!(fi.name_strings(), vec!["Any"]);
    }

    #[test]
    fn test_find_matching_aliases_preserved() {
        let l = layout(&["from os import path as p, getcwd as cwd"]);
        let fi = l.find_matching_from_import("os").unwrap();
        assert_eq!(fi.name_strings(), vec!["path as p", "getcwd as cwd"]);
    }

    // ── can_merge_into ────────────────────────────────────────────────────

    #[test]
    fn test_can_merge_single_line() {
        let fi = ParsedFromImport {
            line: 0,
            end_line: 0,
            module: "typing".to_string(),
            names: vec![ImportedName {
                name: "Any".to_string(),
                alias: None,
            }],
            is_multiline: false,
        };
        assert!(can_merge_into(&fi));
    }

    #[test]
    fn test_can_merge_multiline_with_names() {
        // AST path: multiline but names are populated → can merge.
        let fi = ParsedFromImport {
            line: 0,
            end_line: 3,
            module: "typing".to_string(),
            names: vec![ImportedName {
                name: "Any".to_string(),
                alias: None,
            }],
            is_multiline: true,
        };
        assert!(can_merge_into(&fi));
    }

    #[test]
    fn test_cannot_merge_multiline_without_names() {
        // String-fallback path: multiline with empty names → cannot merge.
        let fi = ParsedFromImport {
            line: 0,
            end_line: 3,
            module: "typing".to_string(),
            names: vec![],
            is_multiline: true,
        };
        assert!(!can_merge_into(&fi));
    }

    #[test]
    fn test_cannot_merge_star() {
        let fi = ParsedFromImport {
            line: 0,
            end_line: 0,
            module: "typing".to_string(),
            names: vec![ImportedName {
                name: "*".to_string(),
                alias: None,
            }],
            is_multiline: false,
        };
        assert!(!can_merge_into(&fi));
    }

    // ── import_sort_key ───────────────────────────────────────────────────

    #[test]
    fn test_import_sort_key_plain() {
        assert_eq!(import_sort_key("Path"), "Path");
    }

    #[test]
    fn test_import_sort_key_alias() {
        assert_eq!(import_sort_key("Path as P"), "Path");
    }

    // ── import_line_sort_key ──────────────────────────────────────────────

    #[test]
    fn test_import_line_sort_key_bare_before_from() {
        let bare = import_line_sort_key("import os");
        let from = import_line_sort_key("from typing import Any");
        assert!(bare < from, "bare imports should sort before from-imports");
    }

    #[test]
    fn test_import_line_sort_key_alphabetical_bare() {
        let a = import_line_sort_key("import os");
        let b = import_line_sort_key("import pathlib");
        let c = import_line_sort_key("import time");
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn test_import_line_sort_key_alphabetical_from() {
        let a = import_line_sort_key("from pathlib import Path");
        let b = import_line_sort_key("from typing import Any");
        assert!(a < b);
    }

    #[test]
    fn test_import_line_sort_key_dotted_module_ordering() {
        let short = import_line_sort_key("from vcc import conx_canoe");
        let long = import_line_sort_key("from vcc.conxtfw.framework import fixture");
        assert!(
            short < long,
            "shorter module path should sort before longer"
        );
    }

    // ── find_sorted_insert_position ───────────────────────────────────────

    #[test]
    fn test_sorted_position_bare_before_existing_bare() {
        let lines = vec!["import os", "import time"];
        let group = ImportGroup {
            first_line: 0,
            last_line: 1,
            kind: ImportKind::Stdlib,
        };
        let key = import_line_sort_key("import pathlib");
        assert_eq!(find_sorted_insert_position(&lines, &group, &key), 1);
    }

    #[test]
    fn test_sorted_position_from_after_all_bare() {
        let lines = vec!["import os", "import time"];
        let group = ImportGroup {
            first_line: 0,
            last_line: 1,
            kind: ImportKind::Stdlib,
        };
        let key = import_line_sort_key("from typing import Any");
        assert_eq!(find_sorted_insert_position(&lines, &group, &key), 2);
    }

    #[test]
    fn test_sorted_position_from_between_existing_froms() {
        let lines = vec!["import pytest", "from aaa import X", "from zzz import Y"];
        let group = ImportGroup {
            first_line: 0,
            last_line: 2,
            kind: ImportKind::ThirdParty,
        };
        let key = import_line_sort_key("from mmm import Z");
        assert_eq!(find_sorted_insert_position(&lines, &group, &key), 2);
    }

    #[test]
    fn test_sorted_position_before_everything() {
        let lines = vec!["import time", "from typing import Any"];
        let group = ImportGroup {
            first_line: 0,
            last_line: 1,
            kind: ImportKind::Stdlib,
        };
        let key = import_line_sort_key("import os");
        assert_eq!(find_sorted_insert_position(&lines, &group, &key), 0);
    }

    // ── adapt_type_for_consumer ───────────────────────────────────────────

    #[test]
    fn test_adapt_dotted_to_short_when_consumer_has_from_import() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        let (adapted, remaining) =
            adapt_type_for_consumer("pathlib.Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "Path");
        assert!(
            remaining.is_empty(),
            "No import should remain: {:?}",
            remaining
        );
    }

    #[test]
    fn test_adapt_no_rewrite_when_consumer_lacks_from_import() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let consumer_map = HashMap::new();
        let (adapted, remaining) =
            adapt_type_for_consumer("pathlib.Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "pathlib.Path");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].import_statement, "import pathlib");
    }

    #[test]
    fn test_adapt_no_rewrite_when_consumer_imports_from_different_module() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from mylib import Path"));
        let (adapted, remaining) =
            adapt_type_for_consumer("pathlib.Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "pathlib.Path");
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_adapt_from_import_specs_pass_through_unchanged() {
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let consumer_map = HashMap::new();
        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "Path");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Path");
    }

    #[test]
    fn test_adapt_complex_generic_with_dotted_and_from() {
        let fixture_imports = vec![
            spec("Optional", "from typing import Optional"),
            spec("pathlib", "import pathlib"),
        ];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        consumer_map.insert(
            "Optional".to_string(),
            spec("Optional", "from typing import Optional"),
        );
        let (adapted, remaining) =
            adapt_type_for_consumer("Optional[pathlib.Path]", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "Optional[Path]");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Optional");
    }

    #[test]
    fn test_adapt_multiple_dotted_refs_same_module() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        consumer_map.insert(
            "PurePath".to_string(),
            spec("PurePath", "from pathlib import PurePath"),
        );
        let (adapted, remaining) = adapt_type_for_consumer(
            "tuple[pathlib.Path, pathlib.PurePath]",
            &fixture_imports,
            &consumer_map,
        );
        assert_eq!(adapted, "tuple[Path, PurePath]");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_partial_match_one_name_missing() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        let (adapted, remaining) = adapt_type_for_consumer(
            "tuple[pathlib.Path, pathlib.PurePath]",
            &fixture_imports,
            &consumer_map,
        );
        assert_eq!(adapted, "tuple[pathlib.Path, pathlib.PurePath]");
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_adapt_aliased_bare_import() {
        let fixture_imports = vec![spec("pl", "import pathlib as pl")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        let (adapted, remaining) =
            adapt_type_for_consumer("pl.Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "Path");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_no_false_match_on_prefix_substring() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        let (adapted, remaining) =
            adapt_type_for_consumer("mypathlib.Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "mypathlib.Path");
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_adapt_dotted_module_collections_abc() {
        let fixture_imports = vec![spec("collections.abc", "import collections.abc")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert(
            "Iterable".to_string(),
            spec("Iterable", "from collections.abc import Iterable"),
        );
        let (adapted, remaining) = adapt_type_for_consumer(
            "collections.abc.Iterable[str]",
            &fixture_imports,
            &consumer_map,
        );
        assert_eq!(adapted, "Iterable[str]");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_consumer_has_bare_import_no_rewrite() {
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));
        let (adapted, remaining) =
            adapt_type_for_consumer("pathlib.Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "pathlib.Path");
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_adapt_short_to_dotted_when_consumer_has_bare_import() {
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));
        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "pathlib.Path");
        assert!(
            remaining.is_empty(),
            "No import should remain: {:?}",
            remaining
        );
    }

    #[test]
    fn test_adapt_short_to_dotted_consumer_has_aliased_bare_import() {
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pl".to_string(), spec("pl", "import pathlib as pl"));
        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "pl.Path");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_short_no_rewrite_when_consumer_lacks_bare_import() {
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let consumer_map = HashMap::new();
        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "Path");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Path");
    }

    #[test]
    fn test_adapt_short_to_dotted_generic_type() {
        let fixture_imports = vec![
            spec("Optional", "from typing import Optional"),
            spec("Path", "from pathlib import Path"),
        ];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));
        let (adapted, remaining) =
            adapt_type_for_consumer("Optional[Path]", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "Optional[pathlib.Path]");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Optional");
    }

    #[test]
    fn test_adapt_short_to_dotted_word_boundary_safety() {
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));
        let (adapted, remaining) =
            adapt_type_for_consumer("PathLike", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "PathLike");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Path");
    }

    #[test]
    fn test_adapt_short_to_dotted_multiple_occurrences() {
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));
        let (adapted, remaining) =
            adapt_type_for_consumer("tuple[Path, Path]", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "tuple[pathlib.Path, pathlib.Path]");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_short_to_dotted_aliased_from_import() {
        let fixture_imports = vec![spec("P", "from pathlib import Path as P")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));
        let (adapted, remaining) = adapt_type_for_consumer("P", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "pathlib.Path");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_short_to_dotted_collections_abc() {
        let fixture_imports = vec![spec("Iterable", "from collections.abc import Iterable")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert(
            "collections.abc".to_string(),
            spec("collections.abc", "import collections.abc"),
        );
        let (adapted, remaining) =
            adapt_type_for_consumer("Iterable[str]", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "collections.abc.Iterable[str]");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_both_directions_in_one_call() {
        let fixture_imports = vec![
            spec("Sequence", "from typing import Sequence"),
            spec("pathlib", "import pathlib"),
        ];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));
        consumer_map.insert("typing".to_string(), spec("typing", "import typing"));
        let (adapted, remaining) =
            adapt_type_for_consumer("Sequence[pathlib.Path]", &fixture_imports, &consumer_map);
        assert_eq!(adapted, "typing.Sequence[Path]");
        assert!(
            remaining.is_empty(),
            "Both specs should be dropped: {:?}",
            remaining
        );
    }
}
