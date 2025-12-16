mod config;
mod fixtures;
mod providers;

use clap::{Parser, Subcommand};
use fixtures::FixtureDatabase;
use providers::Backend;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::request::{GotoImplementationParams, GotoImplementationResponse};
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{LanguageServer, LspService, Server};
use tracing::{error, info, warn};

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("Initialize request received");

        // Scan the workspace for fixtures on initialization
        // This is done in a background task to avoid blocking the LSP initialization
        // Try workspace_folders first (preferred), fall back to deprecated root_uri
        let root_uri = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .map(|folder| folder.uri.clone())
            .or_else(|| {
                #[allow(deprecated)]
                params.root_uri.clone()
            });

        if let Some(root_uri) = root_uri {
            if let Some(root_path) = root_uri.to_file_path() {
                let root_path = root_path.to_path_buf();
                info!("Starting workspace scan: {:?}", root_path);

                // Store the original workspace root (as client provided it)
                *self.original_workspace_root.write().await = Some(root_path.clone());

                // Store the canonical workspace root (with symlinks resolved)
                let canonical_root = root_path
                    .canonicalize()
                    .unwrap_or_else(|_| root_path.clone());
                *self.workspace_root.write().await = Some(canonical_root.clone());

                // Load configuration from pyproject.toml
                let config = config::Config::load(&root_path);
                info!("Loaded config: {:?}", config);
                *self.config.write().await = config;

                // Clone references for the background task
                let fixture_db = Arc::clone(&self.fixture_db);
                let client = self.client.clone();
                let exclude_patterns = self.config.read().await.exclude.clone();

                // Spawn workspace scanning in a background task
                // This allows the LSP to respond immediately while scanning continues
                let scan_handle = tokio::spawn(async move {
                    client
                        .log_message(
                            MessageType::INFO,
                            format!("Scanning workspace: {:?}", root_path),
                        )
                        .await;

                    // Run the synchronous scan in a blocking task to avoid blocking the async runtime
                    let scan_result = tokio::task::spawn_blocking(move || {
                        fixture_db.scan_workspace_with_excludes(&root_path, &exclude_patterns);
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

                // Store the handle so we can cancel it on shutdown
                *self.scan_task.lock().await = Some(scan_handle);
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
                    trigger_characters: Some(vec!["\"".to_string()]),
                    all_commit_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                    completion_item: None,
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                inlay_hint_provider: Some(OneOf::Left(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
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
            // Cache the original URI for this canonical path
            // This ensures we respond with URIs the client recognizes
            self.uri_cache.insert(file_path.clone(), uri.clone());

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

                // Request inlay hint refresh so editors update hints after edits
                // (e.g., when user adds/removes type annotations)
                if let Err(e) = self.client.inlay_hint_refresh().await {
                    // Not all clients support this, so just log and continue
                    info!(
                        "Inlay hint refresh request failed (client may not support it): {}",
                        e
                    );
                }
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        info!("did_close: {:?}", uri);
        if let Some(file_path) = self.uri_to_path(&uri) {
            // Clean up cached data for this file to prevent unbounded memory growth
            self.fixture_db.cleanup_file_cache(&file_path);
            // Clean up URI cache entry
            self.uri_cache.remove(&file_path);
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.handle_goto_definition(params).await
    }

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        self.handle_goto_implementation(params).await
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.handle_hover(params).await
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.handle_references(params).await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.handle_completion(params).await
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        self.handle_code_action(params).await
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        self.handle_document_symbol(params).await
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<WorkspaceSymbolResponse>> {
        let result = self.handle_workspace_symbol(params).await?;
        Ok(result.map(WorkspaceSymbolResponse::Flat))
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        self.handle_code_lens(params).await
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        self.handle_inlay_hint(params).await
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        self.handle_prepare_call_hierarchy(params).await
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        self.handle_incoming_calls(params).await
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        self.handle_outgoing_calls(params).await
    }

    async fn shutdown(&self) -> Result<()> {
        info!("Shutdown request received");

        // Cancel the background scan task if it's still running
        if let Some(handle) = self.scan_task.lock().await.take() {
            info!("Aborting background workspace scan task");
            handle.abort();
            // Wait briefly for the task to finish (don't block shutdown indefinitely)
            match tokio::time::timeout(std::time::Duration::from_millis(100), handle).await {
                Ok(Ok(_)) => info!("Background scan task already completed"),
                Ok(Err(_)) => info!("Background scan task aborted"),
                Err(_) => info!("Background scan task abort timed out, continuing shutdown"),
            }
        }

        info!("Shutdown complete");

        // tower-lsp doesn't always exit cleanly after the exit notification
        // (serve() may block on stdin/stdout), so we spawn a task to force exit
        // after a brief delay to allow the shutdown response to be sent
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            info!("Forcing process exit");
            std::process::exit(0);
        });

        Ok(())
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
    /// Check for unused fixtures (exits with code 1 if found)
    Unused {
        /// Path to the directory containing test files
        path: PathBuf,

        /// Output format: "text" (default) or "json"
        #[arg(long, default_value = "text")]
        format: String,
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
            FixtureCommands::Unused { path, format } => {
                handle_fixtures_unused(path, &format);
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

fn handle_fixtures_unused(path: PathBuf, format: &str) {
    use colored::Colorize;

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

    // Get unused fixtures
    let unused = fixture_db.get_unused_fixtures();

    if unused.is_empty() {
        if format == "json" {
            println!("[]");
        } else {
            println!("{}", "No unused fixtures found.".green());
        }
        std::process::exit(0);
    }

    // Output in requested format
    if format == "json" {
        let json_output: Vec<serde_json::Value> = unused
            .iter()
            .map(|(file_path, fixture_name)| {
                let relative_path = file_path
                    .strip_prefix(&canonical_path)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();
                serde_json::json!({
                    "file": relative_path,
                    "fixture": fixture_name
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_output).unwrap());
    } else {
        println!(
            "{} {} unused fixture(s):\n",
            "Found".red().bold(),
            unused.len()
        );

        for (file_path, fixture_name) in &unused {
            let relative_path = file_path
                .strip_prefix(&canonical_path)
                .unwrap_or(file_path)
                .to_string_lossy();
            println!(
                "  {} {} in {}",
                "•".red(),
                fixture_name.yellow(),
                relative_path.dimmed()
            );
        }

        println!(
            "\n{}",
            "Tip: Remove unused fixtures or add tests that use them.".dimmed()
        );
    }

    // Exit with code 1 to signal unused fixtures found (useful for CI)
    std::process::exit(1);
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

    let (service, socket) = LspService::new(|client| Backend::new(client, fixture_db.clone()));

    info!("LSP server ready");
    Server::new(stdin, stdout, socket).serve(service).await;
    // Note: serve() typically won't return - process exit is handled by shutdown()
}
