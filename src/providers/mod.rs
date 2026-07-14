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
mod language_server;
pub mod references;
pub mod rename;
pub mod workspace_symbol;

use crate::config::Config;
use crate::fixtures::FixtureDatabase;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::Client;
use tracing::warn;

/// Convert a UTF-16 column to a byte offset within `line`.
/// Columns past the end of the line clamp to the line's byte length.
pub(crate) fn utf16_col_to_byte(line: &str, utf16_col: usize) -> usize {
    if line.is_ascii() {
        return utf16_col.min(line.len());
    }
    let mut units = 0usize;
    for (byte_idx, ch) in line.char_indices() {
        if units >= utf16_col {
            return byte_idx;
        }
        units += ch.len_utf16();
    }
    line.len()
}

/// Convert a byte offset within `line` to a UTF-16 column.
/// Offsets past the end of the line clamp to the line's UTF-16 length.
pub(crate) fn byte_col_to_utf16(line: &str, byte_col: usize) -> usize {
    if line.is_ascii() {
        return byte_col.min(line.len());
    }
    let mut units = 0usize;
    for (byte_idx, ch) in line.char_indices() {
        if byte_idx >= byte_col {
            break;
        }
        units += ch.len_utf16();
    }
    units
}

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
    /// Whether the client uses UTF-16 position encoding (the LSP default).
    /// Set to false during initialize when the client supports UTF-8, in which
    /// case our internal byte columns can be sent as-is.
    pub client_utf16: Arc<AtomicBool>,
    /// Per-file change generation counters used to debounce diagnostics
    /// publishing while the user is typing.
    pub change_generation: Arc<DashMap<PathBuf, u64>>,
}

impl Clone for Backend {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            fixture_db: Arc::clone(&self.fixture_db),
            workspace_root: Arc::clone(&self.workspace_root),
            original_workspace_root: Arc::clone(&self.original_workspace_root),
            scan_task: Arc::clone(&self.scan_task),
            uri_cache: Arc::clone(&self.uri_cache),
            config: Arc::clone(&self.config),
            client_utf16: Arc::clone(&self.client_utf16),
            change_generation: Arc::clone(&self.change_generation),
        }
    }
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
            client_utf16: Arc::new(AtomicBool::new(true)),
            change_generation: Arc::new(DashMap::new()),
        }
    }

    /// Get the text of a 1-based line in a file (without the trailing newline).
    fn line_text(&self, file_path: &std::path::Path, internal_line: usize) -> Option<String> {
        let content = self.fixture_db.get_file_content(file_path)?;
        let index = self.fixture_db.get_line_index(file_path, &content);
        let start = *index.get(internal_line.checked_sub(1)?)?;
        let end = index.get(internal_line).copied().unwrap_or(content.len());
        Some(
            content[start..end]
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        )
    }

    /// Convert an inbound LSP position's character to an internal byte column.
    pub(crate) fn to_byte_col(&self, file_path: &std::path::Path, position: Position) -> u32 {
        if !self.client_utf16.load(Ordering::Relaxed) {
            return position.character;
        }
        match self.line_text(file_path, Self::lsp_line_to_internal(position.line)) {
            Some(line) => utf16_col_to_byte(&line, position.character as usize) as u32,
            None => position.character,
        }
    }

    /// Convert an internal byte column to an outbound LSP character.
    pub(crate) fn to_lsp_col(
        &self,
        file_path: &std::path::Path,
        internal_line: usize,
        byte_col: usize,
    ) -> u32 {
        if !self.client_utf16.load(Ordering::Relaxed) {
            return byte_col as u32;
        }
        match self.line_text(file_path, internal_line) {
            Some(line) => byte_col_to_utf16(&line, byte_col) as u32,
            None => byte_col as u32,
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

        // For paths not in cache, we need to handle the macOS symlink issue
        // where /var, /tmp, /etc are symlinks into /private. The client sends
        // /var/... but we store the canonical /private/var/..., so strip the
        // /private prefix when the stripped path resolves back to the same file.
        let path_to_use: Option<PathBuf> = if cfg!(target_os = "macos") {
            path.to_str().and_then(|path_str| {
                let stripped = path_str.strip_prefix("/private")?;
                if !stripped.starts_with('/') {
                    return None;
                }
                let candidate = PathBuf::from(stripped);
                (candidate.canonicalize().ok()? == *path).then_some(candidate)
            })
        } else if cfg!(target_os = "windows") {
            // Strip Windows extended-length path prefix (\\?\) which is added by canonicalize()
            // This prefix causes Uri::from_file_path() to produce malformed URIs
            path.to_str()
                .and_then(|path_str| path_str.strip_prefix(r"\\?\"))
                .map(PathBuf::from)
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

#[cfg(test)]
mod tests {
    use super::{byte_col_to_utf16, utf16_col_to_byte};

    #[test]
    fn test_utf16_byte_conversion_ascii() {
        let line = "def test(fixture):";
        assert_eq!(utf16_col_to_byte(line, 9), 9);
        assert_eq!(byte_col_to_utf16(line, 9), 9);
        // Clamps past end of line.
        assert_eq!(utf16_col_to_byte(line, 100), line.len());
        assert_eq!(byte_col_to_utf16(line, 100), line.len());
    }

    #[test]
    fn test_utf16_byte_conversion_bmp() {
        // "é" is 2 bytes in UTF-8 but 1 UTF-16 code unit.
        let line = "x = 'é'; fixture";
        assert_eq!(utf16_col_to_byte(line, 9), 10);
        assert_eq!(byte_col_to_utf16(line, 10), 9);
    }

    #[test]
    fn test_utf16_byte_conversion_astral() {
        // "🎉" is 4 bytes in UTF-8 and 2 UTF-16 code units (surrogate pair);
        // it starts at byte 5 / UTF-16 unit 5 and ends at byte 9 / unit 7.
        let line = "s = '🎉'; f";
        assert_eq!(utf16_col_to_byte(line, 7), 9);
        assert_eq!(byte_col_to_utf16(line, 9), 7);
        // "f" is at byte 12 / UTF-16 unit 10.
        assert_eq!(utf16_col_to_byte(line, 10), 12);
        assert_eq!(byte_col_to_utf16(line, 12), 10);
        // A column inside the surrogate pair snaps to the next boundary.
        assert_eq!(utf16_col_to_byte(line, 6), 9);
    }
}
