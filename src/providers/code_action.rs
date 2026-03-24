//! Code action provider for pytest fixtures.
//!
//! Provides several code-action kinds:
//!
//! 1. **`quickfix`** (diagnostic-driven) – when a diagnostic with code
//!    `"undeclared-fixture"` is present, offers to add the missing fixture as a
//!    typed parameter to the enclosing test/fixture function, together with any
//!    `import` statement needed to use the fixture's return type annotation in
//!    the consumer file.
//!
//! 2. **`source.pytest-lsp`** (cursor-based) – when the cursor is on a fixture
//!    parameter that already exists but lacks a type annotation, offers to
//!    insert `: ReturnType` (mirroring the inlay-hint text) and any necessary
//!    import statements.
//!
//! 3. **`source.fixAll.pytest-lsp`** (file-wide) – adds **all** missing type
//!    annotations and their imports for every unannotated fixture parameter in
//!    the file in a single action.
//!
//! Import edits are isort/ruff-aware on a **best-effort** basis:
//! - New imports are placed into the correct **isort group** (stdlib vs
//!   third-party), inserting blank-line separators between groups as needed.
//! - When the file already contains a single-line `from X import Y` for the
//!   same module, the new name is merged into that line (sorted alphabetically)
//!   instead of adding a duplicate line.
//! - Placement follows common isort conventions but does **not** read your
//!   project's `pyproject.toml` / `.isort.cfg` settings.  Run
//!   `ruff check --fix` or `isort` after applying these actions to bring
//!   imports into full conformance with your project's configuration.

use super::Backend;
use crate::fixtures::is_stdlib_module;
use crate::fixtures::string_utils::parameter_has_annotation;
use crate::fixtures::types::TypeImportSpec;
use rustpython_parser::ast::Mod;
use std::collections::{HashMap, HashSet};
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tracing::{info, warn};

// ── Custom code-action kinds ─────────────────────────────────────────────────

/// Prefix for all code-action titles so they are visually grouped in the UI.
const TITLE_PREFIX: &str = "pytest-lsp";

/// Add type annotation + import for the fixture at the cursor.
const SOURCE_PYTEST_LSP: CodeActionKind = CodeActionKind::new("source.pytest-lsp");

/// File-wide: add all missing fixture type annotations + imports.
const SOURCE_FIX_ALL_PYTEST_LSP: CodeActionKind = CodeActionKind::new("source.fixAll.pytest-lsp");

// ── Import classification (isort groups) ─────────────────────────────────────

/// Whether an import belongs to the stdlib group or the third-party group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportKind {
    Stdlib,
    ThirdParty,
}

/// A contiguous block of module-level import lines, separated from other
/// blocks by blank lines.
#[derive(Debug)]
struct ImportGroup {
    /// 0-based index of the first import line in this group.
    first_line: usize,
    /// 0-based index of the last import line in this group.
    last_line: usize,
    /// Classification based on the first import in the group.
    kind: ImportKind,
}

/// Extract the top-level package name from an import line.
///
/// - `"from typing import Any"`         → `"typing"`
/// - `"from collections.abc import X"`  → `"collections"`
/// - `"import pathlib"`                 → `"pathlib"`
/// - `"import os.path"`                 → `"os"`
fn extract_top_level_module(line: &str) -> &str {
    let trimmed = line.trim();
    let module_str = if let Some(rest) = trimmed.strip_prefix("from ") {
        rest.split_whitespace().next().unwrap_or("")
    } else if let Some(rest) = trimmed.strip_prefix("import ") {
        rest.split_whitespace().next().unwrap_or("")
    } else {
        return "";
    };
    // "collections.abc" → "collections", "os.path" → "os"
    module_str.split('.').next().unwrap_or("")
}

/// Classify an import statement string as stdlib or third-party.
fn classify_import_statement(statement: &str) -> ImportKind {
    let top = extract_top_level_module(statement);
    if is_stdlib_module(top) {
        ImportKind::Stdlib
    } else {
        ImportKind::ThirdParty
    }
}

