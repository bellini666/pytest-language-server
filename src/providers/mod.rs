//! LSP providers module.
//!
//! This module contains the Backend struct and LSP protocol handlers organized by provider type.

pub mod call_hierarchy;
pub mod code_action;
pub mod code_lens;
pub mod completion;
pub mod definition;
pub mod diagnostics;
pub mod document_symbol;
pub mod hover;
pub mod implementation;
pub mod inlay_hint;
pub mod references;
pub mod workspace_symbol;

use crate::config::Config;
use crate::fixtures::FixtureDatabase;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::Client;
use tracing::warn;

/// The LSP Backend struct containing server state.
pub struct Backend {
    pub client: Client,
    pub fixture_db: Arc<FixtureDatabase>,
    /// The canonical workspace root path (resolved symlinks)
    pub workspace_root: Arc<tokio::sync::RwLock<Option<PathBuf>>>,
    /// The original workspace root path as provided by the client (may contain symlinks)
    pub original_workspace_root: Arc<tokio::sync::RwLock<Option<PathBuf>>>,
    /// Handle to the background workspace scan task, used for cancellation on shutdown
    pub scan_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Cache mapping canonical paths to original URIs from the client
    /// This ensures we respond with URIs the client recognizes
    pub uri_cache: Arc<DashMap<PathBuf, Uri>>,
    /// Configuration loaded from pyproject.toml
    pub config: Arc<tokio::sync::RwLock<Config>>,
}

impl Backend {
    /// Create a new Backend instance
    pub fn new(client: Client, fixture_db: Arc<FixtureDatabase>) -> Self {
        Self {
            client,
            fixture_db,
            workspace_root: Arc::new(tokio::sync::RwLock::new(None)),
            original_workspace_root: Arc::new(tokio::sync::RwLock::new(None)),
            scan_task: Arc::new(tokio::sync::Mutex::new(None)),
            uri_cache: Arc::new(DashMap::new()),
            config: Arc::new(tokio::sync::RwLock::new(Config::default())),
        }
    }

    /// Convert URI to PathBuf with error logging
    /// Canonicalizes the path to handle symlinks (e.g., /var -> /private/var on macOS)
    pub fn uri_to_path(&self, uri: &Uri) -> Option<PathBuf> {
        match uri.to_file_path() {
            Some(path) => {
                // Canonicalize to match how paths are stored in FixtureDatabase
                // This handles symlinks like /var -> /private/var on macOS
                let path = path.to_path_buf();
                Some(path.canonicalize().unwrap_or(path))
            }
            None => {
                warn!("Failed to convert URI to file path: {:?}", uri);
                None
            }
        }
    }

    /// Convert PathBuf to URI with error logging
    /// First checks the URI cache for a previously seen URI, then falls back to creating one
    pub fn path_to_uri(&self, path: &std::path::Path) -> Option<Uri> {
        // First, check if we have a cached URI for this path
        // This ensures we use the same URI format the client originally sent
        if let Some(cached_uri) = self.uri_cache.get(path) {
            return Some(cached_uri.clone());
        }

        // For paths not in cache, we need to handle macOS symlink issue
        // where /var is a symlink to /private/var
        // The client sends /var/... but we store /private/var/...
        // So we need to strip /private prefix when building URIs
        let path_to_use: Option<PathBuf> = if cfg!(target_os = "macos") {
            path.to_str().and_then(|path_str| {
                if path_str.starts_with("/private/var/") || path_str.starts_with("/private/tmp/") {
                    Some(PathBuf::from(path_str.replacen("/private", "", 1)))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let final_path = path_to_use.as_deref().unwrap_or(path);

        // Fall back to creating a new URI from the path
        match Uri::from_file_path(final_path) {
            Some(uri) => Some(uri),
            None => {
                warn!("Failed to convert path to URI: {:?}", path);
                None
            }
        }
    }

    /// Convert LSP position (0-based line) to internal representation (1-based line)
    pub fn lsp_line_to_internal(line: u32) -> usize {
        (line + 1) as usize
    }

    /// Convert internal line (1-based) to LSP position (0-based)
    pub fn internal_line_to_lsp(line: usize) -> u32 {
        line.saturating_sub(1) as u32
    }

    /// Create a Range from start and end positions
    pub fn create_range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
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
    pub fn create_point_range(line: u32, character: u32) -> Range {
        Self::create_range(line, character, line, character)
    }

    /// Format fixture documentation for display (used in both hover and completions)
    pub fn format_fixture_documentation(
        fixture: &crate::fixtures::FixtureDefinition,
        workspace_root: Option<&PathBuf>,
    ) -> String {
        let mut content = String::new();

        // Calculate relative path from workspace root
        let relative_path = if let Some(root) = workspace_root {
            fixture
                .file_path
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    fixture
                        .file_path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                })
        } else {
            fixture
                .file_path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("unknown")
                .to_string()
        };

        // Add "from" line with relative path
        content.push_str(&format!("**from** `{}`\n", relative_path));

        // Add code block with fixture signature
        let return_annotation = if let Some(ref ret_type) = &fixture.return_type {
            format!(" -> {}", ret_type)
        } else {
            String::new()
        };

        content.push_str(&format!(
            "```python\n@pytest.fixture\ndef {}(...){}:\n```",
            fixture.name, return_annotation
        ));

        // Add docstring if present
        if let Some(ref docstring) = fixture.docstring {
            content.push_str("\n\n---\n\n");
            content.push_str(docstring);
        }

        content
    }
}
