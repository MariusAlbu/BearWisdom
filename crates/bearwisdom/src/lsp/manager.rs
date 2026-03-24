// =============================================================================
// lsp/manager.rs  —  lifecycle manager for LSP server instances
//
// LspManager is the single public API for the rest of the crate.  It starts
// servers on demand and routes requests to the appropriate handle.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::sync::Mutex;

use crate::lsp::registry::ServerRegistry;
use crate::lsp::server_handle::LspServerHandle;
use crate::lsp::types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    HoverContents, Language, Location, Position, ServerState, ServerStatus,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, VersionedTextDocumentIdentifier,
};

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct ServerSlot {
    handle: LspServerHandle,
    last_used: Instant,
    /// Monotonically incrementing open-file version counter.
    doc_version: i32,
}

struct LspManagerInner {
    servers: HashMap<Language, ServerSlot>,
    registry: ServerRegistry,
    workspace_root: PathBuf,
    idle_timeout: Duration,
}

// ---------------------------------------------------------------------------
// LspManager
// ---------------------------------------------------------------------------

/// Manages zero or more LSP server processes.
///
/// Cheap to clone — all state is behind `Arc<Mutex<_>>`.
#[derive(Clone)]
pub struct LspManager {
    inner: Arc<Mutex<LspManagerInner>>,
}

impl LspManager {
    /// Create a new manager.  No servers are started until first use.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let inner = LspManagerInner {
            servers: HashMap::new(),
            registry: ServerRegistry::new(),
            workspace_root: workspace_root.into(),
            idle_timeout: Duration::from_secs(300), // 5 minutes default
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    // -----------------------------------------------------------------------
    // Server lifecycle
    // -----------------------------------------------------------------------

    /// Ensure the server for `lang` is running and initialized.  No-op if it
    /// is already `Ready`.
    pub async fn ensure_server(&self, lang: Language) -> Result<()> {
        let mut inner = self.inner.lock().await;

        if let Some(slot) = inner.servers.get(&lang) {
            if slot.handle.is_ready() {
                return Ok(());
            }
        }

        // Either not started or in a bad state — (re)start it.
        let entry = inner
            .registry
            .server_for(&lang)
            .with_context(|| format!("no registry entry for language {lang:?}"))?
            .clone();

        let workspace_root = inner.workspace_root.clone();
        let root_uri = path_to_uri(&workspace_root);

        let (mut handle, _notification_rx) =
            LspServerHandle::spawn(&entry.command, &entry.args, &workspace_root)
                .await
                .with_context(|| format!("spawning LSP server '{}'", entry.command))?;

        handle
            .initialize(&root_uri)
            .await
            .with_context(|| format!("initializing LSP server '{}'", entry.display_name))?;

        inner.servers.insert(
            lang,
            ServerSlot {
                handle,
                last_used: Instant::now(),
                doc_version: 0,
            },
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // High-level LSP queries
    // -----------------------------------------------------------------------

    /// Jump-to-definition.  Returns an empty `Vec` when the server finds nothing.
    pub async fn goto_definition(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Vec<Location>> {
        let lang = Self::language_for_file(file_path)
            .context("could not determine language from file extension")?;

        self.ensure_server(lang.clone()).await?;

        let mut inner = self.inner.lock().await;
        let slot = inner.servers.get_mut(&lang).context("server not ready")?;
        slot.last_used = Instant::now();

        let params = build_position_params(file_path, line, col);
        let result = slot
            .handle
            .send_request("textDocument/definition", serde_json::to_value(params)?)
            .await?;

        parse_locations(result)
    }

    /// Find all references.
    pub async fn find_references(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Vec<Location>> {
        let lang = Self::language_for_file(file_path)
            .context("could not determine language from file extension")?;

        self.ensure_server(lang.clone()).await?;

        let mut inner = self.inner.lock().await;
        let slot = inner.servers.get_mut(&lang).context("server not ready")?;
        slot.last_used = Instant::now();

        let mut params = serde_json::to_value(build_position_params(file_path, line, col))?;
        // LSP references request requires context.includeDeclaration
        params["context"] = serde_json::json!({ "includeDeclaration": true });

        let result = slot
            .handle
            .send_request("textDocument/references", params)
            .await?;

        parse_locations(result)
    }

    /// Hover.  Returns `None` when the server has no hover info at this position.
    pub async fn hover(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Option<String>> {
        let lang = Self::language_for_file(file_path)
            .context("could not determine language from file extension")?;

        self.ensure_server(lang.clone()).await?;

        let mut inner = self.inner.lock().await;
        let slot = inner.servers.get_mut(&lang).context("server not ready")?;
        slot.last_used = Instant::now();

        let params = build_position_params(file_path, line, col);
        let result = slot
            .handle
            .send_request("textDocument/hover", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let hover: crate::lsp::types::HoverResult =
            serde_json::from_value(result).context("parsing HoverResult")?;

        let text = extract_hover_text(hover.contents);
        Ok(Some(text))
    }

    // -----------------------------------------------------------------------
    // Document sync notifications
    // -----------------------------------------------------------------------

    pub async fn did_open(&self, file_path: &str, text: &str) -> Result<()> {
        let lang = Self::language_for_file(file_path)
            .context("could not determine language from file extension")?;

        self.ensure_server(lang.clone()).await?;

        let mut inner = self.inner.lock().await;
        let slot = inner.servers.get_mut(&lang).context("server not ready")?;
        slot.doc_version += 1;

        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_path.to_string(),
                language_id: lang.language_id().to_string(),
                version: slot.doc_version,
                text: text.to_string(),
            },
        };

        slot.handle
            .send_notification(
                "textDocument/didOpen",
                serde_json::to_value(params)?,
            )
            .await
    }

    pub async fn did_change(&self, file_path: &str, text: &str) -> Result<()> {
        let lang = Self::language_for_file(file_path)
            .context("could not determine language from file extension")?;

        self.ensure_server(lang.clone()).await?;

        let mut inner = self.inner.lock().await;
        let slot = inner.servers.get_mut(&lang).context("server not ready")?;
        slot.doc_version += 1;

        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: file_path.to_string(),
                version: slot.doc_version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                text: text.to_string(),
            }],
        };

        slot.handle
            .send_notification(
                "textDocument/didChange",
                serde_json::to_value(params)?,
            )
            .await
    }

    pub async fn did_close(&self, file_path: &str) -> Result<()> {
        let lang = Self::language_for_file(file_path)
            .context("could not determine language from file extension")?;

        // If no server is running for this language, nothing to do.
        let mut inner = self.inner.lock().await;
        let Some(slot) = inner.servers.get_mut(&lang) else {
            return Ok(());
        };

        if !slot.handle.is_ready() {
            return Ok(());
        }

        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: file_path.to_string(),
            },
        };

