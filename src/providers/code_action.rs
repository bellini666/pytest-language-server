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
use crate::fixtures::import_analysis::{
    adapt_type_for_consumer, can_merge_into, classify_import_statement,
    find_sorted_insert_position, import_line_sort_key, import_sort_key, parse_import_layout,
    ImportGroup, ImportKind, ImportLayout,
};
use crate::fixtures::string_utils::parameter_has_annotation;
use crate::fixtures::types::TypeImportSpec;
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

/// Emit `TextEdit`s for a set of from-imports and bare imports, trying to
/// merge from-imports into existing lines before falling back to insertion.
///
/// When `group` is `Some`, new (non-merge) lines are inserted at the correct
/// isort-sorted position within the group.  When `None`, all new lines are
/// inserted at `fallback_insert_line`.
fn emit_kind_import_edits(
    layout: &ImportLayout,
    new_from_imports: &HashMap<String, Vec<String>>,
    new_bare_imports: &[String],
    group: Option<&ImportGroup>,
    fallback_insert_line: u32,
    edits: &mut Vec<TextEdit>,
) {
    // ── Pass 1: merge from-imports into existing lines where possible ────
    let mut unmerged_from: Vec<(String, Vec<String>)> = Vec::new();

    let mut modules: Vec<&String> = new_from_imports.keys().collect();
    modules.sort();

    let line_strs = layout.line_strs();

    for module in modules {
        let new_names = &new_from_imports[module];

        if let Some(fi) = layout.find_matching_from_import(module) {
            if can_merge_into(fi) {
                // Merge new names into the existing import.
                // For multiline imports (AST path), fi.name_strings() returns
                // the correct names; the TextEdit replaces all lines of the block.
                let mut all_names: Vec<String> = fi.name_strings();
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
                    fi.line, merged_line
                );

                // Cover all lines of the import (same range for single-line and
                // multiline — for single-line fi.line == fi.end_line).
                let end_char = layout.line(fi.end_line).len() as u32;
                edits.push(TextEdit {
                    range: Range {
                        start: Position {
                            line: fi.line as u32,
                            character: 0,
                        },
                        end: Position {
                            line: fi.end_line as u32,
                            character: end_char,
                        },
                    },
                    new_text: merged_line,
                });
            } else {
                // Cannot merge (string-fallback multiline without names) → insert new line.
                unmerged_from.push((module.clone(), new_names.clone()));
            }
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
    for stmt in new_bare_imports {
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
            Some(g) => find_sorted_insert_position(&line_strs, g, &ni.sort_key),
            None => fallback_insert_line,
        };
        info!("Adding new import line at {}: {}", insert_line, ni.text);
        edits.push(TextEdit {
            range: Backend::create_point_range(insert_line, 0),
            new_text: format!("{}\n", ni.text),
        });
    }
}

// ── Import-edit helpers ───────────────────────────────────────────────────────

