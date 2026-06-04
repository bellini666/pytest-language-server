//! Rename provider for `@pytest.mark.parametrize` parameters.
//!
//! Renaming a parametrized parameter rewrites, in one edit, the name token inside the
//! `@pytest.mark.parametrize(...)` decorator string, the matching function-signature parameter,
//! and every usage of that parameter in the function body.  The rename can be triggered from any
//! of those three sites.
//!
//! Only parametrize parameters are handled; for any other symbol the request returns `None` so a
//! general Python language server can answer it.

use super::Backend;
use crate::fixtures::{decorators, FixtureDatabase};
use rustpython_parser::ast::{Arguments, Expr, ExprName, Ranged, Stmt, Visitor};
use rustpython_parser::text_size::TextRange;
use rustpython_parser::{parse, Mode};
use std::collections::{HashMap, HashSet};
use tower_lsp_server::jsonrpc::{Error, Result};
use tower_lsp_server::ls_types::*;
use tracing::info;

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// All occurrences of a parametrize parameter within one test function that must change together.
struct RenameTarget {
    /// LSP range of the token under the cursor (for the prepareRename response).
    cursor_token: Range,
    /// Every editable occurrence: decorator name token(s), signature parameter, body usages.
    edits: Vec<Range>,
}

/// A function definition with the parts needed for parametrize rename, borrowed from the AST.
struct FuncCtx<'a> {
    decorators: &'a [Expr],
    args: &'a Arguments,
    body: &'a [Stmt],
    range: TextRange,
}

impl FuncCtx<'_> {
    /// Source span covering the decorators and the `def` body, used to locate the cursor.
    /// (`FunctionDef.range` starts at `def`, so decorators must be folded in explicitly.)
    fn bounds(&self) -> (usize, usize) {
        let mut start = self.range.start().to_usize();
        for dec in self.decorators {
            start = start.min(dec.range().start().to_usize());
        }
        (start, self.range.end().to_usize())
    }

    fn contains(&self, offset: usize) -> bool {
        let (start, end) = self.bounds();
        start <= offset && offset <= end
    }

    fn span(&self) -> usize {
        let (start, end) = self.bounds();
        end - start
    }
}

/// Collects the ranges of every `Name` expression matching a target identifier, recursing through
/// the whole subtree via the generated `Visitor` default traversal.
struct NameUsageCollector {
    target: String,
    ranges: Vec<TextRange>,
}

impl Visitor for NameUsageCollector {
    fn visit_expr_name(&mut self, node: ExprName) {
        if node.id.as_str() == self.target {
            self.ranges.push(node.range);
        }
    }
}

impl Backend {
    /// Handle a `textDocument/prepareRename` request.
    pub async fn handle_prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        let Some(file_path) = self.uri_to_path(&uri) else {
            return Ok(None);
        };
        let Some(content) = self.fixture_db.get_file_content(&file_path) else {
            return Ok(None);
        };

