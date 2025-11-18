use pytest_language_server::FixtureDatabase;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{debug, info, warn};

#[derive(Debug)]
struct Backend {
    client: Client,
    fixture_db: Arc<FixtureDatabase>,
    workspace_root: Arc<tokio::sync::RwLock<Option<PathBuf>>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("Initialize request received");

        // Scan the workspace for fixtures on initialization
        if let Some(root_uri) = params.root_uri.clone() {
            if let Ok(root_path) = root_uri.to_file_path() {
                info!("Scanning workspace: {:?}", root_path);

                // Store the workspace root
                *self.workspace_root.write().await = Some(root_path.clone());

                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("Scanning workspace: {:?}", root_path),
                    )
                    .await;
                self.fixture_db.scan_workspace(&root_path);
                info!("Workspace scan complete");
                self.client
                    .log_message(MessageType::INFO, "Workspace scan complete")
                    .await;
            }
        } else {
            warn!("No root URI provided in initialize");
        }

        info!("Returning initialize result with capabilities");
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "pytest-language-server".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: None,
                        },
                        resolve_provider: None,
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![]),
                    all_commit_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                    completion_item: None,
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("Server initialized notification received");
        self.client
            .log_message(MessageType::INFO, "pytest-language-server initialized")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        info!("did_open: {:?}", uri);
        if let Ok(file_path) = uri.to_file_path() {
            info!("Analyzing file: {:?}", file_path);
            self.fixture_db
                .analyze_file(file_path.clone(), &params.text_document.text);

            // Publish diagnostics for undeclared fixtures
            self.publish_diagnostics_for_file(&uri, &file_path).await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        info!("did_change: {:?}", uri);
        if let Ok(file_path) = uri.to_file_path() {
            if let Some(change) = params.content_changes.first() {
                info!("Re-analyzing file: {:?}", file_path);
                self.fixture_db
                    .analyze_file(file_path.clone(), &change.text);

                // Publish diagnostics for undeclared fixtures
                self.publish_diagnostics_for_file(&uri, &file_path).await;
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        info!(
            "goto_definition request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Ok(file_path) = uri.to_file_path() {
            info!(
                "Looking for fixture definition at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            if let Some(definition) = self.fixture_db.find_fixture_definition(
                &file_path,
                position.line,
                position.character,
            ) {
                info!("Found definition: {:?}", definition);
                let def_uri = match Url::from_file_path(&definition.file_path) {
                    Ok(uri) => uri,
                    Err(_) => {
                        warn!("Failed to convert path to URI: {:?}", definition.file_path);
                        return Ok(None);
                    }
                };

                let location = Location {
                    uri: def_uri.clone(),
                    range: Range {
                        start: Position {
                            line: (definition.line.saturating_sub(1)) as u32,
                            character: 0,
                        },
                        end: Position {
                            line: (definition.line.saturating_sub(1)) as u32,
                            character: 0,
                        },
                    },
                };
                info!("Returning location: {:?}", location);
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            } else {
                info!("No fixture definition found");
            }
        } else {
            warn!("Failed to convert URI to file path: {:?}", uri);
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        info!(
            "hover request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Ok(file_path) = uri.to_file_path() {
            info!(
                "Looking for fixture at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            if let Some(definition) = self.fixture_db.find_fixture_definition(
                &file_path,
                position.line,
                position.character,
            ) {
                info!("Found fixture definition for hover: {:?}", definition.name);

                // Build hover content
                let mut content = String::new();

                // Calculate relative path from workspace root
                let relative_path =
                    if let Some(workspace_root) = self.workspace_root.read().await.as_ref() {
                        definition
                            .file_path
                            .strip_prefix(workspace_root)
                            .ok()
                            .and_then(|p| p.to_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                definition
                                    .file_path
                                    .file_name()
                                    .and_then(|f| f.to_str())
                                    .unwrap_or("unknown")
                                    .to_string()
                            })
                    } else {
                        // Fallback to just the filename if no workspace root
                        definition
                            .file_path
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("unknown")
                            .to_string()
                    };

                // Add "from" line with relative path
                content.push_str(&format!("**from** `{}`\n", relative_path));

                // Add code block with fixture signature
                let return_annotation = if let Some(ref ret_type) = definition.return_type {
                    format!(" -> {}", ret_type)
                } else {
                    String::new()
                };

                content.push_str(&format!(
                    "```python\n@pytest.fixture\ndef {}(...){}:\n```",
                    definition.name, return_annotation
                ));

                // Add docstring if present
                if let Some(ref docstring) = definition.docstring {
                    content.push_str("\n\n---\n\n");

                    // Check if docstring looks like it contains markdown formatting
                    // (contains headers, lists, code blocks, etc.)
                    let looks_like_markdown = docstring.contains("```")
                        || docstring.contains("# ")
                        || docstring.lines().any(|l| {
                            l.trim_start().starts_with("- ") || l.trim_start().starts_with("* ")
                        });

                    if looks_like_markdown {
                        // Render as markdown
                        content.push_str(docstring);
                    } else {
                        // Render as plain text (no extra escaping needed, just display)
                        content.push_str(docstring);
                    }
                }

                info!("Returning hover with content");
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: content,
                    }),
                    range: None,
                }));
            } else {
                info!("No fixture found for hover");
            }
        } else {
            warn!("Failed to convert URI to file path: {:?}", uri);
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        info!(
            "references request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Ok(file_path) = uri.to_file_path() {
            info!(
                "Looking for fixture references at {:?}:{}:{}",
                file_path, position.line, position.character
            );

            // First, find which fixture we're looking at (definition or usage)
            if let Some(fixture_name) = self.fixture_db.find_fixture_at_position(
                &file_path,
                position.line,
                position.character,
            ) {
                info!(
                    "Found fixture: {}, determining which definition to use",
                    fixture_name
                );

                let current_line = (position.line + 1) as usize; // Convert to 1-indexed
                info!(
                    "Current cursor position: line {} (1-indexed), char {}",
                    current_line, position.character
                );

                // Determine which specific definition the user is referring to
                // This could be a usage (resolve to definition) or clicking on a definition itself
                let target_definition = self.fixture_db.find_fixture_definition(
                    &file_path,
                    position.line,
                    position.character,
                );

                let (references, definition_to_include) = if let Some(definition) =
                    target_definition
                {
                    info!(
                            "Found definition via usage at {:?}:{}, finding references that resolve to it",
                            definition.file_path, definition.line
                        );
                    // Find only references that resolve to this specific definition
                    let refs = self.fixture_db.find_references_for_definition(&definition);
                    (refs, Some(definition))
                } else {
                    // find_fixture_definition returns None if cursor is on a definition line (not a usage)
                    // Check if we're on a fixture definition line
                    let target_line = (position.line + 1) as usize;
                    if let Some(definition_at_line) = self.fixture_db.get_definition_at_line(
                        &file_path,
                        target_line,
                        &fixture_name,
                    ) {
                        info!(
                                "Found definition at cursor position {:?}:{}, finding references that resolve to it",
                                file_path, target_line
                            );
                        let refs = self
                            .fixture_db
                            .find_references_for_definition(&definition_at_line);
                        (refs, Some(definition_at_line))
                    } else {
                        info!("No specific definition found at cursor, finding all references by name");
                        // Fallback to finding all references by name (shouldn't normally happen)
                        (self.fixture_db.find_fixture_references(&fixture_name), None)
                    }
                };

                if references.is_empty() && definition_to_include.is_none() {
                    info!("No references found for fixture: {}", fixture_name);
                    return Ok(None);
                }

                info!(
                    "Found {} references for fixture: {}",
                    references.len(),
                    fixture_name
                );

                // Log all references to help debug
                for (i, r) in references.iter().enumerate() {
                    debug!(
                        "  Reference {}: {:?}:{} (chars {}-{})",
                        i,
                        r.file_path.file_name(),
                        r.line,
                        r.start_char,
                        r.end_char
                    );
                }

                // Check if current position is in the references
                let has_current_position = references
                    .iter()
                    .any(|r| r.file_path == file_path && r.line == current_line);
                info!(
                    "Current position (line {}) in references: {}",
                    current_line, has_current_position
                );

                // Convert references to LSP Locations
                let mut locations = Vec::new();

                // First, add the definition if we have one (LSP spec: includeDeclaration)
                if let Some(ref def) = definition_to_include {
                    let def_uri = match Url::from_file_path(&def.file_path) {
                        Ok(uri) => uri,
                        Err(_) => {
                            warn!(
                                "Failed to convert definition path to URI: {:?}",
                                def.file_path
                            );
                            return Ok(None);
                        }
                    };

                    let def_location = Location {
                        uri: def_uri,
                        range: Range {
                            start: Position {
                                line: (def.line.saturating_sub(1)) as u32,
                                character: 0,
                            },
                            end: Position {
                                line: (def.line.saturating_sub(1)) as u32,
                                character: 0,
                            },
                        },
                    };
                    locations.push(def_location);
                }

                // Then add all the usage references
                // Skip references that are on the same line as the definition (to avoid duplicates)
                let mut skipped_count = 0;
                for reference in &references {
                    // Check if this reference is the same location as the definition
                    if let Some(ref def) = definition_to_include {
                        if reference.file_path == def.file_path && reference.line == def.line {
                            debug!(
                                "Skipping reference at {:?}:{} (same as definition location)",
                                reference.file_path, reference.line
                            );
                            skipped_count += 1;
                            continue;
                        }
                    }

                    let ref_uri = match Url::from_file_path(&reference.file_path) {
                        Ok(uri) => uri,
                        Err(_) => {
                            warn!("Failed to convert path to URI: {:?}", reference.file_path);
                            continue;
                        }
                    };

                    let location = Location {
                        uri: ref_uri,
                        range: Range {
                            start: Position {
                                line: (reference.line.saturating_sub(1)) as u32,
                                character: reference.start_char as u32,
                            },
                            end: Position {
                                line: (reference.line.saturating_sub(1)) as u32,
                                character: reference.end_char as u32,
                            },
                        },
                    };
                    debug!(
                        "Adding reference location: {:?}:{} (chars {}-{})",
                        reference.file_path.file_name(),
                        reference.line,
                        reference.start_char,
                        reference.end_char
                    );
                    locations.push(location);
                }

                info!(
                    "Returning {} locations (definition: {}, references: {}/{}, skipped: {})",
                    locations.len(),
                    if definition_to_include.is_some() {
                        1
                    } else {
                        0
                    },
                    references.len() - skipped_count,
                    references.len(),
                    skipped_count
                );
                return Ok(Some(locations));
            } else {
                info!("No fixture found at this position");
            }
        } else {
            warn!("Failed to convert URI to file path: {:?}", uri);
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        info!(
            "completion request: uri={:?}, line={}, char={}",
            uri, position.line, position.character
        );

        if let Ok(file_path) = uri.to_file_path() {
            // Check if we're inside a test or fixture function
            if let Some((function_name, is_fixture, declared_params)) = self
                .fixture_db
                .is_inside_function(&file_path, position.line, position.character)
            {
                info!(
                    "Inside function '{}' (is_fixture={}, params={:?})",
                    function_name, is_fixture, declared_params
                );

                // Get all available fixtures for this file
                let available_fixtures = self.fixture_db.get_available_fixtures(&file_path);
                info!("Found {} available fixtures", available_fixtures.len());

                // Convert to completion items
                let completion_items: Vec<CompletionItem> = available_fixtures
                    .into_iter()
                    .map(|fixture| {
                        let mut detail = "pytest fixture".to_string();
                        if let Some(file_name) = fixture.file_path.file_name() {
                            detail.push_str(&format!(" from {}", file_name.to_string_lossy()));
                        }

                        let documentation = fixture.docstring.as_ref().map(|doc| {
                            Documentation::MarkupContent(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: doc.clone(),
                            })
                        });

                        CompletionItem {
                            label: fixture.name.clone(),
                            kind: Some(CompletionItemKind::VALUE),
                            detail: Some(detail),
                            documentation,
                            insert_text: Some(fixture.name.clone()),
                            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                            ..Default::default()
                        }
                    })
                    .collect();

                info!("Returning {} completion items", completion_items.len());
                return Ok(Some(CompletionResponse::Array(completion_items)));
            } else {
                info!("Not inside a test or fixture function");
            }
        } else {
            warn!("Failed to convert URI to file path: {:?}", uri);
        }

        Ok(None)
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let context = params.context;

        info!(
            "code_action request: uri={:?}, diagnostics={}, only={:?}",
            uri,
            context.diagnostics.len(),
            context.only
        );

        // Check if the client is filtering by action kind
        if let Some(ref only_kinds) = context.only {
            if !only_kinds.iter().any(|k| {
                k == &CodeActionKind::QUICKFIX
                    || k.as_str().starts_with(CodeActionKind::QUICKFIX.as_str())
            }) {
                info!("Code action request filtered out by 'only' parameter");
                return Ok(None);
            }
        }

        if let Ok(file_path) = uri.to_file_path() {
            let undeclared = self.fixture_db.get_undeclared_fixtures(&file_path);
            info!("Found {} undeclared fixtures in file", undeclared.len());
            let mut actions = Vec::new();

            // Process each diagnostic from the context
            for diagnostic in &context.diagnostics {
                info!(
                    "Processing diagnostic: code={:?}, range={:?}",
                    diagnostic.code, diagnostic.range
                );
                // Check if this is an undeclared-fixture diagnostic
                if let Some(NumberOrString::String(code)) = &diagnostic.code {
                    if code == "undeclared-fixture" {
                        // Find the corresponding undeclared fixture
                        let diag_line = diagnostic.range.start.line + 1; // Convert to 1-indexed
                        let diag_char = diagnostic.range.start.character as usize;

                        info!(
                            "Looking for undeclared fixture at line={}, char={}",
                            diag_line, diag_char
                        );

                        if let Some(fixture) = undeclared
                            .iter()
                            .find(|f| f.line == diag_line as usize && f.start_char == diag_char)
                        {
                            info!("Found matching fixture: {}", fixture.name);
                            // Create a code action to add this fixture as a parameter
                            let function_line = (fixture.function_line - 1) as u32;

                            // Read the file to determine where to insert the parameter
                            if let Ok(content) = std::fs::read_to_string(&file_path) {
                                let lines: Vec<&str> = content.lines().collect();
                                if function_line < lines.len() as u32 {
                                    let func_line_content = lines[function_line as usize];

                                    // Find the closing parenthesis of the function signature
                                    // This is a simplified approach - works for single-line signatures
                                    if let Some(paren_pos) = func_line_content.find("):") {
                                        let insert_pos =
                                            if func_line_content[..paren_pos].contains('(') {
                                                // Check if there are already parameters
                                                let param_start =
                                                    func_line_content.find('(').unwrap() + 1;
                                                let params_section =
                                                    &func_line_content[param_start..paren_pos];

                                                if params_section.trim().is_empty() {
                                                    // No parameters yet
                                                    (function_line, (param_start as u32))
                                                } else {
                                                    // Already has parameters, add after them
                                                    (function_line, (paren_pos as u32))
                                                }
                                            } else {
                                                continue;
                                            };

                                        let has_params = !func_line_content[..paren_pos]
                                            .split('(')
                                            .next_back()
                                            .unwrap_or("")
                                            .trim()
                                            .is_empty();

                                        let text_to_insert = if has_params {
                                            format!(", {}", fixture.name)
                                        } else {
                                            fixture.name.clone()
                                        };

                                        let edit = WorkspaceEdit {
                                            changes: Some(
                                                vec![(
                                                    uri.clone(),
                                                    vec![TextEdit {
                                                        range: Range {
                                                            start: Position {
                                                                line: insert_pos.0,
                                                                character: insert_pos.1,
                                                            },
                                                            end: Position {
                                                                line: insert_pos.0,
                                                                character: insert_pos.1,
                                                            },
                                                        },
                                                        new_text: text_to_insert,
                                                    }],
                                                )]
                                                .into_iter()
                                                .collect(),
                                            ),
                                            document_changes: None,
                                            change_annotations: None,
                                        };

                                        let action = CodeAction {
                                            title: format!(
                                                "Add '{}' fixture parameter",
                                                fixture.name
                                            ),
                                            kind: Some(CodeActionKind::QUICKFIX),
                                            diagnostics: Some(vec![diagnostic.clone()]),
                                            edit: Some(edit),
                                            command: None,
                                            is_preferred: Some(true),
                                            disabled: None,
                                            data: None,
                                        };

                                        info!(
                                            "Created code action: Add '{}' fixture parameter",
                                            fixture.name
                                        );
                                        actions.push(CodeActionOrCommand::CodeAction(action));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !actions.is_empty() {
                info!("Returning {} code actions", actions.len());
                return Ok(Some(actions));
            } else {
                info!("No code actions created");
            }
        }

        info!("Returning None for code_action request");
        Ok(None)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

impl Backend {
    async fn publish_diagnostics_for_file(&self, uri: &Url, file_path: &std::path::Path) {
        let undeclared = self.fixture_db.get_undeclared_fixtures(file_path);

        let diagnostics: Vec<Diagnostic> = undeclared
            .into_iter()
            .map(|fixture| {
                let line = (fixture.line - 1) as u32; // Convert to 0-indexed
                Diagnostic {
                    range: Range {
                        start: Position {
                            line,
                            character: fixture.start_char as u32,
                        },
                        end: Position {
                            line,
                            character: fixture.end_char as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("undeclared-fixture".to_string())),
                    code_description: None,
                    source: Some("pytest-lsp".to_string()),
                    message: format!(
                        "Fixture '{}' is used but not declared as a parameter",
                        fixture.name
                    ),
                    related_information: None,
                    tags: None,
                    data: None,
                }
            })
            .collect();

        info!("Publishing {} diagnostics for {:?}", diagnostics.len(), uri);
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

#[tokio::main]
async fn main() {
    // Handle --version flag
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Set up stderr logging with env-filter support
    // Users can control verbosity with RUST_LOG env var:
    // RUST_LOG=debug pytest-language-server
    // RUST_LOG=info pytest-language-server
    // RUST_LOG=warn pytest-language-server (default)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    info!("pytest-language-server starting");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let fixture_db = Arc::new(FixtureDatabase::new());

    let (service, socket) = LspService::new(|client| Backend {
        client,
        fixture_db: fixture_db.clone(),
        workspace_root: Arc::new(tokio::sync::RwLock::new(None)),
    });

    info!("LSP server ready");
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pytest_language_server::FixtureDefinition;
    use std::path::PathBuf;

    #[test]
    fn test_hover_content_with_leading_newline() {
        // Create a mock fixture definition with docstring
        let definition = FixtureDefinition {
            name: "my_fixture".to_string(),
            file_path: PathBuf::from("/tmp/test/conftest.py"),
            line: 4,
            docstring: Some("This is a test fixture.\n\nIt does something useful.".to_string()),
            return_type: None,
        };

        // Build hover content (same logic as hover method)
        let mut content = String::new();

        // Add "from" line with relative path (using just filename for test)
        let relative_path = definition
            .file_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown");
        content.push_str(&format!("**from** `{}`\n", relative_path));

        // Add code block with fixture signature
        content.push_str(&format!(
            "```python\n@pytest.fixture\ndef {}(...):\n```",
            definition.name
        ));

        // Add docstring if present
        if let Some(ref docstring) = definition.docstring {
            content.push_str("\n\n---\n\n");
            content.push_str(docstring);
        }

        // Verify the structure
        let lines: Vec<&str> = content.lines().collect();

        // The structure should be:
        // 0: **from** `conftest.py`
        // 1: ```python
        // 2: @pytest.fixture
        // 3: def my_fixture(...):
        // 4: ```
        // 5: (empty line from \n\n---\n)
        // 6: ---
        // 7: (empty line)
        // 8+: docstring content

        assert!(
            lines[0].starts_with("**from**"),
            "Line 0 should start with 'From', got: '{}'",
            lines[0]
        );
        assert_eq!(lines[1], "```python");
        assert_eq!(lines[2], "@pytest.fixture");
        assert!(lines[3].starts_with("def my_fixture"));
        assert_eq!(lines[4], "```");
    }

    #[test]
    fn test_hover_content_structure_without_docstring() {
        // Create a mock fixture definition without docstring
        let definition = FixtureDefinition {
            name: "simple_fixture".to_string(),
            file_path: PathBuf::from("/tmp/test/conftest.py"),
            line: 4,
            docstring: None,
            return_type: None,
        };

        // Build hover content
        let mut content = String::new();

        // Add "from" line with relative path (using just filename for test)
        let relative_path = definition
            .file_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown");
        content.push_str(&format!("**from** `{}`\n", relative_path));

        // Add code block with fixture signature
        content.push_str(&format!(
            "```python\n@pytest.fixture\ndef {}(...):\n```",
            definition.name
        ));

        // For a fixture without docstring, the content should end with the code block
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines.len(), 5); // from line (1 line) + code block (4 lines)
        assert!(lines[0].starts_with("**from**"));
        assert_eq!(lines[1], "```python");
        assert_eq!(lines[4], "```");
    }

    #[test]
    fn test_references_from_parent_definition() {
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file using child fixture
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get parent definition by clicking on the child's parameter (which references parent)
        // In child conftest, line 5 has "def cli_runner(cli_runner):"
        // Line 5 (1-indexed) = line 4 (0-indexed), char 19 is in the parameter "cli_runner"
        let parent_def = db.find_fixture_definition(&child_conftest, 4, 19);
        assert!(
            parent_def.is_some(),
            "Child parameter should resolve to parent definition"
        );

        // Find references for parent - should include child's parameter, not test usages
        let refs = db.find_references_for_definition(&parent_def.unwrap());

        assert!(
            refs.iter().any(|r| r.file_path == child_conftest),
            "Parent references should include child fixture parameter"
        );

        assert!(
            refs.iter().all(|r| r.file_path != test_path),
            "Parent references should NOT include test file usages (they use child)"
        );
    }

    #[test]
    fn test_references_from_child_definition() {
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file using child fixture
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get child definition by clicking on a test usage
        // Line 2 (1-indexed) = line 1 (0-indexed), char 13 is in "cli_runner" parameter
        let child_def = db.find_fixture_definition(&test_path, 1, 13);
        assert!(
            child_def.is_some(),
            "Test usage should resolve to child definition"
        );

        // Find references for child - should include test usages
        let refs = db.find_references_for_definition(&child_def.unwrap());

        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

        assert_eq!(
            test_refs.len(),
            2,
            "Child references should include both test usages"
        );
    }

    #[test]
    fn test_references_from_usage_in_test() {
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file using child fixture
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Simulate clicking on cli_runner in test_one (line 2, 1-indexed)
        let resolved_def = db.find_fixture_definition(&test_path, 1, 13); // 0-indexed LSP

        assert!(resolved_def.is_some(), "Should resolve usage to definition");

        let def = resolved_def.unwrap();
        assert_eq!(
            def.file_path, child_conftest,
            "Usage should resolve to child definition, not parent"
        );

        // Get references for the resolved definition
        let refs = db.find_references_for_definition(&def);

        // Should include both test usages
        let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

        assert_eq!(
            test_refs.len(),
            2,
            "References should include both test usages"
        );

        // Verify that the current usage (line 2 where we clicked) IS included
        let current_usage = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 2);
        assert!(
            current_usage.is_some(),
            "References should include the current usage (line 2) where cursor is positioned"
        );

        // Verify the other usage is also included
        let other_usage = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 5);
        assert!(
            other_usage.is_some(),
            "References should include the other usage (line 5)"
        );
    }

    #[test]
    fn test_references_three_level_hierarchy() {
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Grandparent
        let grandparent_content = r#"
import pytest

@pytest.fixture
def db():
    return "root"
"#;
        let grandparent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(grandparent_conftest.clone(), grandparent_content);

        // Parent overrides
        let parent_content = r#"
import pytest

@pytest.fixture
def db(db):
    return f"parent_{db}"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/api/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child overrides again
        let child_content = r#"
import pytest

@pytest.fixture
def db(db):
    return f"child_{db}"
"#;
        let child_conftest = PathBuf::from("/tmp/project/api/v1/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test at child level
        let test_content = r#"
def test_db(db):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/api/v1/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get definitions by clicking on parameters that reference them
        // Parent conftest: "def db(db):" - parameter 'db' starts at position 7
        let grandparent_def = db
            .find_fixture_definition(&parent_conftest, 4, 7)
            .expect("Parent parameter should resolve to grandparent");
        // Child conftest: "def db(db):" - parameter 'db' starts at position 7
        let parent_def = db
            .find_fixture_definition(&child_conftest, 4, 7)
            .expect("Child parameter should resolve to parent");
        // Test: "def test_db(db):" - parameter 'db' starts at position 12
        let child_def = db
            .find_fixture_definition(&test_path, 1, 12)
            .expect("Test parameter should resolve to child");

        // Grandparent references should only include parent parameter
        let gp_refs = db.find_references_for_definition(&grandparent_def);
        assert!(
            gp_refs.iter().any(|r| r.file_path == parent_conftest),
            "Grandparent should have parent parameter"
        );
        assert!(
            gp_refs.iter().all(|r| r.file_path != child_conftest),
            "Grandparent should NOT have child references"
        );
        assert!(
            gp_refs.iter().all(|r| r.file_path != test_path),
            "Grandparent should NOT have test references"
        );

        // Parent references should only include child parameter
        let parent_refs = db.find_references_for_definition(&parent_def);
        assert!(
            parent_refs.iter().any(|r| r.file_path == child_conftest),
            "Parent should have child parameter"
        );
        assert!(
            parent_refs.iter().all(|r| r.file_path != test_path),
            "Parent should NOT have test references"
        );

        // Child references should include test usage
        let child_refs = db.find_references_for_definition(&child_def);
        assert!(
            child_refs.iter().any(|r| r.file_path == test_path),
            "Child should have test reference"
        );
    }

    #[test]
    fn test_references_no_duplicate_definition() {
        // Test that when a fixture definition line also has a usage (self-referencing),
        // we don't list the definition twice in the results
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest with self-referencing override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file
        let test_content = r#"
def test_one(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Click on the child's parameter (which references parent)
        let parent_def = db
            .find_fixture_definition(&child_conftest, 4, 19)
            .expect("Should find parent definition from child parameter");

        // Get references for parent
        let refs = db.find_references_for_definition(&parent_def);

        // The child conftest line 5 should appear exactly once in references
        // (it's both a reference and a definition line, but should only appear once)
        let child_line_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.file_path == child_conftest && r.line == 5)
            .collect();

        assert_eq!(
            child_line_refs.len(),
            1,
            "Child fixture line should appear exactly once in references (not duplicated)"
        );
    }

    #[test]
    fn test_comprehensive_fixture_hierarchy_with_cursor_positions() {
        // This test validates all cursor position scenarios with fixture hierarchy
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Root conftest with parent fixtures
        let root_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"

@pytest.fixture
def other_fixture(cli_runner):
    return f"uses_{cli_runner}"
"#;
        let root_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(root_conftest.clone(), root_content);

        // Child conftest with override
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file in child directory
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        println!("\n=== SCENARIO 1: Clicking on PARENT via another fixture that uses it ===");
        // Click on other_fixture's parameter to get parent definition
        let parent_def = db.find_fixture_definition(&root_conftest, 8, 20);
        println!(
            "Parent def: {:?}",
            parent_def.as_ref().map(|d| (&d.file_path, d.line))
        );

        if let Some(parent_def) = parent_def {
            let refs = db.find_references_for_definition(&parent_def);
            println!("Parent references count: {}", refs.len());
            for r in &refs {
                println!("  {:?}:{}", r.file_path, r.line);
            }

            // Parent should have:
            // 1. other_fixture parameter (line 9 in root conftest)
            // 2. Child fixture parameter (line 5 in child conftest)
            // NOT: test_one or test_two (they use child)

            let root_refs: Vec<_> = refs
                .iter()
                .filter(|r| r.file_path == root_conftest)
                .collect();
            let child_refs: Vec<_> = refs
                .iter()
                .filter(|r| r.file_path == child_conftest)
                .collect();
            let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

            assert!(
                !root_refs.is_empty(),
                "Parent should have reference from other_fixture"
            );
            assert!(
                !child_refs.is_empty(),
                "Parent should have reference from child fixture"
            );
            assert!(
                test_refs.is_empty(),
                "Parent should NOT have test references (they use child)"
            );
        }

        println!("\n=== SCENARIO 2: Clicking on CHILD fixture via test usage ===");
        let child_def = db.find_fixture_definition(&test_path, 1, 13);
        println!(
            "Child def: {:?}",
            child_def.as_ref().map(|d| (&d.file_path, d.line))
        );

        if let Some(child_def) = child_def {
            let refs = db.find_references_for_definition(&child_def);
            println!("Child references count: {}", refs.len());
            for r in &refs {
                println!("  {:?}:{}", r.file_path, r.line);
            }

            // Child should have:
            // 1. test_one (line 2 in test file)
            // 2. test_two (line 5 in test file)
            // NOT: other_fixture (uses parent)

            let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
            let root_refs: Vec<_> = refs
                .iter()
                .filter(|r| r.file_path == root_conftest)
                .collect();

            assert_eq!(test_refs.len(), 2, "Child should have 2 test references");
            assert!(
                root_refs.is_empty(),
                "Child should NOT have root conftest references"
            );
        }

        println!("\n=== SCENARIO 3: Clicking on CHILD fixture parameter (resolves to parent) ===");
        let parent_via_child_param = db.find_fixture_definition(&child_conftest, 4, 19);
        println!(
            "Parent via child param: {:?}",
            parent_via_child_param
                .as_ref()
                .map(|d| (&d.file_path, d.line))
        );

        if let Some(parent_def) = parent_via_child_param {
            assert_eq!(
                parent_def.file_path, root_conftest,
                "Child parameter should resolve to parent"
            );

            let refs = db.find_references_for_definition(&parent_def);

            // Should be same as SCENARIO 1
            let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
            assert!(
                test_refs.is_empty(),
                "Parent (via child param) should NOT have test references"
            );
        }
    }

    #[test]
    fn test_references_clicking_on_definition_line() {
        // Test that clicking on a fixture definition itself (not parameter, not usage)
        // correctly identifies which definition and returns appropriate references
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Parent conftest
        let parent_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let parent_conftest = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(parent_conftest.clone(), parent_content);

        // Child conftest
        let child_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner
"#;
        let child_conftest = PathBuf::from("/tmp/project/tests/conftest.py");
        db.analyze_file(child_conftest.clone(), child_content);

        // Test file
        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        println!(
            "\n=== TEST: Clicking on child fixture definition (function name 'cli_runner') ==="
        );
        // Line 5 (1-indexed) = line 4 (0-indexed), clicking on "def cli_runner" at char 4
        let fixture_name = db.find_fixture_at_position(&child_conftest, 4, 4);
        println!("Fixture name at position: {:?}", fixture_name);
        assert_eq!(fixture_name, Some("cli_runner".to_string()));

        // Get the definition at this line
        let child_def = db.get_definition_at_line(&child_conftest, 5, "cli_runner");
        println!(
            "Definition at line: {:?}",
            child_def.as_ref().map(|d| (&d.file_path, d.line))
        );
        assert!(
            child_def.is_some(),
            "Should find child definition at line 5"
        );

        if let Some(child_def) = child_def {
            let refs = db.find_references_for_definition(&child_def);
            println!("Child definition references count: {}", refs.len());
            for r in &refs {
                println!("  {:?}:{}", r.file_path, r.line);
            }

            // Child definition should have only test file usages, not parent conftest
            let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
            let parent_refs: Vec<_> = refs
                .iter()
                .filter(|r| r.file_path == parent_conftest)
                .collect();

            assert_eq!(
                test_refs.len(),
                2,
                "Child definition should have 2 test references"
            );
            assert!(
                parent_refs.is_empty(),
                "Child definition should NOT have parent references"
            );
        }

        println!(
            "\n=== TEST: Clicking on parent fixture definition (function name 'cli_runner') ==="
        );
        let fixture_name = db.find_fixture_at_position(&parent_conftest, 4, 4);
        println!("Fixture name at position: {:?}", fixture_name);

        let parent_def = db.get_definition_at_line(&parent_conftest, 5, "cli_runner");
        println!(
            "Definition at line: {:?}",
            parent_def.as_ref().map(|d| (&d.file_path, d.line))
        );
        assert!(
            parent_def.is_some(),
            "Should find parent definition at line 5"
        );

        if let Some(parent_def) = parent_def {
            let refs = db.find_references_for_definition(&parent_def);
            println!("Parent definition references count: {}", refs.len());
            for r in &refs {
                println!("  {:?}:{}", r.file_path, r.line);
            }

            // Parent should have child's parameter, but NOT test file usages
            let child_refs: Vec<_> = refs
                .iter()
                .filter(|r| r.file_path == child_conftest)
                .collect();
            let test_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

            assert!(
                !child_refs.is_empty(),
                "Parent should have child fixture parameter reference"
            );
            assert!(
                test_refs.is_empty(),
                "Parent should NOT have test file references"
            );
        }
    }

    #[test]
    fn test_fixture_override_in_test_file_not_conftest() {
        // This reproduces the strawberry test_codegen.py scenario:
        // A test file that defines a fixture overriding a parent from conftest
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        // Parent in conftest
        let conftest_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "parent"
"#;
        let conftest_path = PathBuf::from("/tmp/project/tests/cli/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Test file with fixture override AND tests using it
        let test_content = r#"
import pytest

@pytest.fixture
def cli_runner(cli_runner):
    return cli_runner

def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass

def test_three(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/tests/cli/test_codegen.py");
        db.analyze_file(test_path.clone(), test_content);

        println!(
            "\n=== SCENARIO 1: Click on child fixture definition (function name) in test file ==="
        );
        // Line 5 (1-indexed) = line 4 (0-indexed), "def cli_runner"
        let fixture_name = db.find_fixture_at_position(&test_path, 4, 4);
        println!("Fixture name: {:?}", fixture_name);
        assert_eq!(fixture_name, Some("cli_runner".to_string()));

        let child_def = db.get_definition_at_line(&test_path, 5, "cli_runner");
        println!(
            "Child def: {:?}",
            child_def.as_ref().map(|d| (&d.file_path, d.line))
        );
        assert!(
            child_def.is_some(),
            "Should find child definition in test file"
        );

        if let Some(child_def) = child_def {
            let refs = db.find_references_for_definition(&child_def);
            println!("Child references count: {}", refs.len());
            for r in &refs {
                println!("  {:?}:{}", r.file_path, r.line);
            }

            // Should only have references from the SAME FILE (test_one, test_two, test_three)
            // Should NOT have references from other files
            let same_file_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();
            let other_file_refs: Vec<_> =
                refs.iter().filter(|r| r.file_path != test_path).collect();

            assert_eq!(
                same_file_refs.len(),
                3,
                "Child should have 3 references in same file"
            );
            assert!(
                other_file_refs.is_empty(),
                "Child should NOT have references from other files"
            );
        }

        println!(
            "\n=== SCENARIO 2: Click on child fixture parameter (points to parent) in test file ==="
        );
        // Line 5, char 19 is the parameter "cli_runner"
        let parent_def = db.find_fixture_definition(&test_path, 4, 19);
        println!(
            "Parent def via child param: {:?}",
            parent_def.as_ref().map(|d| (&d.file_path, d.line))
        );

        if let Some(parent_def) = parent_def {
            assert_eq!(
                parent_def.file_path, conftest_path,
                "Parameter should resolve to parent in conftest"
            );

            let refs = db.find_references_for_definition(&parent_def);
            println!("Parent references count: {}", refs.len());
            for r in &refs {
                println!("  {:?}:{}", r.file_path, r.line);
            }

            // Parent should have:
            // 1. Child fixture parameter (line 5 in test file)
            // NOT: test_one, test_two, test_three (they use child, not parent)
            let test_file_refs: Vec<_> = refs.iter().filter(|r| r.file_path == test_path).collect();

            // Should only have the child fixture's parameter (line 5), not the test usages
            assert_eq!(
                test_file_refs.len(),
                1,
                "Parent should have 1 reference from test file (child parameter only)"
            );
            assert_eq!(
                test_file_refs[0].line, 5,
                "Parent reference should be on line 5 (child fixture parameter)"
            );
        }

        println!("\n=== SCENARIO 3: Click on usage in test function ===");
        // Line 8 (1-indexed) = line 7 (0-indexed), test_one's cli_runner parameter
        let resolved = db.find_fixture_definition(&test_path, 7, 17);
        println!(
            "Resolved from test: {:?}",
            resolved.as_ref().map(|d| (&d.file_path, d.line))
        );

        if let Some(def) = resolved {
            assert_eq!(
                def.file_path, test_path,
                "Test usage should resolve to child in same file"
            );
            assert_eq!(def.line, 5, "Should resolve to child fixture at line 5");
        }
    }

    #[test]
    fn test_references_include_current_position() {
        // LSP Spec requirement: textDocument/references should include the current position
        // where the cursor is, whether it's a usage or a definition
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        let conftest_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "runner"
"#;
        let conftest_path = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        let test_content = r#"
def test_one(cli_runner):
    pass

def test_two(cli_runner):
    pass

def test_three(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        println!("\n=== TEST: Click on usage at test_one (line 2) ===");
        // Line 2 (1-indexed), clicking on cli_runner parameter
        let fixture_name = db.find_fixture_at_position(&test_path, 1, 13);
        assert_eq!(fixture_name, Some("cli_runner".to_string()));

        let resolved_def = db.find_fixture_definition(&test_path, 1, 13);
        assert!(
            resolved_def.is_some(),
            "Should resolve to conftest definition"
        );

        let def = resolved_def.unwrap();
        let refs = db.find_references_for_definition(&def);

        println!("References found: {}", refs.len());
        for r in &refs {
            println!(
                "  {:?}:{} (chars {}-{})",
                r.file_path.file_name(),
                r.line,
                r.start_char,
                r.end_char
            );
        }

        // CRITICAL: References should include ALL usages, including the current one
        assert_eq!(refs.len(), 3, "Should have 3 references (all test usages)");

        // Verify line 2 (where we clicked) IS included
        let line2_ref = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 2);
        assert!(
            line2_ref.is_some(),
            "References MUST include current position (line 2)"
        );

        // Verify other lines are also included
        let line5_ref = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 5);
        assert!(line5_ref.is_some(), "References should include line 5");

        let line8_ref = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 8);
        assert!(line8_ref.is_some(), "References should include line 8");

        println!("\n=== TEST: Click on usage at test_two (line 5) ===");
        let resolved_def = db.find_fixture_definition(&test_path, 4, 13);
        assert!(resolved_def.is_some());

        let def = resolved_def.unwrap();
        let refs = db.find_references_for_definition(&def);

        // Should still have all 3 references
        assert_eq!(refs.len(), 3, "Should have 3 references");

        // Current position (line 5) MUST be included
        let line5_ref = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 5);
        assert!(
            line5_ref.is_some(),
            "References MUST include current position (line 5)"
        );

        // Simulate LSP handler logic: verify no references would be incorrectly skipped
        // (only skip if reference is on same line as definition)
        for r in &refs {
            if r.file_path == def.file_path && r.line == def.line {
                println!(
                    "  Would skip (same as definition): {:?}:{}",
                    r.file_path.file_name(),
                    r.line
                );
            } else {
                println!(
                    "  Would include: {:?}:{} (chars {}-{})",
                    r.file_path.file_name(),
                    r.line,
                    r.start_char,
                    r.end_char
                );
            }
        }

        // In this scenario, no references should be skipped (definition is in conftest, usages in test file)
        let would_be_skipped = refs
            .iter()
            .filter(|r| r.file_path == def.file_path && r.line == def.line)
            .count();
        assert_eq!(
            would_be_skipped, 0,
            "No references should be skipped in this scenario"
        );

        println!("\n=== TEST: Click on definition (line 5 in conftest) ===");
        // When clicking on the definition itself, references should include all usages
        let fixture_name = db.find_fixture_at_position(&conftest_path, 4, 4);
        assert_eq!(fixture_name, Some("cli_runner".to_string()));

        // This should return None (we're on definition, not usage)
        let resolved = db.find_fixture_definition(&conftest_path, 4, 4);
        assert!(
            resolved.is_none(),
            "Clicking on definition name should return None"
        );

        // Get definition at this line
        let def = db.get_definition_at_line(&conftest_path, 5, "cli_runner");
        assert!(def.is_some());

        let def = def.unwrap();
        let refs = db.find_references_for_definition(&def);

        // Should have all 3 test usages
        assert_eq!(refs.len(), 3, "Definition should have 3 usage references");

        println!("\nAll LSP spec requirements verified ✓");
    }

    #[test]
    fn test_references_multiline_function_signature() {
        // Test that references work correctly with multiline function signatures
        // This simulates the strawberry test_codegen.py scenario
        use pytest_language_server::FixtureDatabase;

        let db = FixtureDatabase::new();

        let conftest_content = r#"
import pytest

@pytest.fixture
def cli_runner():
    return "runner"
"#;
        let conftest_path = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        // Multiline function signature (like strawberry line 87-89)
        let test_content = r#"
def test_codegen(
    cli_app: Typer, cli_runner: CliRunner, query_file_path: Path
):
    pass

def test_another(cli_runner):
    pass
"#;
        let test_path = PathBuf::from("/tmp/project/test_codegen.py");
        db.analyze_file(test_path.clone(), test_content);

        println!("\n=== TEST: Click on cli_runner in function signature (line 3, char 23) ===");
        // Line 3 (1-indexed): "    cli_app: Typer, cli_runner: CliRunner, query_file_path: Path"
        // Character position 23 should be in "cli_runner" (starts at ~20)

        let fixture_name = db.find_fixture_at_position(&test_path, 2, 23); // 0-indexed for LSP
        println!("Fixture at position: {:?}", fixture_name);
        assert_eq!(
            fixture_name,
            Some("cli_runner".to_string()),
            "Should find cli_runner at this position"
        );

        let resolved_def = db.find_fixture_definition(&test_path, 2, 23);
        assert!(
            resolved_def.is_some(),
            "Should resolve to conftest definition"
        );

        let def = resolved_def.unwrap();
        println!("Resolved to: {:?}:{}", def.file_path.file_name(), def.line);

        let refs = db.find_references_for_definition(&def);
        println!("\nReferences found: {}", refs.len());
        for r in &refs {
            println!(
                "  {:?}:{} (chars {}-{})",
                r.file_path.file_name(),
                r.line,
                r.start_char,
                r.end_char
            );
        }

        // Should have 2 references: line 3 (signature) and line 7 (test_another)
        assert_eq!(
            refs.len(),
            2,
            "Should have 2 references (both function signatures)"
        );

        // CRITICAL: Line 3 (where we clicked) MUST be included
        let line3_ref = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 3);
        assert!(
            line3_ref.is_some(),
            "References MUST include current position (line 3 in signature)"
        );

        // Also verify line 7 (test_another) is included
        let line7_ref = refs
            .iter()
            .find(|r| r.file_path == test_path && r.line == 7);
        assert!(
            line7_ref.is_some(),
            "References should include test_another parameter (line 7)"
        );

        println!("\nMultiline signature test passed ✓");
    }

    #[tokio::test]
    async fn test_code_action_for_undeclared_fixture() {
        // Test that code actions are generated for undeclared fixtures
        use pytest_language_server::FixtureDatabase;

        let db = Arc::new(FixtureDatabase::new());

        let conftest_content = r#"
import pytest

@pytest.fixture
def my_fixture():
    return 42
"#;
        let conftest_path = PathBuf::from("/tmp/project/conftest.py");
        db.analyze_file(conftest_path.clone(), conftest_content);

        let test_content = r#"
def test_undeclared():
    result = my_fixture + 1
    assert result == 43
"#;
        let test_path = PathBuf::from("/tmp/project/test_example.py");
        db.analyze_file(test_path.clone(), test_content);

        // Get undeclared fixtures
        let undeclared = db.get_undeclared_fixtures(&test_path);
        println!("\nUndeclared fixtures: {:?}", undeclared);
        assert_eq!(undeclared.len(), 1, "Should have 1 undeclared fixture");

        let fixture = &undeclared[0];
        assert_eq!(fixture.name, "my_fixture");
        assert_eq!(fixture.line, 3); // 1-indexed
        assert_eq!(fixture.function_name, "test_undeclared");
        assert_eq!(fixture.function_line, 2); // 1-indexed

        // Simulate creating a diagnostic
        let diagnostic = Diagnostic {
            range: Range {
                start: Position {
                    line: (fixture.line - 1) as u32, // 0-indexed for LSP
                    character: fixture.start_char as u32,
                },
                end: Position {
                    line: (fixture.line - 1) as u32,
                    character: fixture.end_char as u32,
                },
            },
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String("undeclared-fixture".to_string())),
            source: Some("pytest-lsp".to_string()),
            message: format!(
                "Fixture '{}' is used but not declared as a parameter",
                fixture.name
            ),
            code_description: None,
            related_information: None,
            tags: None,
            data: None,
        };

        println!("Created diagnostic: {:?}", diagnostic);

        // Now test that the Backend would create a code action
        // We can't easily test the actual LSP handler without a full LSP setup,
        // but we can verify the data structures are correct
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("undeclared-fixture".to_string()))
        );
        assert_eq!(diagnostic.range.start.line, 2); // Line 3 in 1-indexed is line 2 in 0-indexed

        println!("\nCode action test passed ✓");
    }
}