/// Parse the top-of-file import layout into classified groups.
///
/// Scans from the top of the file, collecting contiguous runs of unindented
/// `import`/`from` statements into groups separated by blank lines.  Stops at
/// the first non-import, non-blank, non-comment line that appears **after** at
/// least one import has been seen (so that leading docstrings are skipped).
///
/// Each group is classified as [`ImportKind::Stdlib`] or
/// [`ImportKind::ThirdParty`] based on its first import line.
fn parse_import_groups(lines: &[&str]) -> Vec<ImportGroup> {
    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut current_start: Option<usize> = None;
    let mut current_last: usize = 0;
    let mut current_kind = ImportKind::ThirdParty;
    let mut seen_any_import = false;

    for (i, &line) in lines.iter().enumerate() {
        // Module-level (unindented) import.
        if line.starts_with("import ") || line.starts_with("from ") {
            seen_any_import = true;
            if current_start.is_none() {
                current_start = Some(i);
                let module = extract_top_level_module(line);
                current_kind = if is_stdlib_module(module) {
                    ImportKind::Stdlib
                } else {
                    ImportKind::ThirdParty
                };
            }
            current_last = i;
            continue;
        }

        let trimmed = line.trim();

        // Blank line or comment — close current group, keep scanning.
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

        // Non-import, non-blank line.
        if seen_any_import {
            // We've passed the import section — stop.
            if let Some(start) = current_start.take() {
                groups.push(ImportGroup {
                    first_line: start,
                    last_line: current_last,
                    kind: current_kind,
                });
            }
            break;
        }
        // Before any import: preamble (docstring, shebang value, etc.) — keep scanning.
    }

    // Close final group if file ends during imports.
    if let Some(start) = current_start {
        groups.push(ImportGroup {
            first_line: start,
            last_line: current_last,
            kind: current_kind,
        });
    }

    groups
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Check whether `action_kind` is permitted by the client's `only` filter.
///
/// Per the LSP specification the server should return an action whose kind `K`
/// matches an entry `E` in the `only` list when `K` starts with `E` (using a
/// dot-separated prefix match).  For example:
///
/// - `only: ["source"]` matches `source.fixAll.pytest-lsp`
/// - `only: ["source.fixAll"]` matches `source.fixAll.pytest-lsp`
/// - `only: ["quickfix"]` does **not** match `source.pytest-lsp`
///
/// When `only` is `None` every kind is accepted.
fn kind_requested(only: &Option<Vec<CodeActionKind>>, action_kind: &CodeActionKind) -> bool {
    let Some(ref kinds) = only else {
        return true; // no filter → everything accepted
    };
    let action_str = action_kind.as_str();
    kinds.iter().any(|k| {
        let k_str = k.as_str();
        // Exact match or the filter entry is a prefix with a dot boundary.
        action_str == k_str || action_str.starts_with(&format!("{}.", k_str))
    })
}

// ── Import-edit helpers (isort-aware) ────────────────────────────────────────

/// Parse a `from X import Y` style import statement.
///
/// Returns `Some((module, name))` for `from`-imports, `None` for bare
/// `import X` statements.
///
/// # Examples
/// - `"from typing import Any"` → `Some(("typing", "Any"))`
/// - `"from pathlib import Path as P"` → `Some(("pathlib", "Path as P"))`
/// - `"import pathlib"` → `None`
fn parse_from_import(statement: &str) -> Option<(&str, &str)> {
    let rest = statement.strip_prefix("from ")?;
    let (module, rest) = rest.split_once(" import ")?;
    let module = module.trim();
    let name = rest.trim();
    if module.is_empty() || name.is_empty() {
        return None;
    }
    Some((module, name))
}

/// Try to find an existing single-line `from <module> import ...` in the file.
///
/// Only matches **module-level** (unindented) imports — indented imports inside
/// function/class bodies are ignored.
///
/// Returns `Some((line_index_0based, vec_of_existing_name_parts))` on success.
/// Skips multi-line imports (containing `(` / `)`) and star imports (`*`).
fn find_matching_from_import_line<'a>(
    lines: &[&'a str],
    module: &str,
) -> Option<(usize, Vec<&'a str>)> {
    let prefix = format!("from {} import ", module);
    for (i, &line) in lines.iter().enumerate() {
        // Only match unindented (module-level) imports.
        if !line.starts_with(&prefix) {
            continue;
        }
        let trimmed = line.trim();
        // Skip multi-line and star imports.
        if trimmed.contains('(') || trimmed.contains(')') || trimmed.contains('*') {
            continue;
        }
        let names_part = &trimmed[prefix.len()..];
        let names: Vec<&str> = names_part.split(',').map(|s| s.trim()).collect();
        if names.iter().all(|n| !n.is_empty()) {
            return Some((i, names));
        }
    }
    None
}

/// Extract the sort key from an import name part.
///
/// For `"Path"` returns `"Path"`.
/// For `"Path as P"` returns `"Path"` (isort sorts by the original name).
fn import_sort_key(name: &str) -> &str {
    match name.find(" as ") {
        Some(pos) => name[..pos].trim(),
        None => name.trim(),
    }
}

/// Sort key for an entire import **line**, following isort/ruff conventions:
///
/// 1. Bare imports (`import X`) sort **before** from-imports (`from X import Y`).
/// 2. Within each category, sort alphabetically by the full dotted module path
///    (case-insensitive).
///
/// Returns `(category, lowercased_module)` where category `0` = bare, `1` = from.
fn import_line_sort_key(line: &str) -> (u8, String) {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("import ") {
        // "import pathlib as pl" → module "pathlib"
        let module = rest.split_whitespace().next().unwrap_or("");
        (0, module.to_lowercase())
    } else if let Some(rest) = trimmed.strip_prefix("from ") {
        // "from collections.abc import Sequence" → module "collections.abc"
        let module = rest.split(" import ").next().unwrap_or("").trim();
        (1, module.to_lowercase())
    } else {
        (2, String::new())
    }
}

/// Find the correct sorted insertion line for a new import within an existing
/// group, so that the result stays isort-sorted (bare before from, alphabetical
/// by module within each sub-category).
///
/// Returns the 0-based line number at which a point-insert should be placed.
/// When the new import sorts after every existing line in the group, the
/// position is `group.last_line + 1`.
fn find_sorted_insert_position(
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

/// Emit `TextEdit`s for a set of from-imports and bare imports, trying to
/// merge from-imports into existing lines before falling back to insertion.
///
/// When `group` is `Some`, new (non-merge) lines are inserted at the correct
/// isort-sorted position within the group.  When `None`, all new lines are
/// inserted at `fallback_insert_line`.
fn emit_kind_import_edits(
    lines: &[&str],
    from_imports: &HashMap<String, Vec<String>>,
    bare_imports: &[String],
    group: Option<&ImportGroup>,
    fallback_insert_line: u32,
    edits: &mut Vec<TextEdit>,
) {
    // ── Pass 1: merge from-imports into existing lines where possible ────
    let mut unmerged_from: Vec<(String, Vec<String>)> = Vec::new();

    let mut modules: Vec<&String> = from_imports.keys().collect();
    modules.sort();

    for module in modules {
        let new_names = &from_imports[module];

        if let Some((line_idx, existing_names)) = find_matching_from_import_line(lines, module) {
            // Merge into the existing line.
            let mut all_names: Vec<String> = existing_names.iter().map(|s| s.to_string()).collect();
            for n in new_names {
                if !all_names.iter().any(|existing| existing.trim() == n.trim()) {
                    all_names.push(n.clone());
                }
            }
            all_names.sort_by(|a, b| {
                import_sort_key(a)
                    .to_lowercase()
                    .cmp(&import_sort_key(b).to_lowercase())
            });
            all_names.dedup();

            let merged_line = format!("from {} import {}", module, all_names.join(", "));
            info!(
                "Merging import into existing line {}: {}",
                line_idx, merged_line
            );

            let original_line = lines[line_idx];
            let line_len = original_line.len() as u32;
            edits.push(TextEdit {
                range: Range {
                    start: Position {
                        line: line_idx as u32,
                        character: 0,
                    },
                    end: Position {
                        line: line_idx as u32,
                        character: line_len,
                    },
                },
                new_text: merged_line,
            });
        } else {
            unmerged_from.push((module.clone(), new_names.clone()));
        }
    }

    // ── Pass 2: collect all new lines, sort them, then insert ────────────
    //
    // We build a vec of (sort_key, formatted_text) so that when multiple
    // inserts land at the same original position they appear in the correct
    // isort order (bare before from, alphabetical by module).
    struct NewImport {
        sort_key: (u8, String),
        text: String,
    }

    let mut new_imports: Vec<NewImport> = Vec::new();

    // Bare imports.
    for stmt in bare_imports {
        new_imports.push(NewImport {
            sort_key: import_line_sort_key(stmt),
            text: stmt.clone(),
        });
    }

    // Unmerged from-imports.
    for (module, names) in &unmerged_from {
        let mut sorted_names = names.clone();
        sorted_names.sort_by(|a, b| {
            import_sort_key(a)
                .to_lowercase()
                .cmp(&import_sort_key(b).to_lowercase())
        });
        let text = format!("from {} import {}", module, sorted_names.join(", "));
        new_imports.push(NewImport {
            sort_key: import_line_sort_key(&text),
            text,
        });
    }

    // Sort so that array order matches isort order (matters when multiple
    // inserts share the same original line position).
    new_imports.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));

    for ni in &new_imports {
        let insert_line = match group {
            Some(g) => find_sorted_insert_position(lines, g, &ni.sort_key),
            None => fallback_insert_line,
        };
        info!("Adding new import line at {}: {}", insert_line, ni.text);
        edits.push(TextEdit {
            range: Backend::create_point_range(insert_line, 0),
            new_text: format!("{}\n", ni.text),
        });
    }
}