/// Build `TextEdit`s to add import statements, respecting isort-style grouping.
///
/// Specs whose `check_name` is already in `existing_imports` are skipped.
/// New imports are classified as stdlib or third-party and placed into the
/// correct import group (creating a new group with blank-line separators when
/// necessary).  Within a group, from-imports for the same module are merged
/// into a single line with names sorted alphabetically.
fn build_import_edits(
    layout: &ImportLayout,
    specs: &[&TypeImportSpec],
    existing_imports: &HashSet<String>,
) -> Vec<TextEdit> {
    let groups = &layout.groups;

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

        if let Some(rest) = spec.import_statement.strip_prefix("from ") {
            if let Some((module, name)) = rest.split_once(" import ") {
                let module = module.trim();
                let name = name.trim();
                if !module.is_empty() && !name.is_empty() {
                    match kind {
                        ImportKind::Future | ImportKind::Stdlib => &mut stdlib_from,
                        ImportKind::ThirdParty => &mut tp_from,
                    }
                    .entry(module.to_string())
                    .or_default()
                    .push(name.to_string());
                    continue;
                }
            }
        }
        match kind {
            ImportKind::Future | ImportKind::Stdlib => &mut stdlib_bare,
            ImportKind::ThirdParty => &mut tp_bare,
        }
        .push(spec.import_statement.clone());
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
    let will_insert_stdlib =
        stdlib_from
            .keys()
            .any(|m| match layout.find_matching_from_import(m) {
                None => true,
                Some(fi) => !can_merge_into(fi),
            })
            || !stdlib_bare.is_empty();
    let will_insert_tp = tp_from
        .keys()
        .any(|m| match layout.find_matching_from_import(m) {
            None => true,
            Some(fi) => !can_merge_into(fi),
        })
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
            layout,
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
            layout,
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
        // Cached by content hash — reused across code-action and inlay-hint requests.
        let consumer_import_map = self.fixture_db.get_name_to_import_map(&file_path, &content);

        // Parse the import layout once for this request (groups + individual
        // import entries).  Used by build_import_edits for all three action kinds.
        let layout = parse_import_layout(&content);

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
                let mut all_edits = build_import_edits(&layout, &spec_refs, &existing_imports);

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
                            // Skip string-based usages from @pytest.mark.usefixtures(...),
                            // pytestmark assignments, and parametrize indirect — these are
                            // not function parameters and cannot receive type annotations.
                            if !usage.is_parameter {
                                continue;
                            }

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
                                build_import_edits(&layout, &spec_refs, &existing_imports);

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
                            // Skip string-based usages from @pytest.mark.usefixtures(...),
                            // pytestmark assignments, and parametrize indirect — these are
                            // not function parameters and cannot receive type annotations.
                            if !usage.is_parameter {
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
                                build_import_edits(&layout, &spec_refs, &existing_imports);
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
    use crate::fixtures::import_analysis::parse_import_layout;

    // ── helper ───────────────────────────────────────────────────────────

    /// Build an ImportLayout from a slice of lines joined with newlines.
    fn layout_from_lines(lines: &[&str]) -> ImportLayout {
        parse_import_layout(&lines.join("\n"))
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

    // ── build_import_edits tests ─────────────────────────────────────────

    #[test]
    fn test_build_import_edits_merge_into_existing() {
        let lines = vec![
            "import pytest",
            "from typing import Optional",
            "",
            "def test(): pass",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.start.character, 0);
        assert_eq!(edits[0].range.end.line, 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional");
    }

    #[test]
    fn test_build_import_edits_skips_already_imported() {
        let lines = vec!["from typing import Any"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let mut existing: HashSet<String> = HashSet::new();
        existing.insert("Any".to_string());
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert!(edits.is_empty());
    }

    #[test]
    fn test_build_import_edits_merge_multiple_into_existing() {
        let lines = vec!["from typing import Union", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec1 = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec2 = TypeImportSpec {
            check_name: "Optional".to_string(),
            import_statement: "from typing import Optional".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec1, &spec2], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional, Union");
    }

    #[test]
    fn test_build_import_edits_merge_preserves_alias() {
        let lines = vec!["from pathlib import Path as P", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "PurePath".to_string(),
            import_statement: "from pathlib import PurePath".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "from pathlib import Path as P, PurePath");
    }

    #[test]
    fn test_build_import_edits_deduplicates_specs() {
        let lines = vec!["import pytest", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec1 = TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        };
        let spec2 = TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec1, &spec2], &existing);
        let import_edits: Vec<_> = edits
            .iter()
            .filter(|e| e.new_text.contains("Path"))
            .collect();
        assert_eq!(import_edits.len(), 1);
        assert_eq!(import_edits[0].new_text, "from pathlib import Path\n");
    }

    #[test]
    fn test_build_import_edits_merge_into_multi_name_existing() {
        let lines = vec!["from os import path, othermodule", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "getcwd".to_string(),
            import_statement: "from os import getcwd".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "from os import getcwd, othermodule, path"
        );
    }

    #[test]
    fn test_build_import_edits_merge_strips_comment() {
        let lines = vec![
            "from typing import Any  # needed for X",
            "",
            "def test(): pass",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Optional".to_string(),
            import_statement: "from typing import Optional".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional");
        assert!(
            !edits[0].new_text.contains('#'),
            "merged line must not contain the original comment"
        );
    }

    #[test]
    fn test_build_import_edits_multiline_import_merged() {
        // With AST-based parsing, merging into a multiline import is now supported.
        // The entire block (lines 0–3) should be replaced with a single merged line.
        let lines = vec![
            "from typing import (",
            "    Any,",
            "    Optional,",
            ")",
            "",
            "def test(): pass",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Union".to_string(),
            import_statement: "from typing import Union".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);

        // Should merge all names into a single replacement edit.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 0);
        assert_eq!(edits[0].range.start.character, 0);
        assert_eq!(edits[0].range.end.line, 3);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional, Union");
    }

    // adapt tests live in src/fixtures/import_analysis.rs

    #[test]
    fn test_stdlib_import_into_existing_stdlib_group() {
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.framework import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_stdlib_import_before_third_party_when_no_stdlib_group() {
        let lines = vec![
            "import pytest",
            "from vcc.framework import fixture",
            "",
            "def test(): pass",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
        assert_eq!(edits[0].range.start.line, 0);
        assert_eq!(edits[1].new_text, "\n");
        assert_eq!(edits[1].range.start.line, 0);
    }

    #[test]
    fn test_third_party_import_after_stdlib_when_no_tp_group() {
        let lines = vec!["import os", "import time", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new_text, "\n");
        assert_eq!(edits[0].range.start.line, 2);
        assert_eq!(edits[1].new_text, "from flask.testing import FlaskClient\n");
        assert_eq!(edits[1].range.start.line, 2);
    }

    #[test]
    fn test_third_party_import_into_existing_tp_group() {
        let lines = vec!["import time", "", "import pytest", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 3);
        assert_eq!(edits[0].new_text, "from flask.testing import FlaskClient\n");
    }

    #[test]
    fn test_no_imports_at_all() {
        let lines = vec!["def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Path".to_string(),
            import_statement: "from pathlib import Path".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 0);
        assert_eq!(edits[0].new_text, "from pathlib import Path\n");
    }

    #[test]
    fn test_both_stdlib_and_tp_imports_no_existing_groups() {
        let lines = vec!["def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec_stdlib = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec_tp = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec_stdlib, &spec_tp], &existing);
        assert_eq!(edits.len(), 3);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
        assert_eq!(edits[1].new_text, "\n");
        assert_eq!(edits[2].new_text, "from flask.testing import FlaskClient\n");
    }

    #[test]
    fn test_bare_stdlib_import_sorted_within_group() {
        let lines = vec![
            "import os",
            "import time",
            "",
            "import pytest",
            "",
            "def test(): pass",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "pathlib".to_string(),
            import_statement: "import pathlib".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "import pathlib\n");
    }

    #[test]
    fn test_from_import_sorts_after_bare_imports_in_group() {
        let lines = vec!["import os", "import time", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 2);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_mixed_stdlib_from_imports_grouped() {
        let lines = vec!["import time", "", "import pytest", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec1 = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec2 = TypeImportSpec {
            check_name: "Optional".to_string(),
            import_statement: "from typing import Optional".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec1, &spec2], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].new_text, "from typing import Any, Optional\n");
    }

    #[test]
    fn test_tp_from_import_sorted_before_existing() {
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.conxtfw.framework.pytest.fixtures.component import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "conx_canoe".to_string(),
            import_statement: "from vcc import conx_canoe".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 3);
        assert_eq!(edits[0].new_text, "from vcc import conx_canoe\n");
    }

    #[test]
    fn test_user_scenario_stdlib_into_correct_group() {
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.conxtfw.framework.pytest.fixtures.component import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.start.character, 0);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_user_scenario_fix_all_multi_import() {
        let lines = vec![
            "import time",
            "",
            "import pytest",
            "from vcc.conxtfw.framework.pytest.fixtures.component import fixture",
            "",
            "LOGGING_TIME = 2",
        ];
        let layout = layout_from_lines(&lines);
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
        let edits = build_import_edits(
            &layout,
            &[&spec_typing, &spec_pathlib, &spec_vcc],
            &existing,
        );
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
        // `from __future__ import annotations` gets ImportKind::Future.
        // `last_stdlib_group` skips Future groups → stdlib inserts after os/time.
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
        let layout = layout_from_lines(&lines);
        let spec = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec], &existing);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 4);
        assert_eq!(edits[0].new_text, "from typing import Any\n");
    }

    #[test]
    fn test_different_modules_stdlib_and_tp() {
        let lines = vec!["import os", "", "import pytest", "", "def test(): pass"];
        let layout = layout_from_lines(&lines);
        let spec_stdlib = TypeImportSpec {
            check_name: "Any".to_string(),
            import_statement: "from typing import Any".to_string(),
        };
        let spec_tp = TypeImportSpec {
            check_name: "FlaskClient".to_string(),
            import_statement: "from flask.testing import FlaskClient".to_string(),
        };
        let existing: HashSet<String> = HashSet::new();
        let edits = build_import_edits(&layout, &[&spec_stdlib, &spec_tp], &existing);
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
}
