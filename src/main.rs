use clap::{Parser, Subcommand};
use pytest_language_server::FixtureDatabase;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{debug, error, info, warn};

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
        // This is done in a background task to avoid blocking the LSP initialization
        if let Some(root_uri) = params.root_uri.clone() {
            if let Ok(root_path) = root_uri.to_file_path() {
                info!("Starting workspace scan: {:?}", root_path);

                // Store the workspace root
                *self.workspace_root.write().await = Some(root_path.clone());

                // Clone references for the background task
                let fixture_db = Arc::clone(&self.fixture_db);
                let client = self.client.clone();

                // Spawn workspace scanning in a background task
                // This allows the LSP to respond immediately while scanning continues
                tokio::spawn(async move {
                    client
                        .log_message(
                            MessageType::INFO,
                            format!("Scanning workspace: {:?}", root_path),
                        )
                        .await;

                    // Run the synchronous scan in a blocking task to avoid blocking the async runtime
                    let scan_result = tokio::task::spawn_blocking(move || {
                        fixture_db.scan_workspace(&root_path);
                    })
                    .await;

                    match scan_result {
                        Ok(()) => {
                            info!("Workspace scan complete");
                            client
                                .log_message(MessageType::INFO, "Workspace scan complete")
                                .await;
                        }
                        Err(e) => {
                            error!("Workspace scan failed: {:?}", e);
                            client
                                .log_message(
                                    MessageType::ERROR,
                                    format!("Workspace scan failed: {:?}", e),
                                )
                                .await;
                        }
                    }
                });
            }
        } else {
            warn!("No root URI provided in initialize - workspace scanning disabled");
            self.client
                .log_message(
                    MessageType::WARNING,
                    "No workspace root provided - fixture analysis disabled",
                )
                .await;
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
        if let Some(file_path) = self.uri_to_path(&uri) {
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
        if let Some(file_path) = self.uri_to_path(&uri) {
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

        if let Some(file_path) = self.uri_to_path(&uri) {
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
                let Some(def_uri) = self.path_to_uri(&definition.file_path) else {
                    return Ok(None);
                };

                let def_line = Self::internal_line_to_lsp(definition.line);
                let location = Location {
                    uri: def_uri.clone(),
                    range: Self::create_point_range(def_line, 0),
                };
                info!("Returning location: {:?}", location);
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            } else {
                info!("No fixture definition found");
            }
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

        if let Some(file_path) = self.uri_to_path(&uri) {
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

        if let Some(file_path) = self.uri_to_path(&uri) {
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

                let current_line = Self::lsp_line_to_internal(position.line);
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
                    let target_line = Self::lsp_line_to_internal(position.line);
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
                    let Some(def_uri) = self.path_to_uri(&def.file_path) else {
                        return Ok(None);
                    };

                    let def_line = Self::internal_line_to_lsp(def.line);
                    let def_location = Location {
                        uri: def_uri,
                        range: Self::create_point_range(def_line, 0),
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

                    let Some(ref_uri) = self.path_to_uri(&reference.file_path) else {
                        continue;
                    };

                    let ref_line = Self::internal_line_to_lsp(reference.line);
                    let location = Location {
                        uri: ref_uri,
                        range: Self::create_range(
                            ref_line,
                            reference.start_char as u32,
                            ref_line,
                            reference.end_char as u32,
                        ),
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

        if let Some(file_path) = self.uri_to_path(&uri) {
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

        if let Some(file_path) = self.uri_to_path(&uri) {
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
                        let diag_line = Self::lsp_line_to_internal(diagnostic.range.start.line);
                        let diag_char = diagnostic.range.start.character as usize;

                        info!(
                            "Looking for undeclared fixture at line={}, char={}",
                            diag_line, diag_char
                        );

                        if let Some(fixture) = undeclared
                            .iter()
                            .find(|f| f.line == diag_line && f.start_char == diag_char)
                        {
                            info!("Found matching fixture: {}", fixture.name);
                            // Create a code action to add this fixture as a parameter
                            let function_line = Self::internal_line_to_lsp(fixture.function_line);

                            // Read the file to determine where to insert the parameter
                            if let Ok(content) = std::fs::read_to_string(&file_path) {
                                let lines: Vec<&str> = content.lines().collect();
                                // Use get() instead of direct indexing for safety
                                if let Some(func_line_content) = lines.get(function_line as usize) {
                                    // Find the closing parenthesis of the function signature
                                    // This is a simplified approach - works for single-line signatures
                                    if let Some(paren_pos) = func_line_content.find("):") {
                                        let insert_pos = if func_line_content[..paren_pos]
                                            .contains('(')
                                        {
                                            // Check if there are already parameters
                                            // Use find() result safely without unwrap
                                            let param_start = match func_line_content.find('(') {
                                                Some(pos) => pos + 1,
                                                None => {
                                                    warn!("Invalid function signature: missing opening parenthesis at {:?}:{}", file_path, function_line);
                                                    continue;
                                                }
                                            };
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
                                                        range: Self::create_point_range(
                                                            insert_pos.0,
                                                            insert_pos.1,
                                                        ),
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
    /// Convert URI to PathBuf with error logging
    fn uri_to_path(&self, uri: &Url) -> Option<PathBuf> {
        match uri.to_file_path() {
            Ok(path) => Some(path),
            Err(_) => {
                warn!("Failed to convert URI to file path: {:?}", uri);
                None
            }
        }
    }

    /// Convert PathBuf to URI with error logging
    fn path_to_uri(&self, path: &std::path::Path) -> Option<Url> {
        match Url::from_file_path(path) {
            Ok(uri) => Some(uri),
            Err(_) => {
                warn!("Failed to convert path to URI: {:?}", path);
                None
            }
        }
    }

    /// Convert LSP position (0-based line) to internal representation (1-based line)
    fn lsp_line_to_internal(line: u32) -> usize {
        (line + 1) as usize
    }

    /// Convert internal line (1-based) to LSP position (0-based)
    fn internal_line_to_lsp(line: usize) -> u32 {
        line.saturating_sub(1) as u32
    }

    /// Create a Range from start and end positions
    fn create_range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
        Range {
            start: Position {
                line: start_line,
                character: start_char,
            },
            end: Position {
                line: end_line,
                character: end_char,
            },
        }
    }

    /// Create a point Range (start == end) for a single position
    fn create_point_range(line: u32, character: u32) -> Range {
        Self::create_range(line, character, line, character)
    }

    async fn publish_diagnostics_for_file(&self, uri: &Url, file_path: &std::path::Path) {
        let undeclared = self.fixture_db.get_undeclared_fixtures(file_path);

        let diagnostics: Vec<Diagnostic> = undeclared
            .into_iter()
            .map(|fixture| {
                let line = Self::internal_line_to_lsp(fixture.line);
                Diagnostic {
                    range: Self::create_range(
                        line,
                        fixture.start_char as u32,
                        line,
                        fixture.end_char as u32,
                    ),
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

/// A blazingly fast Language Server Protocol implementation for pytest
#[derive(Parser)]
#[command(name = "pytest-language-server")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "A Language Server Protocol implementation for pytest", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Fixture-related commands
    Fixtures {
        #[command(subcommand)]
        command: FixtureCommands,
    },
}

#[derive(Subcommand)]
enum FixtureCommands {
    /// List all fixtures in a hierarchical tree view
    List {
        /// Path to the directory containing test files
        path: PathBuf,

        /// Skip unused fixtures from the output
        #[arg(long)]
        skip_unused: bool,

        /// Show only unused fixtures
        #[arg(long, conflicts_with = "skip_unused")]
        only_unused: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Fixtures { command }) => match command {
            FixtureCommands::List {
                path,
                skip_unused,
                only_unused,
            } => {
                handle_fixtures_list(path, skip_unused, only_unused);
            }
        },
        None => {
            // No subcommand provided - start LSP server
            start_lsp_server().await;
        }
    }
}

fn handle_fixtures_list(path: PathBuf, skip_unused: bool, only_unused: bool) {
    // Convert to absolute path
    let absolute_path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&path)
    };

    if !absolute_path.exists() {
        eprintln!("Error: Path does not exist: {}", absolute_path.display());
        std::process::exit(1);
    }

    if !absolute_path.is_dir() {
        eprintln!(
            "Error: Path is not a directory: {}",
            absolute_path.display()
        );
        std::process::exit(1);
    }

    // Canonicalize the path to resolve symlinks and relative components
    let canonical_path = absolute_path.canonicalize().unwrap_or(absolute_path);

    // Create a fixture database and scan the directory
    let fixture_db = FixtureDatabase::new();
    fixture_db.scan_workspace(&canonical_path);

    // Print the tree
    fixture_db.print_fixtures_tree(&canonical_path, skip_unused, only_unused);
}

async fn start_lsp_server() {
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
