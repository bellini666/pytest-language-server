//! `LanguageServer` trait implementation for `Backend`.
//!
//! This was extracted from `main.rs` so that both the binary crate and the
//! library crate compile the impl, making `Backend` usable in integration
//! tests via `LspService::new`.

use std::sync::Arc;

use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::request::{GotoImplementationParams, GotoImplementationResponse};
use tower_lsp_server::ls_types::*;
use tower_lsp_server::LanguageServer;
use tracing::{error, info, warn};

use super::Backend;
use crate::config;

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
                let loaded_config = config::Config::load(&root_path);
                info!("Loaded config: {:?}", loaded_config);
                *self.config.write().await = loaded_config;

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
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::new("source.pytest-lsp"),
                            CodeActionKind::new("source.fixAll.pytest-lsp"),
                        ]),
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: None,
                        },
                        resolve_provider: None,
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        "\"".to_string(),
                        "(".to_string(),
                        ",".to_string(),
                    ]),
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

        // Register a file watcher for __init__.py create/delete events.
        // When package markers change, `file_path_to_module_path()` results
        // (captured in `FixtureDefinition::return_type_imports`) become stale,
        // so we re-analyze affected fixture files to refresh them.
        let watch_init_py = Registration {
            id: "watch-init-py".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: Some(
                serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                    watchers: vec![FileSystemWatcher {
                        glob_pattern: GlobPattern::String("**/__init__.py".to_string()),
                        kind: Some(WatchKind::Create | WatchKind::Delete),
                    }],
                })
                .unwrap(),
            ),
        };

        if let Err(e) = self.client.register_capability(vec![watch_init_py]).await {
            // Not fatal — file watching is best-effort.  The user can still
            // manually re-open fixture files to trigger re-analysis.
            info!(
                "Failed to register __init__.py file watcher (client may not support it): {}",
                e
            );
        }
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

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        // Re-analyze fixture files whose `return_type_imports` may have become
        // stale because an `__init__.py` was created or deleted, changing the
        // result of `file_path_to_module_path()`.
        for event in &params.changes {
            if event.typ != FileChangeType::CREATED && event.typ != FileChangeType::DELETED {
                continue;
            }

            let Some(init_path) = self.uri_to_path(&event.uri) else {
                continue;
            };

            // The __init__.py change affects the directory it lives in and
            // every directory below it.  Any fixture file at or under that
            // directory may produce a different module path now.
            let affected_dir = match init_path.parent() {
                Some(dir) => dir.to_path_buf(),
                None => continue,
            };

            let kind = if event.typ == FileChangeType::CREATED {
                "created"
            } else {
                "deleted"
            };
            info!(
                "__init__.py {} in {:?} — re-analyzing affected fixture files",
                kind, affected_dir
            );

            // Collect fixture files that live at or below the affected directory.
            let files_to_reanalyze: Vec<std::path::PathBuf> = self
                .fixture_db
                .file_definitions
                .iter()
                .filter(|entry| entry.key().starts_with(&affected_dir))
                .map(|entry| entry.key().clone())
                .collect();

            for file_path in files_to_reanalyze {
                if let Some(content) = self.fixture_db.get_file_content(&file_path) {
                    info!("Re-analyzing {:?} after __init__.py change", file_path);
                    self.fixture_db.analyze_file(file_path.clone(), &content);

                    // Re-publish diagnostics for the file if we have a cached URI.
                    if let Some(uri) = self.uri_cache.get(&file_path) {
                        self.publish_diagnostics_for_file(&uri, &file_path).await;
                    }
                }
            }
        }

        // Refresh inlay hints in case return types changed.
        if !params.changes.is_empty() {
            if let Err(e) = self.client.inlay_hint_refresh().await {
                info!(
                    "Inlay hint refresh after __init__.py change failed (client may not support it): {}",
                    e
                );
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