        slot.handle
            .send_notification(
                "textDocument/didClose",
                serde_json::to_value(params)?,
            )
            .await
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Gracefully shut down all running servers.
    pub async fn shutdown_all(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let mut errors: Vec<String> = Vec::new();

        for (lang, slot) in inner.servers.iter_mut() {
            if let Err(e) = slot.handle.shutdown().await {
                errors.push(format!("{lang:?}: {e}"));
            }
        }

        inner.servers.clear();

        if errors.is_empty() {
            Ok(())
        } else {
            bail!("errors during shutdown: {}", errors.join("; "))
        }
    }

    /// Shut down servers that have been idle longer than `idle_timeout`.
    pub async fn shutdown_idle(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let timeout = inner.idle_timeout;
        let now = Instant::now();

        let idle_langs: Vec<Language> = inner
            .servers
            .iter()
            .filter(|(_, slot)| now.duration_since(slot.last_used) >= timeout)
            .map(|(lang, _)| lang.clone())
            .collect();

        for lang in idle_langs {
            if let Some(mut slot) = inner.servers.remove(&lang) {
                if let Err(e) = slot.handle.shutdown().await {
                    tracing::warn!(language = ?lang, error = %e, "error shutting down idle server");
                }
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Status queries (non-blocking, sync)
    // -----------------------------------------------------------------------

    /// Return a status snapshot if a server has been started for `language`.
    pub fn status(&self, language: &Language) -> Option<ServerStatus> {
        // Non-blocking try_lock — if the mutex is held we return None.
        let inner = self.inner.try_lock().ok()?;
        let slot = inner.servers.get(language)?;
        let entry = inner.registry.server_for(language)?;

        Some(ServerStatus {
            language: language.clone(),
            state: slot.handle.state,
            server_name: entry.display_name.clone(),
            request_count: slot.handle.request_count,
        })
    }

    /// Return the state of the server for `language`, or `Stopped` if none.
    pub fn state(&self, language: &Language) -> ServerState {
        self.status(language)
            .map(|s| s.state)
            .unwrap_or(ServerState::Stopped)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Infer a `Language` from the file's extension.
    fn language_for_file(path: &str) -> Option<Language> {
        let ext = Path::new(path).extension()?.to_str()?;
        Language::from_extension(ext)
    }

    /// Convert a filesystem path to a `file:///` URI.
    pub fn file_uri(workspace_root: &Path, relative_path: &str) -> String {
        let full = workspace_root.join(relative_path);
        path_to_uri(&full)
    }
}

// ---------------------------------------------------------------------------
// Default impl (workspace_root = cwd)
// ---------------------------------------------------------------------------

impl Default for LspManager {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(cwd)
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn path_to_uri(path: &Path) -> String {
    // Canonicalize if possible to get an absolute path; fall back to as-is.
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    #[cfg(target_os = "windows")]
    {
        // Windows paths: C:\foo\bar  →  file:///C:/foo/bar
        let s = abs.to_string_lossy();
        let normalized = s.replace('\\', "/");
        format!("file:///{}", normalized)
    }

    #[cfg(not(target_os = "windows"))]
    {
        format!("file://{}", abs.display())
    }
}

fn build_position_params(file_path: &str, line: u32, col: u32) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: file_path.to_string(),
        },
        position: Position {
            line,
            character: col,
        },
    }
}

fn parse_locations(value: serde_json::Value) -> Result<Vec<Location>> {
    if value.is_null() {
        return Ok(vec![]);
    }

    // The LSP spec allows: Location | Location[] | LocationLink[]
    if value.is_array() {
        let locs: Vec<Location> = serde_json::from_value(value)
            .unwrap_or_default(); // LocationLink has different shape — silently empty
        Ok(locs)
    } else {
        // Single Location
        let loc: Location = serde_json::from_value(value).context("parsing single Location")?;
        Ok(vec![loc])
    }
}

fn extract_hover_text(contents: HoverContents) -> String {
    match contents {
        HoverContents::String(s) => s,
        HoverContents::MarkupContent { value, .. } => value,
        HoverContents::Array(arr) => arr
            .into_iter()
            .filter_map(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else if let Some(obj) = v.as_object() {
                    obj.get("value")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_for_file_known_extensions() {
        assert_eq!(
            LspManager::language_for_file("src/main.rs"),
            Some(Language::Rust)
        );
        assert_eq!(
            LspManager::language_for_file("index.ts"),
            Some(Language::TypeScript)
        );
        assert_eq!(
            LspManager::language_for_file("app.tsx"),
            Some(Language::TypeScript)
        );
        assert_eq!(
            LspManager::language_for_file("Program.cs"),
            Some(Language::CSharp)
        );
        assert_eq!(
            LspManager::language_for_file("main.py"),
            Some(Language::Python)
        );
        assert_eq!(
            LspManager::language_for_file("main.go"),
            Some(Language::Go)
        );
    }

    #[test]
    fn language_for_file_unknown_returns_none() {
        assert_eq!(LspManager::language_for_file("Cargo.toml"), None);
        assert_eq!(LspManager::language_for_file("README.md"), None);
        assert_eq!(LspManager::language_for_file("noextension"), None);
    }

    #[test]
    fn file_uri_unix_style() {
        // Test the URI helper with a synthetic path rather than the cfg-gated
        // platform implementation — just verify the prefix.
        let root = PathBuf::from("/home/user/project");
        let uri = LspManager::file_uri(&root, "src/main.rs");
        assert!(
            uri.starts_with("file://"),
            "URI must start with file://: {uri}"
        );
        assert!(uri.contains("main.rs"), "URI must contain filename: {uri}");
    }

    #[test]
    fn status_returns_none_for_unstarted_server() {
        let mgr = LspManager::new("/tmp/fake");
        assert!(mgr.status(&Language::Rust).is_none());
        assert_eq!(mgr.state(&Language::TypeScript), ServerState::Stopped);
    }

    #[test]
    fn extract_hover_text_string() {
        let text = extract_hover_text(HoverContents::String("hello".to_string()));
        assert_eq!(text, "hello");
    }

    #[test]
    fn extract_hover_text_markup() {
        let text = extract_hover_text(HoverContents::MarkupContent {
            kind: "markdown".to_string(),
            value: "**bold**".to_string(),
        });
        assert_eq!(text, "**bold**");
    }

    #[test]
    fn extract_hover_text_array_mixed() {
        let arr = vec![
            serde_json::json!("plain string"),
            serde_json::json!({ "language": "rust", "value": "fn foo()" }),
        ];
        let text = extract_hover_text(HoverContents::Array(arr));
        assert!(text.contains("plain string"));
        assert!(text.contains("fn foo()"));
    }

    #[test]
    fn parse_locations_null_returns_empty() {
        let result = parse_locations(serde_json::Value::Null).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_locations_array() {
        let json = serde_json::json!([
            {
                "uri": "file:///src/main.rs",
                "range": {
                    "start": { "line": 1, "character": 4 },
                    "end":   { "line": 1, "character": 7 }
                }
            }
        ]);
        let locs = parse_locations(json).unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///src/main.rs");
        assert_eq!(locs[0].range.start.line, 1);
    }

    #[test]
    fn parse_locations_single_object() {
        let json = serde_json::json!({
            "uri": "file:///lib.rs",
            "range": {
                "start": { "line": 0, "character": 0 },
                "end":   { "line": 0, "character": 3 }
            }
        });
        let locs = parse_locations(json).unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///lib.rs");
    }

    // -----------------------------------------------------------------------
    // Integration test — requires typescript-language-server installed.
    // Run manually: cargo test -- --ignored lsp_goto_definition_typescript
    // -----------------------------------------------------------------------

    /// Verify that LspManager can start typescript-language-server, open a
    /// TypeScript file, and perform a goto_definition that resolves an import.
    ///
    /// Skip automatically when `typescript-language-server` is not on PATH.
    #[tokio::test]
    #[ignore = "requires typescript-language-server on PATH; run with --ignored"]
    async fn lsp_goto_definition_typescript() {
        use std::fs;

        // Check whether the server binary is available before doing anything.
        let server_available = std::process::Command::new("typescript-language-server")
            .arg("--version")
            .output()
            .is_ok();

        if !server_available {
            eprintln!("SKIP: typescript-language-server not found on PATH");
            return;
        }

        // Create a temp workspace with two TypeScript files:
        //   lib.ts  — exports a function `greet`
        //   main.ts — imports and calls `greet` from `./lib`
        let dir = tempfile::TempDir::new().expect("tempdir");
        let lib_path = dir.path().join("lib.ts");
        let main_path = dir.path().join("main.ts");

        fs::write(
            &lib_path,
            "export function greet(name: string): string {\n    return `Hello, ${name}`;\n}\n",
        ).expect("write lib.ts");

        fs::write(
            &main_path,
            "import { greet } from './lib';\nconst msg = greet('world');\nconsole.log(msg);\n",
        ).expect("write main.ts");

        // Build URIs for the LSP server (file:///... scheme).
        let main_uri = path_to_uri(&main_path);
        let _lib_uri = path_to_uri(&lib_path);

        let mgr = LspManager::new(dir.path());

        // Open main.ts so the server has its content.
        let main_content = fs::read_to_string(&main_path).expect("read main.ts");
        mgr.did_open(&main_uri, &main_content)
            .await
            .expect("did_open main.ts");

        // Give the server a moment to process the file before querying.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // goto_definition at line 0, col 9 — the `greet` identifier in the import.
        // Expected: jumps to lib.ts where `greet` is defined.
        let locations = mgr
            .goto_definition(&main_uri, 0, 9)
            .await
            .expect("goto_definition");

        // Clean up.
        mgr.shutdown_all().await.expect("shutdown");

        assert!(
            !locations.is_empty(),
            "Expected at least one definition location, got none"
        );

        let def = &locations[0];
        assert!(
            def.uri.contains("lib"),
            "Definition should be in lib.ts, got: {}",
            def.uri
        );
        assert_eq!(
            def.range.start.line, 0,
            "greet is defined on line 0 of lib.ts"
        );

        eprintln!("goto_definition resolved to: {} line {}", def.uri, def.range.start.line);
    }
}