/// Replace all standalone occurrences of `old_name` with `new_name` in a type
/// string, respecting identifier boundaries.
///
/// A match is "standalone" when it is not preceded or followed by an
/// alphanumeric character or underscore, preventing partial matches like
/// `"PathLike"` when searching for `"Path"`.
fn replace_type_identifier(type_str: &str, old_name: &str, new_name: &str) -> String {
    let mut result = String::with_capacity(type_str.len() + new_name.len());
    let bytes = type_str.as_bytes();
    let old_bytes = old_name.as_bytes();
    let len = bytes.len();
    let old_len = old_bytes.len();
    let mut i = 0;

    while i < len {
        if i + old_len <= len && &bytes[i..i + old_len] == old_bytes {
            let left_ok = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric()
                    || bytes[i - 1] == b'_'
                    || bytes[i - 1] == b'.');
            let right_ok = i + old_len >= len
                || !(bytes[i + old_len].is_ascii_alphanumeric() || bytes[i + old_len] == b'_');

            if left_ok && right_ok {
                result.push_str(new_name);
                i += old_len;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

/// Find a bare-import entry in the consumer's import map for a given module.
///
/// Scans all specs looking for `import <module>` or `import <module> as <alias>`.
/// Returns the consumer's `check_name` for that module (which may be an alias).
fn find_consumer_bare_import<'a>(
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

/// Adapt a fixture's return-type annotation and import specs to the consumer
/// file's existing import context.
///
/// Two adaptations are performed:
///
/// 1. **Dotted → short** — when a fixture uses a bare `import` (e.g.
///    `import pathlib`) producing `pathlib.Path`, and the consumer already has
///    `from pathlib import Path`, the annotation is shortened to `Path` and the
///    bare-import spec is dropped.
///
/// 2. **Short → dotted** — when a fixture uses `from X import Y` producing the
///    short name `Y`, and the consumer already has `import X` (bare), the
///    annotation is lengthened to `X.Y` and the from-import spec is dropped,
///    respecting the consumer's import style.
///
/// Returns `(adapted_type_string, remaining_import_specs)`.
fn adapt_type_for_consumer(
    return_type: &str,
    fixture_imports: &[TypeImportSpec],
    consumer_import_map: &HashMap<String, TypeImportSpec>,
) -> (String, Vec<TypeImportSpec>) {
    let mut adapted = return_type.to_string();
    let mut remaining = Vec::new();

    for spec in fixture_imports {
        if spec.import_statement.starts_with("import ") {
            // ── Case 1: bare-import spec → try dotted-to-short rewrite ───
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

                // Guard against partial matches (e.g. `mypathlib.X` matching `pathlib.`)
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

                // Check the consumer's import map for this name.
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
                remaining.push(spec.clone());
            }
        } else if let Some((module, name_part)) = parse_from_import(&spec.import_statement) {
            // ── Case 2: from-import spec → try short-to-dotted rewrite ───
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
                let new_adapted = replace_type_identifier(&adapted, &spec.check_name, &dotted);
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

/// Build `TextEdit`s to add import statements, respecting isort-style grouping.
///
/// Specs whose `check_name` is already in `existing_imports` are skipped.
/// New imports are classified as stdlib or third-party and placed into the
/// correct import group (creating a new group with blank-line separators when
/// necessary).  Within a group, from-imports for the same module are merged
/// into a single line with names sorted alphabetically.
fn build_import_edits(
    lines: &[&str],
    specs: &[&TypeImportSpec],
    existing_imports: &HashSet<String>,
) -> Vec<TextEdit> {
    let groups = parse_import_groups(lines);

    // 1. Filter already-imported specs, deduplicate, and classify.
    let mut stdlib_from: HashMap<String, Vec<String>> = HashMap::new();
    let mut tp_from: HashMap<String, Vec<String>> = HashMap::new();
    let mut stdlib_bare: Vec<String> = Vec::new();
    let mut tp_bare: Vec<String> = Vec::new();
    let mut seen_names: HashSet<&str> = HashSet::new();

    for spec in specs {
        if existing_imports.contains(&spec.check_name) {
            info!("Import '{}' already present, skipping", spec.check_name);
            continue;
        }
        if !seen_names.insert(&spec.check_name) {
            continue;
        }

        let kind = classify_import_statement(&spec.import_statement);

        if let Some((module, name)) = parse_from_import(&spec.import_statement) {
            match kind {
                ImportKind::Stdlib => &mut stdlib_from,
                ImportKind::ThirdParty => &mut tp_from,
            }
            .entry(module.to_string())
            .or_default()
            .push(name.to_string());
        } else {
            match kind {
                ImportKind::Stdlib => &mut stdlib_bare,
                ImportKind::ThirdParty => &mut tp_bare,
            }
            .push(spec.import_statement.clone());
        }
    }

    let has_new_stdlib = !stdlib_from.is_empty() || !stdlib_bare.is_empty();
    let has_new_tp = !tp_from.is_empty() || !tp_bare.is_empty();

    if !has_new_stdlib && !has_new_tp {
        return vec![];
    }

    // 2. Locate existing groups (use *last* stdlib group for "insert after"
    //    so that `from __future__` groups are skipped over).
    let last_stdlib_group = groups.iter().rev().find(|g| g.kind == ImportKind::Stdlib);
    let first_tp_group = groups.iter().find(|g| g.kind == ImportKind::ThirdParty);
    let last_tp_group = groups
        .iter()
        .rev()
        .find(|g| g.kind == ImportKind::ThirdParty);

    // 3. Pre-compute whether each kind will actually *insert* new lines
    //    (as opposed to only merging into existing `from X import …` lines).
    //    Separators are only needed when new lines appear — merging into an
    //    existing line doesn't change the group layout.
    let will_insert_stdlib = stdlib_from
        .keys()
        .any(|m| find_matching_from_import_line(lines, m).is_none())
        || !stdlib_bare.is_empty();
    let will_insert_tp = tp_from
        .keys()
        .any(|m| find_matching_from_import_line(lines, m).is_none())
        || !tp_bare.is_empty();

    let mut edits: Vec<TextEdit> = Vec::new();

    // 4. Stdlib imports.
    if has_new_stdlib {
        let fallback_line = match (last_stdlib_group, first_tp_group) {
            (Some(sg), _) => (sg.last_line + 1) as u32,
            (None, Some(tpg)) => tpg.first_line as u32,
            (None, None) => 0,
        };

        emit_kind_import_edits(
            lines,
            &stdlib_from,
            &stdlib_bare,
            last_stdlib_group,
            fallback_line,
            &mut edits,
        );

        // Trailing separator when inserting a new stdlib group before an
        // *existing* third-party group.
        if will_insert_stdlib && last_stdlib_group.is_none() && first_tp_group.is_some() {
            edits.push(TextEdit {
                range: Backend::create_point_range(fallback_line, 0),
                new_text: "\n".to_string(),
            });
        }
    }

    // 5. Third-party imports.
    if has_new_tp {
        let fallback_line = match (last_tp_group, last_stdlib_group) {
            (Some(tpg), _) => (tpg.last_line + 1) as u32,
            (None, Some(sg)) => (sg.last_line + 1) as u32,
            (None, None) => 0,
        };

        // Leading separator when inserting a new third-party group after
        // an existing or newly-created stdlib group.
        if will_insert_tp
            && last_tp_group.is_none()
            && (last_stdlib_group.is_some() || will_insert_stdlib)
        {
            edits.push(TextEdit {
                range: Backend::create_point_range(fallback_line, 0),
                new_text: "\n".to_string(),
            });
        }

        emit_kind_import_edits(
            lines,
            &tp_from,
            &tp_bare,
            last_tp_group,
            fallback_line,
            &mut edits,
        );
    }

    edits
}

// ── Main handler ─────────────────────────────────────────────────────────────

impl Backend {
    /// Handle `textDocument/codeAction` request.
    pub async fn handle_code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let range = params.range;
        let context = params.context;

        info!(
            "code_action request: uri={:?}, diagnostics={}, only={:?}",
            uri,
            context.diagnostics.len(),
            context.only
        );

        let Some(file_path) = self.uri_to_path(&uri) else {
            info!("Returning None for code_action request: could not resolve URI");
            return Ok(None);
        };

        // Pre-fetch the file content once — we need it both for parameter
        // insertion and for finding the import-insertion line.
        let Some(content) = self.fixture_db.get_file_content(&file_path) else {
            info!("Returning None: file content not in cache");
            return Ok(None);
        };
        let lines: Vec<&str> = content.lines().collect();

        // Snapshot the names already imported in the test file so we can decide
        // which import statements need to be added.
        let existing_imports = self
            .fixture_db
            .imports
            .get(&file_path)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        // Build a name→TypeImportSpec map for the consumer (test) file so we
        // can detect when the file already imports a name that appears in a
        // dotted form in a fixture's return type (e.g. `pathlib.Path` → `Path`).
        let consumer_import_map: HashMap<String, TypeImportSpec> =
            match self.fixture_db.get_parsed_ast(&file_path, &content) {
                Some(ast) => {
                    if let Mod::Module(module) = ast.as_ref() {
                        self.fixture_db
                            .build_name_to_import_map(&module.body, &file_path)
                    } else {
                        HashMap::new()
                    }
                }
                None => HashMap::new(),
            };

        let mut actions: Vec<CodeActionOrCommand> = Vec::new();

        // ════════════════════════════════════════════════════════════════════
        // Pass 1: diagnostic-driven actions (undeclared fixtures) — QUICKFIX
        // ════════════════════════════════════════════════════════════════════

        if kind_requested(&context.only, &CodeActionKind::QUICKFIX) {
            let undeclared = self.fixture_db.get_undeclared_fixtures(&file_path);
            info!("Found {} undeclared fixtures in file", undeclared.len());

            for diagnostic in &context.diagnostics {
                info!(
                    "Processing diagnostic: code={:?}, range={:?}",
                    diagnostic.code, diagnostic.range
                );

                let Some(NumberOrString::String(code)) = &diagnostic.code else {
                    continue;
                };
                if code != "undeclared-fixture" {
                    continue;
                }

                let diag_line = Self::lsp_line_to_internal(diagnostic.range.start.line);
                let diag_char = diagnostic.range.start.character as usize;

                info!(
                    "Looking for undeclared fixture at line={}, char={}",
                    diag_line, diag_char
                );

                let Some(fixture) = undeclared
                    .iter()
                    .find(|f| f.line == diag_line && f.start_char == diag_char)
                else {
                    continue;
                };

                info!("Found matching fixture: {}", fixture.name);

                // ── Resolve the fixture definition to obtain return-type info ─
                let fixture_def = self
                    .fixture_db
                    .resolve_fixture_for_file(&file_path, &fixture.name);

                let (type_suffix, return_type_imports) = match &fixture_def {
                    Some(def) => {
                        if let Some(rt) = &def.return_type {
                            let (adapted, remaining) = adapt_type_for_consumer(
                                rt,
                                &def.return_type_imports,
                                &consumer_import_map,
                            );
                            (format!(": {}", adapted), remaining)
                        } else {
                            (String::new(), vec![])
                        }
                    }
                    None => (String::new(), vec![]),
                };

                // ── Build the parameter insertion TextEdit ───────────────────
                let function_line = Self::internal_line_to_lsp(fixture.function_line);

                let Some(func_line_content) = lines.get(function_line as usize) else {
                    warn!(
                        "Function line {} is out of range in {:?}",
                        function_line, file_path
                    );
                    continue;
                };

                // Locate the closing `):` of the function signature.
                let Some(paren_pos) = func_line_content.find("):") else {
                    continue;
                };

                if !func_line_content[..paren_pos].contains('(') {
                    continue;
                }

                let param_start = match func_line_content.find('(') {
                    Some(pos) => pos + 1,
                    None => {
                        warn!(
                            "Invalid function signature at {:?}:{}",
                            file_path, function_line
                        );
                        continue;
                    }
                };

                let params_section = &func_line_content[param_start..paren_pos];
                let has_params = !params_section.trim().is_empty();

                let (insert_line, insert_char) = if has_params {
                    (function_line, paren_pos as u32)
                } else {
                    (function_line, param_start as u32)
                };

                let param_text = if has_params {
                    format!(", {}{}", fixture.name, type_suffix)
                } else {
                    format!("{}{}", fixture.name, type_suffix)
                };

                // ── Build import + parameter edits ───────────────────────────
                let spec_refs: Vec<&TypeImportSpec> = return_type_imports.iter().collect();
                let mut all_edits = build_import_edits(&lines, &spec_refs, &existing_imports);

                // Parameter insertion goes last so that line numbers for earlier
                // import edits remain valid (imports are above the function).
                all_edits.push(TextEdit {
                    range: Self::create_point_range(insert_line, insert_char),
                    new_text: param_text,
                });

                let edit = WorkspaceEdit {
                    changes: Some(vec![(uri.clone(), all_edits)].into_iter().collect()),
                    document_changes: None,
                    change_annotations: None,
                };

                // Use the adapted type in the title (e.g. "Path" not "pathlib.Path").
                let display_type = type_suffix.strip_prefix(": ").unwrap_or("");
                let title = if !display_type.is_empty() {
                    format!(
                        "{}: Add '{}' fixture parameter ({})",
                        TITLE_PREFIX, fixture.name, display_type
                    )
                } else {
                    format!("{}: Add '{}' fixture parameter", TITLE_PREFIX, fixture.name)
                };

                let action = CodeAction {
                    title,
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(edit),
                    command: None,
                    is_preferred: Some(true),
                    disabled: None,
                    data: None,
                };

                info!("Created code action: {}", action.title);
                actions.push(CodeActionOrCommand::CodeAction(action));
            }
        }

        // ════════════════════════════════════════════════════════════════════
        // Pass 2 & 3 share the fixture map — build it lazily.
        // ════════════════════════════════════════════════════════════════════

        let want_source = kind_requested(&context.only, &SOURCE_PYTEST_LSP);
        let want_fix_all = kind_requested(&context.only, &SOURCE_FIX_ALL_PYTEST_LSP);

        let need_fixture_map = want_source || want_fix_all;

        if need_fixture_map {
            if let Some(ref usages) = self.fixture_db.usages.get(&file_path) {
                let available = self.fixture_db.get_available_fixtures(&file_path);
                let fixture_map: std::collections::HashMap<&str, _> = available
                    .iter()
                    .filter_map(|def| def.return_type.as_ref().map(|_rt| (def.name.as_str(), def)))
                    .collect();

                if !fixture_map.is_empty() {
                    // ════════════════════════════════════════════════════════
                    // Pass 2: cursor-based single-fixture annotation
                    //   source.pytest-lsp
                    // ════════════════════════════════════════════════════════

                    if want_source {
                        let cursor_line_internal = Self::lsp_line_to_internal(range.start.line);

                        for usage in usages.iter() {
                            if usage.line != cursor_line_internal {
                                continue;
                            }

                            let cursor_char = range.start.character as usize;
                            if cursor_char < usage.start_char || cursor_char > usage.end_char {
                                continue;
                            }

                            if parameter_has_annotation(&lines, usage.line, usage.end_char) {
                                continue;
                            }

                            let Some(def) = fixture_map.get(usage.name.as_str()) else {
                                continue;
                            };

                            let return_type = match &def.return_type {
                                Some(rt) => rt,
                                None => continue,
                            };

                            // Adapt dotted types to consumer's import context.
                            let (adapted_type, adapted_imports) = adapt_type_for_consumer(
                                return_type,
                                &def.return_type_imports,
                                &consumer_import_map,
                            );

                            info!(
                                "Cursor-based annotation action for '{}': {}",
                                usage.name, adapted_type
                            );

                            // ── Build TextEdits ──────────────────────────────
                            let spec_refs: Vec<&TypeImportSpec> = adapted_imports.iter().collect();
                            let mut all_edits =
                                build_import_edits(&lines, &spec_refs, &existing_imports);

                            let lsp_line = Self::internal_line_to_lsp(usage.line);
                            all_edits.push(TextEdit {
                                range: Self::create_point_range(lsp_line, usage.end_char as u32),
                                new_text: format!(": {}", adapted_type),
                            });

                            let ws_edit = WorkspaceEdit {
                                changes: Some(vec![(uri.clone(), all_edits)].into_iter().collect()),
                                document_changes: None,
                                change_annotations: None,
                            };

                            let title = format!(
                                "{}: Add type annotation for fixture '{}'",
                                TITLE_PREFIX, usage.name
                            );

                            let action = CodeAction {
                                title: title.clone(),
                                kind: Some(SOURCE_PYTEST_LSP),
                                diagnostics: None,
                                edit: Some(ws_edit),
                                command: None,
                                is_preferred: Some(true),
                                disabled: None,
                                data: None,
                            };
                            info!("Created source.pytest-lsp action: {}", title);
                            actions.push(CodeActionOrCommand::CodeAction(action));
                        }
                    }

                    // ════════════════════════════════════════════════════════
                    // Pass 3: file-wide fix-all
                    //   source.fixAll.pytest-lsp
                    // ════════════════════════════════════════════════════════

                    if want_fix_all {
                        // Collect all import specs and annotation edits.
                        let mut all_adapted_imports: Vec<TypeImportSpec> = Vec::new();
                        let mut annotation_edits: Vec<TextEdit> = Vec::new();
                        let mut annotated_count: usize = 0;

                        for usage in usages.iter() {
                            if parameter_has_annotation(&lines, usage.line, usage.end_char) {
                                continue;
                            }

                            let Some(def) = fixture_map.get(usage.name.as_str()) else {
                                continue;
                            };

                            let return_type = match &def.return_type {
                                Some(rt) => rt,
                                None => continue,
                            };

                            // Adapt dotted types to consumer's import context.
                            let (adapted_type, adapted_imports) = adapt_type_for_consumer(
                                return_type,
                                &def.return_type_imports,
                                &consumer_import_map,
                            );

                            // Collect import specs (build_import_edits handles
                            // deduplication internally).
                            all_adapted_imports.extend(adapted_imports);

                            // Annotation edit.
                            let lsp_line = Self::internal_line_to_lsp(usage.line);
                            annotation_edits.push(TextEdit {
                                range: Self::create_point_range(lsp_line, usage.end_char as u32),
                                new_text: format!(": {}", adapted_type),
                            });

                            annotated_count += 1;
                        }

                        if !annotation_edits.is_empty() {
                            let spec_refs: Vec<&TypeImportSpec> =
                                all_adapted_imports.iter().collect();
                            let mut all_edits =
                                build_import_edits(&lines, &spec_refs, &existing_imports);
                            all_edits.extend(annotation_edits);

                            let ws_edit = WorkspaceEdit {
                                changes: Some(vec![(uri.clone(), all_edits)].into_iter().collect()),
                                document_changes: None,
                                change_annotations: None,
                            };

                            let title = format!(
                                "{}: Add all fixture type annotations ({} fixture{})",
                                TITLE_PREFIX,
                                annotated_count,
                                if annotated_count == 1 { "" } else { "s" }
                            );

                            let action = CodeAction {
                                title: title.clone(),
                                kind: Some(SOURCE_FIX_ALL_PYTEST_LSP),
                                diagnostics: None,
                                edit: Some(ws_edit),
                                command: None,
                                is_preferred: Some(false),
                                disabled: None,
                                data: None,
                            };

                            info!("Created source.fixAll.pytest-lsp action: {}", title);
                            actions.push(CodeActionOrCommand::CodeAction(action));
                        }
                    }
                }
            }
        }

        // ════════════════════════════════════════════════════════════════════

        if !actions.is_empty() {
            info!("Returning {} code actions", actions.len());
            return Ok(Some(actions));
        }

        info!("Returning None for code_action request");
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_top_level_module tests ────────────────────────────────────

    #[test]
    fn test_extract_top_level_module_from_import() {
        assert_eq!(extract_top_level_module("from typing import Any"), "typing");
    }

    #[test]
    fn test_extract_top_level_module_dotted() {
        assert_eq!(
            extract_top_level_module("from collections.abc import Sequence"),
            "collections"
        );
    }

    #[test]
    fn test_extract_top_level_module_bare() {
        assert_eq!(extract_top_level_module("import pathlib"), "pathlib");
    }

    #[test]
    fn test_extract_top_level_module_bare_dotted() {
        assert_eq!(extract_top_level_module("import os.path"), "os");
    }

    #[test]
    fn test_extract_top_level_module_bare_alias() {
        assert_eq!(extract_top_level_module("import pathlib as pl"), "pathlib");
    }

    #[test]
    fn test_extract_top_level_module_non_import() {
        assert_eq!(extract_top_level_module("x = 1"), "");
    }

    // ── classify_import_statement tests ──────────────────────────────────

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

    // ── parse_import_groups tests ────────────────────────────────────────

    #[test]
    fn test_parse_groups_stdlib_and_third_party() {
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.framework import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let groups = parse_import_groups(&lines);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].first_line, 0);
        assert_eq!(groups[0].last_line, 0);
        assert_eq!(groups[0].kind, ImportKind::Stdlib);
        assert_eq!(groups[1].first_line, 2);
        assert_eq!(groups[1].last_line, 3);
        assert_eq!(groups[1].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_single_third_party() {
        let lines = vec!["import pytest", "", "def test(): pass"];
        let groups = parse_import_groups(&lines);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, ImportKind::ThirdParty);
        assert_eq!(groups[0].first_line, 0);
        assert_eq!(groups[0].last_line, 0);
    }

    #[test]
    fn test_parse_groups_no_imports() {
        let lines = vec!["def test(): pass"];
        let groups = parse_import_groups(&lines);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_parse_groups_empty_file() {
        let groups = parse_import_groups(&[]);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_parse_groups_with_docstring_preamble() {
        let lines = vec![
            r#""""Module docstring.""""#,
            "",
            "import pytest",
            "from pathlib import Path",
            "",
            "def test(): pass",
        ];
        let groups = parse_import_groups(&lines);
        // pytest is third-party, pathlib is stdlib — but they're in the same
        // contiguous block, classified by the first import (pytest → third-party).
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].first_line, 2);
        assert_eq!(groups[0].last_line, 3);
        assert_eq!(groups[0].kind, ImportKind::ThirdParty);
    }

    #[test]
    fn test_parse_groups_ignores_indented_imports() {
        let lines = vec![
            "import pytest",
            "",
            "def test():",
            "    from .utils import helper",
            "    import os",
        ];
        let groups = parse_import_groups(&lines);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].first_line, 0);
        assert_eq!(groups[0].last_line, 0);
    }

    #[test]
    fn test_parse_groups_future_then_stdlib_then_third_party() {
        let lines = vec![
            "from __future__ import annotations",
            "",
            "import os",
            "import time",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ];
        let groups = parse_import_groups(&lines);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].kind, ImportKind::Stdlib); // __future__ is stdlib
        assert_eq!(groups[1].kind, ImportKind::Stdlib); // os, time
        assert_eq!(groups[2].kind, ImportKind::ThirdParty); // pytest
    }

    #[test]
    fn test_parse_groups_with_comments_between() {
        let lines = vec![
            "import os",
            "# stdlib above, third-party below",
            "import pytest",
            "",
            "def test(): pass",
        ];
        let groups = parse_import_groups(&lines);
        // Comment closes the first group, starts a new one.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, ImportKind::Stdlib);
        assert_eq!(groups[0].last_line, 0);
        assert_eq!(groups[1].kind, ImportKind::ThirdParty);
        assert_eq!(groups[1].first_line, 2);
    }

    // ── kind_requested tests ─────────────────────────────────────────────

    #[test]
    fn test_kind_requested_no_filter_accepts_everything() {
        assert!(kind_requested(&None, &CodeActionKind::QUICKFIX));
        assert!(kind_requested(&None, &SOURCE_PYTEST_LSP));
        assert!(kind_requested(&None, &SOURCE_FIX_ALL_PYTEST_LSP));
    }

    #[test]
    fn test_kind_requested_exact_match() {
        let only = Some(vec![CodeActionKind::QUICKFIX]);
        assert!(kind_requested(&only, &CodeActionKind::QUICKFIX));
        assert!(!kind_requested(&only, &SOURCE_PYTEST_LSP));
    }

    #[test]
    fn test_kind_requested_parent_source_matches_children() {
        let only = Some(vec![CodeActionKind::SOURCE]);
        assert!(kind_requested(&only, &SOURCE_PYTEST_LSP));
        assert!(kind_requested(&only, &SOURCE_FIX_ALL_PYTEST_LSP));
        assert!(!kind_requested(&only, &CodeActionKind::QUICKFIX));
    }

    #[test]
    fn test_kind_requested_parent_source_fix_all_matches_child() {
        let only = Some(vec![CodeActionKind::SOURCE_FIX_ALL]);
        assert!(kind_requested(&only, &SOURCE_FIX_ALL_PYTEST_LSP));
        assert!(!kind_requested(&only, &SOURCE_PYTEST_LSP));
    }

    #[test]
    fn test_kind_requested_specific_child_does_not_match_sibling() {
        let only = Some(vec![SOURCE_PYTEST_LSP]);
        assert!(kind_requested(&only, &SOURCE_PYTEST_LSP));
        assert!(!kind_requested(&only, &SOURCE_FIX_ALL_PYTEST_LSP));
    }

    #[test]
    fn test_kind_requested_multiple_filters() {
        let only = Some(vec![
            CodeActionKind::QUICKFIX,
            CodeActionKind::SOURCE_FIX_ALL,
        ]);
        assert!(kind_requested(&only, &CodeActionKind::QUICKFIX));
        assert!(kind_requested(&only, &SOURCE_FIX_ALL_PYTEST_LSP));
        assert!(!kind_requested(&only, &SOURCE_PYTEST_LSP));
    }

    #[test]
    fn test_kind_requested_quickfix_only_rejects_source() {
        let only = Some(vec![CodeActionKind::QUICKFIX]);
        assert!(!kind_requested(&only, &SOURCE_PYTEST_LSP));
        assert!(!kind_requested(&only, &SOURCE_FIX_ALL_PYTEST_LSP));
    }

    // ── parse_from_import tests ──────────────────────────────────────────

    #[test]
    fn test_parse_from_import_simple() {
        assert_eq!(
            parse_from_import("from typing import Any"),
            Some(("typing", "Any"))
        );
    }

    #[test]
    fn test_parse_from_import_with_alias() {
        assert_eq!(
            parse_from_import("from pathlib import Path as P"),
            Some(("pathlib", "Path as P"))
        );
    }

    #[test]
    fn test_parse_from_import_deep_module() {
        assert_eq!(
            parse_from_import("from collections.abc import Sequence"),
            Some(("collections.abc", "Sequence"))
        );
    }

    #[test]
    fn test_parse_from_import_bare_import() {
        assert_eq!(parse_from_import("import pathlib"), None);
    }

    #[test]
    fn test_parse_from_import_bare_import_with_alias() {
        assert_eq!(parse_from_import("import pathlib as pl"), None);
    }

    // ── find_matching_from_import_line tests ─────────────────────────────

    #[test]
    fn test_find_matching_line_found() {
        let lines = vec![
            "import pytest",
            "from typing import Optional",
            "",
            "def test(): pass",
        ];
        let result = find_matching_from_import_line(&lines, "typing");
        assert_eq!(result, Some((1, vec!["Optional"])));
    }

    #[test]
    fn test_find_matching_line_multiple_names() {
        let lines = vec![
            "from typing import Any, Optional, Union",
            "from pathlib import Path",
        ];
        let result = find_matching_from_import_line(&lines, "typing");
        assert_eq!(result, Some((0, vec!["Any", "Optional", "Union"])));
    }

    #[test]
    fn test_find_matching_line_not_found() {
        let lines = vec!["import pytest", "from pathlib import Path"];
        assert_eq!(find_matching_from_import_line(&lines, "typing"), None);
    }

    #[test]
    fn test_find_matching_line_skips_multiline() {
        let lines = vec!["from typing import (", "    Any,", "    Optional,", ")"];
        assert_eq!(find_matching_from_import_line(&lines, "typing"), None);
    }

    #[test]
    fn test_find_matching_line_skips_star() {
        let lines = vec!["from typing import *"];
        assert_eq!(find_matching_from_import_line(&lines, "typing"), None);
    }

    #[test]
    fn test_find_matching_line_ignores_indented() {
        let lines = vec![
            "import pytest",
            "",
            "def test():",
            "    from typing import Any",
        ];
        assert_eq!(find_matching_from_import_line(&lines, "typing"), None);
    }

    // ── import_sort_key tests ────────────────────────────────────────────

    #[test]
    fn test_import_sort_key_plain() {
        assert_eq!(import_sort_key("Path"), "Path");
    }

    #[test]
    fn test_import_sort_key_alias() {
        assert_eq!(import_sort_key("Path as P"), "Path");
    }

    // ── build_import_edits tests ─────────────────────────────────────────

    #[test]
    fn test_build_import_edits_merge_into_existing() {
        // Existing `from typing import Optional` should get `Any` merged in.
        let lines = vec![
            "import pytest",
            "from typing import Optional",
            "",
            "def test(): pass",
        ];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.start.character, 0);
        assert_eq!(edits[0].range.end.line, 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional");
    }

    #[test]
    fn test_build_import_edits_skips_already_imported() {
        let lines = vec!["from typing import Any"];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let mut existing: HashSet<String> = HashSet::new();
        existing.insert("Any".to_string());
        let edits = build_import_edits(&lines, &[&spec], &existing);

        assert!(edits.is_empty());
    }

    #[test]
    fn test_build_import_edits_merge_multiple_into_existing() {
        let lines = vec!["from typing import Union", "", "def test(): pass"];
        let spec1 = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec2 = TypeImportSpec {
            check_name: "Optional".to_string(),
            import_statement: "from typing import Optional".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec1, &spec2], &existing);

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional, Union");
    }

    #[test]
    fn test_build_import_edits_merge_preserves_alias() {
        let lines = vec!["from pathlib import Path as P", "", "def test(): pass"];
        let spec = TypeImportSpec {
            check_name: "PurePath".to_string(),
            import_statement: "from pathlib import PurePath".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "from pathlib import Path as P, PurePath");
    }

    #[test]
    fn test_build_import_edits_deduplicates_specs() {
        let lines = vec!["import pytest", "", "def test(): pass"];
        let spec1 = TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        };
        let spec2 = TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec1, &spec2], &existing);

        // Insertion + separator (stdlib before existing third-party group).
        let import_edits: Vec<_> = edits
            .iter()
            .filter(|e| e.new_text.contains("Path"))
            .collect();
        assert_eq!(import_edits.len(), 1);
        assert_eq!(import_edits[0].new_text, "from pathlib import Path\n");
    }

    // ── isort-group-aware insertion tests ─────────────────────────────────

    #[test]
    fn test_stdlib_import_into_existing_stdlib_group() {
        // File has stdlib group (import time) and third-party group (import pytest).
        // Adding `from typing import Any` should go into the stdlib group.
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.framework import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // from-import sorts after the bare `import time`, so insert at line 1.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_stdlib_import_before_third_party_when_no_stdlib_group() {
        // File has only third-party imports. Stdlib import should go before them
        // with a blank-line separator.
        let lines = vec![
            "import pytest",
            "from vcc.framework import fixture",
            "",
            "def test(): pass",
        ];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // Insertion at line 0 (before third-party) + separator.
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
        assert_eq!(edits[0].range.start.line, 0);
        assert_eq!(edits[1].new_text, "\n");
        assert_eq!(edits[1].range.start.line, 0);
    }

    #[test]
    fn test_third_party_import_after_stdlib_when_no_tp_group() {
        // File has only stdlib imports. Third-party import should go after them
        // with a blank-line separator.
        let lines = vec!["import os", "import time", "", "def test(): pass"];
        let spec = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // Separator + insertion after stdlib group (line 1), at line 2.
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new_text, "\n");
        assert_eq!(edits[0].range.start.line, 2);
        assert_eq!(edits[1].new_text, "from flask.testing import FlaskClient\n");
        assert_eq!(edits[1].range.start.line, 2);
    }

    #[test]
    fn test_third_party_import_into_existing_tp_group() {
        // File has both groups. Third-party import goes into the tp group, sorted.
        let lines = vec!["import time", "", "import pytest", "", "def test(): pass"];
        let spec = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // from-import sorts after bare `import pytest`, so insert at line 3.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 3);
        assert_eq!(edits[0].new_text, "from flask.testing import FlaskClient\n");
    }

    #[test]
    fn test_no_imports_at_all() {
        let lines = vec!["def test(): pass"];
        let spec = TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // Insert at line 0 (no groups exist).
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 0);
        assert_eq!(edits[0].new_text, "from pathlib import Path\n");
    }

    #[test]
    fn test_both_stdlib_and_tp_imports_no_existing_groups() {
        // No existing imports at all. Adding both stdlib and third-party should
        // produce stdlib first, separator, then third-party.
        let lines = vec!["def test(): pass"];
        let spec_stdlib = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec_tp = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec_stdlib, &spec_tp], &existing);

        // stdlib insertion, then tp separator + tp insertion (all at line 0).
        // Array order: [stdlib_edit, tp_separator, tp_edit]
        assert_eq!(edits.len(), 3);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
        assert_eq!(edits[1].new_text, "\n"); // separator
        assert_eq!(edits[2].new_text, "from flask.testing import FlaskClient\n");
    }

    #[test]
    fn test_bare_stdlib_import_sorted_within_group() {
        // `import pathlib` should sort between `import os` and `import time`.
        let lines = vec![
            "import os",
            "import time",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ];
        let spec = TypeImportSpec {
            check_name: "pathlib".to_string(),
            import_statement: "import pathlib".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // `pathlib` sorts after `os` (line 0) but before `time` (line 1).
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "import pathlib\n");
    }

    #[test]
    fn test_from_import_sorts_after_bare_imports_in_group() {
        // A from-import should go after all bare imports within the same group.
        let lines = vec!["import os", "import time", "", "def test(): pass"];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // from-import sorts after all bare imports, so line 2 (after `import time`).
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 2);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_mixed_stdlib_from_imports_grouped() {
        // Adding two stdlib from-imports for the same module should combine them.
        let lines = vec!["import time", "", "import pytest", "", "def test(): pass"];
        let spec1 = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec2 = TypeImportSpec {
            check_name: "Optional".to_string(),
            import_statement: "from typing import Optional".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec1, &spec2], &existing);

        // Combined from-import sorts after `import time` (line 0), at line 1.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional\n");
    }

    #[test]
    fn test_tp_from_import_sorted_before_existing() {
        // `from vcc import conx_canoe` should sort before `from vcc.conxtfw...`.
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.conxtfw.framework.pytest.fixtures.component import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let spec = TypeImportSpec {
            check_name: "conx_canoe".to_string(),
            import_statement: "from vcc import conx_canoe".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // "vcc" < "vcc.conxtfw...", so insert before line 3.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 3);
        assert_eq!(edits[0].new_text, "from vcc import conx_canoe\n");
    }

    #[test]
    fn test_user_scenario_stdlib_into_correct_group() {
        // This is the exact scenario from the bug report:
        // File has `import time` (stdlib) + `import pytest` + `from vcc...` (third-party).
        // Adding `from typing import Any` should go into the stdlib group.
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.conxtfw.framework.pytest.fixtures.component import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // from-import sorts after bare `import time`, insert at line 1.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.start.character, 0);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_user_scenario_fix_all_multi_import() {
        // Full fixAll scenario: adding stdlib (pathlib, typing) and tp (vcc)
        // imports into a file that already has both groups.
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.conxtfw.framework.pytest.fixtures.component import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let spec_typing = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec_pathlib = TypeImportSpec {
            check_name: "pathlib".to_string(),
            import_statement: "import pathlib".to_string(),
        };
        let spec_vcc = TypeImportSpec {
            check_name: "conx_canoe".to_string(),
            import_statement: "from vcc import conx_canoe".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits =
            build_import_edits(&lines, &[&spec_typing, &spec_pathlib, &spec_vcc], &existing);

        // Three insertion edits, no separators (groups already exist):
        //   stdlib: `import pathlib` before `import time` (line 0), key (0,"pathlib") < (0,"time")
        //   stdlib: `from typing import Any` after `import time` (line 1), key (1,"typing") > (0,"time")
        //   tp:     `from vcc import conx_canoe` before existing from-import (line 3),
        //           key (1,"vcc") < (1,"vcc.conxtfw...")
        assert_eq!(edits.len(), 3);

        let pathlib_edit = edits
            .iter()
            .find(|e| e.new_text.contains("pathlib"))
            .unwrap();
        assert_eq!(pathlib_edit.range.start.line, 0);
        assert_eq!(pathlib_edit.new_text, "import pathlib\n");

        let typing_edit = edits
            .iter()
            .find(|e| e.new_text.contains("typing"))
            .unwrap();
        assert_eq!(typing_edit.range.start.line, 1);
        assert_eq!(typing_edit.new_text, "from typing import Any\n");

        let vcc_edit = edits
            .iter()
            .find(|e| e.new_text.contains("conx_canoe"))
            .unwrap();
        assert_eq!(vcc_edit.range.start.line, 3);
        assert_eq!(vcc_edit.new_text, "from vcc import conx_canoe\n");
    }

    #[test]
    fn test_future_import_skipped_for_stdlib_insertion() {
        // __future__ is its own group. Regular stdlib should go into the second
        // stdlib group (after os/time), not after __future__.
        let lines = vec![
            "from __future__ import annotations",
            "",
            "import os",
            "import time",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ];
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec], &existing);

        // from-import sorts after bare imports in the os/time group (lines 2-3),
        // so insert at line 4.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 4);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_different_modules_stdlib_and_tp() {
        // Adding one stdlib and one third-party import to a file that has both groups.
        let lines = vec!["import os", "", "import pytest", "", "def test(): pass"];
        let spec_stdlib = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec_tp = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&lines, &[&spec_stdlib, &spec_tp], &existing);

        // stdlib from-import after `import os` (line 1), tp from-import after `import pytest` (line 3).
        assert_eq!(edits.len(), 2);
        let stdlib_edit = edits
            .iter()
            .find(|e| e.new_text.contains("typing"))
            .unwrap();
        assert_eq!(stdlib_edit.range.start.line, 1);
        assert_eq!(stdlib_edit.new_text, "from typing import Any\n");
        let tp_edit = edits.iter().find(|e| e.new_text.contains("flask")).unwrap();
        assert_eq!(tp_edit.range.start.line, 3);
        assert_eq!(tp_edit.new_text, "from flask.testing import FlaskClient\n");
    }

    // ── import_line_sort_key tests ───────────────────────────────────────

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

    // ── find_sorted_insert_position tests ────────────────────────────────

    #[test]
    fn test_sorted_position_bare_before_existing_bare() {
        let lines = vec!["import os", "import time"];
        let group = ImportGroup {
            first_line: 0,
            last_line: 1,
            kind: ImportKind::Stdlib,
        };
        // `import pathlib` sorts between os and time.
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
        // from-import sorts after all bare imports.
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
        // `import os` sorts before `import time`.
        let key = import_line_sort_key("import os");
        assert_eq!(find_sorted_insert_position(&lines, &group, &key), 0);
    }

    // ── adapt_type_for_consumer tests ────────────────────────────────────

    /// Helper: build a TypeImportSpec quickly.
    fn spec(check_name: &str, import_statement: &str) -> TypeImportSpec {
        TypeImportSpec {
            check_name: check_name.to_string(),
            import_statement: import_statement.to_string(),
        }
    }

    #[test]
    fn test_adapt_dotted_to_short_when_consumer_has_from_import() {
        // Fixture: `import pathlib` → type `pathlib.Path`
        // Consumer: `from pathlib import Path`
        // Expected: type rewritten to `Path`, bare-import spec dropped.
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
        // Fixture: `import pathlib` → type `pathlib.Path`
        // Consumer: no pathlib imports at all
        // Expected: type unchanged, import spec kept.
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
        // Fixture: `import pathlib` → type `pathlib.Path`
        // Consumer: `from mylib import Path` (different module!)
        // Expected: type unchanged, import spec kept.
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
        // `from pathlib import Path` specs already use the short name — no rewrite.
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let consumer_map = HashMap::new();

        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);

        assert_eq!(adapted, "Path");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Path");
    }

    #[test]
    fn test_adapt_complex_generic_with_dotted_and_from() {
        // Fixture: `import pathlib` + `from typing import Optional`
        // Type: `Optional[pathlib.Path]`
        // Consumer: `from pathlib import Path` + `from typing import Optional`
        // Expected: `Optional[Path]`, only the bare-import spec dropped,
        //           the `from typing import Optional` spec passes through.
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
        // Only the `from typing import Optional` spec should remain.
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Optional");
    }

    #[test]
    fn test_adapt_multiple_dotted_refs_same_module() {
        // Type: `tuple[pathlib.Path, pathlib.PurePath]`
        // Consumer has both `from pathlib import Path` and `from pathlib import PurePath`.
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
        // Type: `tuple[pathlib.Path, pathlib.PurePath]`
        // Consumer only has `from pathlib import Path` — `PurePath` is missing.
        // Expected: no rewrite (all-or-nothing for a given import spec).
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
        // Fixture: `import pathlib as pl` → type `pl.Path`
        // Consumer: `from pathlib import Path`
        // Expected: `Path`, spec dropped.
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
        // Type contains `mypathlib.Path` — must NOT match the `pathlib.` prefix.
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("Path".to_string(), spec("Path", "from pathlib import Path"));

        let (adapted, remaining) =
            adapt_type_for_consumer("mypathlib.Path", &fixture_imports, &consumer_map);

        // `mypathlib.Path` should NOT be rewritten — `mypathlib` != `pathlib`.
        assert_eq!(adapted, "mypathlib.Path");
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_adapt_dotted_module_collections_abc() {
        // Fixture: `import collections.abc` → type `collections.abc.Iterable[str]`
        // Consumer: `from collections.abc import Iterable`
        // check_name for bare `import collections.abc` is `"collections.abc"`.
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
        // Fixture: `import pathlib` → type `pathlib.Path`
        // Consumer: `import pathlib` (bare import, NOT from-import)
        // Expected: no rewrite — the consumer's map has `"pathlib"` (not `"Path"`).
        let fixture_imports = vec![spec("pathlib", "import pathlib")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));

        let (adapted, remaining) =
            adapt_type_for_consumer("pathlib.Path", &fixture_imports, &consumer_map);

        // `Path` is NOT in consumer_map — only `pathlib` is. No rewrite.
        assert_eq!(adapted, "pathlib.Path");
        assert_eq!(remaining.len(), 1);
    }

    // ── adapt_type_for_consumer: reverse (short → dotted) tests ──────────

    #[test]
    fn test_adapt_short_to_dotted_when_consumer_has_bare_import() {
        // Fixture: `from pathlib import Path` → type `Path`
        // Consumer: `import pathlib`
        // Expected: type rewritten to `pathlib.Path`, from-import spec dropped.
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
        // Fixture: `from pathlib import Path` → type `Path`
        // Consumer: `import pathlib as pl`
        // Expected: type rewritten to `pl.Path`, from-import spec dropped.
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pl".to_string(), spec("pl", "import pathlib as pl"));

        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);

        assert_eq!(adapted, "pl.Path");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_short_no_rewrite_when_consumer_lacks_bare_import() {
        // Fixture: `from pathlib import Path` → type `Path`
        // Consumer: no pathlib imports at all
        // Expected: type unchanged, from-import spec kept.
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let consumer_map = HashMap::new();

        let (adapted, remaining) = adapt_type_for_consumer("Path", &fixture_imports, &consumer_map);

        assert_eq!(adapted, "Path");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Path");
    }

    #[test]
    fn test_adapt_short_to_dotted_generic_type() {
        // Fixture: `from pathlib import Path` + `from typing import Optional`
        // Type: `Optional[Path]`
        // Consumer: `import pathlib` (but NOT `from typing import Optional`)
        // Expected: `Optional[pathlib.Path]`, Path spec dropped, Optional kept.
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
        // Type contains `PathLike` — replacing `Path` must not produce `pathlib.PathLike`.
        let fixture_imports = vec![spec("Path", "from pathlib import Path")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));

        let (adapted, remaining) =
            adapt_type_for_consumer("PathLike", &fixture_imports, &consumer_map);

        // `PathLike` is NOT the same identifier as `Path` — no rewrite.
        // The spec is kept because the replacement had no effect.
        assert_eq!(adapted, "PathLike");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].check_name, "Path");
    }

    #[test]
    fn test_adapt_short_to_dotted_multiple_occurrences() {
        // Type: `tuple[Path, Path]` — `Path` appears twice.
        // Consumer: `import pathlib`
        // Expected: both replaced to `pathlib.Path`.
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
        // Fixture: `from pathlib import Path as P` → type uses `P`
        // Consumer: `import pathlib`
        // Expected: `P` → `pathlib.Path` (uses the original name, not the alias).
        let fixture_imports = vec![spec("P", "from pathlib import Path as P")];
        let mut consumer_map = HashMap::new();
        consumer_map.insert("pathlib".to_string(), spec("pathlib", "import pathlib"));

        let (adapted, remaining) = adapt_type_for_consumer("P", &fixture_imports, &consumer_map);

        assert_eq!(adapted, "pathlib.Path");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_adapt_short_to_dotted_collections_abc() {
        // Fixture: `from collections.abc import Iterable` → type `Iterable[str]`
        // Consumer: `import collections.abc`
        // Expected: `collections.abc.Iterable[str]`, from-import spec dropped.
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
        // Mix of Case 1 and Case 2 in a single type:
        // Fixture: `import pathlib` + `from typing import Sequence`
        // Type: `Sequence[pathlib.Path]`
        // Consumer: `from pathlib import Path` + `import typing`
        // Expected: `typing.Sequence[Path]`
        //   - pathlib.Path → Path (Case 1: consumer has from-import)
        //   - Sequence → typing.Sequence (Case 2: consumer has bare import)
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

    // ── replace_type_identifier tests ────────────────────────────────────

    #[test]
    fn test_replace_identifier_simple() {
        assert_eq!(
            replace_type_identifier("Path", "Path", "pathlib.Path"),
            "pathlib.Path"
        );
    }

    #[test]
    fn test_replace_identifier_in_generic() {
        assert_eq!(
            replace_type_identifier("Optional[Path]", "Path", "pathlib.Path"),
            "Optional[pathlib.Path]"
        );
    }

    #[test]
    fn test_replace_identifier_no_partial_match() {
        // `PathLike` should NOT be affected when replacing `Path`.
        assert_eq!(
            replace_type_identifier("PathLike", "Path", "pathlib.Path"),
            "PathLike"
        );
    }

    #[test]
    fn test_replace_identifier_no_prefix_match() {
        // `MyPath` should NOT be affected when replacing `Path`.
        assert_eq!(
            replace_type_identifier("MyPath", "Path", "pathlib.Path"),
            "MyPath"
        );
    }

    #[test]
    fn test_replace_identifier_multiple_occurrences() {
        assert_eq!(
            replace_type_identifier("tuple[Path, Path]", "Path", "pathlib.Path"),
            "tuple[pathlib.Path, pathlib.Path]"
        );
    }

    #[test]
    fn test_replace_identifier_union_pipe() {
        assert_eq!(
            replace_type_identifier("Path | None", "Path", "pathlib.Path"),
            "pathlib.Path | None"
        );
    }

    #[test]
    fn test_replace_identifier_does_not_touch_dotted() {
        // If the type already has `pathlib.Path`, replacing standalone `Path`
        // should not double-qualify it. The `.` before `Path` prevents the match.
        assert_eq!(
            replace_type_identifier("pathlib.Path", "Path", "xx.Path"),
            "pathlib.Path"
        );
    }
}
