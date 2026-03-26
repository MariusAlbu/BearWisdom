// =============================================================================
// lsp/types.rs  —  shared types for the LSP integration layer
// =============================================================================

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

/// The programming language a server handles.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    CSharp,
    TypeScript,
    JavaScript,
    Rust,
    Python,
    Go,
    Java,
    Cpp,
}

impl Language {
    /// Returns the LSP `languageId` string for this language.
    pub fn language_id(&self) -> &str {
        match self {
            Language::CSharp => "csharp",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Rust => "rust",
            Language::Python => "python",
            Language::Go => "go",
            Language::Java => "java",
            Language::Cpp => "cpp",
        }
    }

    /// Maps a file extension (without leading dot) to a Language, if known.
    pub fn from_extension(ext: &str) -> Option<Language> {
        match ext {
            "cs" => Some(Language::CSharp),
            "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::TypeScript),
            "js" | "mjs" | "cjs" => Some(Language::JavaScript),
            "jsx" => Some(Language::JavaScript),
            "rs" => Some(Language::Rust),
            "py" | "pyi" => Some(Language::Python),
            "go" => Some(Language::Go),
            "java" => Some(Language::Java),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h" | "c" => {
                Some(Language::Cpp)
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ServerState / ServerStatus
// ---------------------------------------------------------------------------

/// Lifecycle state of an LSP server process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    /// The process has not been started yet.
    Stopped,
    /// The process has been launched; the OS is still bringing it up.
    Starting,
    /// The process is running; the LSP `initialize` handshake is in progress.
    Initializing,
    /// The process is running and the LSP initialize handshake has completed.
    Ready,
    /// A graceful shutdown sequence has been initiated.
    ShuttingDown,
    /// The process crashed or was killed unexpectedly.
    Failed,
}

/// A snapshot of an LSP server's status — returned to callers of LspManager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub language: Language,
    pub state: ServerState,
    /// The server's display name, e.g. "rust-analyzer" or "typescript-language-server".
    pub server_name: String,
    /// Number of LSP requests sent to this server since it was started.
    pub request_count: u64,
}

// ---------------------------------------------------------------------------
// LSP protocol subset — positions, ranges, locations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

// ---------------------------------------------------------------------------
// Text document types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentItem {
    pub uri: String,
    pub language_id: String,
    pub version: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentPositionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

/// Full-document content change (TextDocumentSyncKind::Full).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentContentChangeEvent {
    pub text: String,
}

// ---------------------------------------------------------------------------
// Notification params
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidOpenTextDocumentParams {
    pub text_document: TextDocumentItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidChangeTextDocumentParams {
    pub text_document: VersionedTextDocumentIdentifier,
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidCloseTextDocumentParams {
    pub text_document: TextDocumentIdentifier,
}

// ---------------------------------------------------------------------------
// Initialize request / response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub process_id: Option<u32>,
    pub root_uri: Option<String>,
    pub capabilities: ClientCapabilities,
}

/// We request no optional capabilities — servers will use their defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_provider: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references_provider: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover_provider: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_document_sync: Option<TextDocumentSyncOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentSyncOptions {
    /// Whether open/close notifications are sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_close: Option<bool>,
    /// 1 = Full, 2 = Incremental.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<u8>,
}

// ---------------------------------------------------------------------------
// Hover response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverResult {
    pub contents: HoverContents,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HoverContents {
    /// Plain string.
    String(String),
    /// Markdown or plaintext markup content.
    MarkupContent { kind: String, value: String },
    /// Array of MarkedString or MarkupContent values.
    Array(Vec<serde_json::Value>),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