        Ok(self
            .parametrize_rename_target(&content, position)
            .map(|target| PrepareRenameResponse::Range(target.cursor_token)))
    }

    /// Handle a `textDocument/rename` request.
    pub async fn handle_rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        let Some(file_path) = self.uri_to_path(&uri) else {
            return Ok(None);
        };
        let Some(content) = self.fixture_db.get_file_content(&file_path) else {
            return Ok(None);
        };

        let Some(target) = self.parametrize_rename_target(&content, position) else {
            return Ok(None);
        };

        if !is_valid_python_identifier(&new_name) {
            return Err(Error::invalid_params(format!(
                "'{new_name}' is not a valid Python identifier"
            )));
        }

        info!(
            "rename: {} occurrence(s) of parametrize param -> '{}'",
            target.edits.len(),
            new_name
        );

        let edits: Vec<TextEdit> = target
            .edits
            .into_iter()
            .map(|range| TextEdit {
                range,
                new_text: new_name.clone(),
            })
            .collect();

        let mut changes = HashMap::new();
        changes.insert(uri, edits);

        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    /// Resolve the parametrize parameter at `position` and gather all of its occurrences.
    fn parametrize_rename_target(&self, content: &str, position: Position) -> Option<RenameTarget> {
        let rustpython_parser::ast::Mod::Module(module) = parse(content, Mode::Module, "").ok()?
        else {
            return None;
        };

        let line_index = FixtureDatabase::build_line_index(content);
        let cursor_offset = *line_index.get(position.line as usize)? + position.character as usize;

        // Innermost function whose decorators or body contain the cursor.
        let mut functions = Vec::new();
        collect_functions(&module.body, &mut functions);
        let func = functions
            .into_iter()
            .filter(|f| f.contains(cursor_offset))
            .min_by_key(FuncCtx::span)?;

        // Parametrize names declared across all decorators, excluding indirect ones (those route
        // to a fixture, so a local-only rename would silently break the test).
        let mut name_to_decorator_ranges: HashMap<String, Vec<TextRange>> = HashMap::new();
        let mut indirect: HashSet<String> = HashSet::new();
        for dec in func.decorators {
            for (name, _) in decorators::extract_parametrize_indirect_fixtures(dec) {
                indirect.insert(name);
            }
        }
        for dec in func.decorators {
            for (name, range) in decorators::extract_parametrize_argnames(dec) {
                if indirect.contains(&name) {
                    continue;
                }
                name_to_decorator_ranges
                    .entry(name)
                    .or_default()
                    .push(range);
            }
        }
        if name_to_decorator_ranges.is_empty() {
            return None;
        }

        // Signature parameter names, used to confirm the cursor sits on a real parameter.
        let signature_params: HashSet<&str> = FixtureDatabase::all_args(func.args)
            .map(|arg| arg.def.arg.as_str())
            .collect();

        // Determine the target name from whichever site the cursor is on.
        let target_name = name_to_decorator_ranges
            .iter()
            .find(|(_, ranges)| ranges.iter().any(|r| range_contains(r, cursor_offset)))
            .map(|(name, _)| name.clone())
            .or_else(|| {
                let line_content = content.lines().nth(position.line as usize)?;
                let word = self
                    .fixture_db
                    .extract_word_at_position(line_content, position.character as usize)?;
                (name_to_decorator_ranges.contains_key(&word)
                    && signature_params.contains(word.as_str()))
                .then_some(word)
            })?;

        // Gather every occurrence to edit.
        let mut occurrences: Vec<TextRange> = Vec::new();
        occurrences.extend(
            name_to_decorator_ranges
                .remove(&target_name)
                .into_iter()
                .flatten(),
        );

        if let Some(arg) =
            FixtureDatabase::all_args(func.args).find(|arg| arg.def.arg.as_str() == target_name)
        {
            let start = arg.def.range.start();
            occurrences.push(TextRange::new(
                start,
                start + rustpython_parser::text_size::TextSize::from(target_name.len() as u32),
            ));
        }

        let mut collector = NameUsageCollector {
            target: target_name.clone(),
            ranges: Vec::new(),
        };
        for stmt in func.body {
            collector.visit_stmt(stmt.clone());
        }
        occurrences.extend(collector.ranges);

        occurrences.sort_by_key(|r| (r.start().to_usize(), r.end().to_usize()));
        occurrences.dedup();

        let cursor_tr = occurrences
            .iter()
            .find(|r| range_contains(r, cursor_offset))
            .copied()
            .unwrap_or(occurrences[0]);

        let to_lsp = |tr: &TextRange| self.text_range_to_lsp(tr, &line_index);
        Some(RenameTarget {
            cursor_token: to_lsp(&cursor_tr),
            edits: occurrences.iter().map(to_lsp).collect(),
        })
    }

    /// Convert a source [`TextRange`] into an LSP [`Range`] using the file's line index.
    fn text_range_to_lsp(&self, tr: &TextRange, line_index: &[usize]) -> Range {
        let start_offset = tr.start().to_usize();
        let end_offset = tr.end().to_usize();
        let start_line = self
            .fixture_db
            .get_line_from_offset(start_offset, line_index);
        let end_line = self.fixture_db.get_line_from_offset(end_offset, line_index);
        Range {
            start: Position {
                line: (start_line - 1) as u32,
                character: self
                    .fixture_db
                    .get_char_position_from_offset(start_offset, line_index)
                    as u32,
            },
            end: Position {
                line: (end_line - 1) as u32,
                character: self
                    .fixture_db
                    .get_char_position_from_offset(end_offset, line_index)
                    as u32,
            },
        }
    }
}

fn range_contains(range: &TextRange, offset: usize) -> bool {
    range.start().to_usize() <= offset && offset <= range.end().to_usize()
}

/// Recursively collect every function definition, descending into classes and nested functions.
fn collect_functions<'a>(stmts: &'a [Stmt], out: &mut Vec<FuncCtx<'a>>) {
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef(f) => {
                out.push(FuncCtx {
                    decorators: &f.decorator_list,
                    args: &f.args,
                    body: &f.body,
                    range: f.range,
                });
                collect_functions(&f.body, out);
            }
            Stmt::AsyncFunctionDef(f) => {
                out.push(FuncCtx {
                    decorators: &f.decorator_list,
                    args: &f.args,
                    body: &f.body,
                    range: f.range,
                });
                collect_functions(&f.body, out);
            }
            Stmt::ClassDef(c) => collect_functions(&c.body, out),
            _ => {}
        }
    }
}

fn is_valid_python_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    if !chars.all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return false;
    }
    !PYTHON_KEYWORDS.contains(&name)
}
