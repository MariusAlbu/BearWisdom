// =============================================================================
// lsp/registry.rs  —  per-language server command registry
//
// Maps Language → the OS command used to launch the server binary.
// `detect_installed` probes PATH at runtime so callers know which servers
// are actually available on the current machine.
// =============================================================================

use crate::lsp::types::Language;

// ---------------------------------------------------------------------------
// ServerEntry
// ---------------------------------------------------------------------------

/// A single entry in the server registry.
#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub language: Language,
    /// The binary name (looked up on PATH) or absolute path.
    pub command: String,
    /// Additional CLI arguments passed after the command.
    pub args: Vec<String>,
    /// Human-readable display name shown in the UI.
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// ServerRegistry
// ---------------------------------------------------------------------------

pub struct ServerRegistry {
    pub entries: Vec<ServerEntry>,
}

impl ServerRegistry {
    /// Build the registry with all well-known language-server entries.
    pub fn new() -> Self {
        let entries = vec![
            ServerEntry {
                language: Language::TypeScript,
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                display_name: "TypeScript Language Server".to_string(),
            },
            ServerEntry {
                language: Language::JavaScript,
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                display_name: "TypeScript Language Server (JS)".to_string(),
            },
            ServerEntry {
                language: Language::CSharp,
                command: "csharp-ls".to_string(),
                args: vec![],
                display_name: "csharp-ls".to_string(),
            },
            ServerEntry {
                language: Language::Rust,
                command: "rust-analyzer".to_string(),
                args: vec![],
                display_name: "rust-analyzer".to_string(),
            },
            ServerEntry {
                language: Language::Python,
                command: "pyright-langserver".to_string(),
                args: vec!["--stdio".to_string()],
                display_name: "Pyright Language Server".to_string(),
            },
            ServerEntry {
                language: Language::Go,
                command: "gopls".to_string(),
                args: vec!["serve".to_string()],
                display_name: "gopls".to_string(),
            },
            ServerEntry {
                language: Language::Java,
                command: "jdtls".to_string(),
                args: vec![],
                display_name: "Eclipse JDT Language Server".to_string(),
            },
            ServerEntry {
                language: Language::Cpp,
                command: "clangd".to_string(),
                args: vec![],
                display_name: "clangd".to_string(),
            },
        ];

        Self { entries }
    }

    /// Look up the entry for `lang`.  Returns the first matching entry.
    pub fn server_for(&self, lang: &Language) -> Option<&ServerEntry> {
        self.entries.iter().find(|e| &e.language == lang)
    }

    /// Return every entry whose binary can be found on PATH.
    ///
    /// Uses `where` on Windows, `which` on Unix.
    pub async fn detect_installed(&self) -> Vec<&ServerEntry> {
        let mut found = Vec::new();

        for entry in &self.entries {
            if Self::binary_exists(&entry.command).await {
                found.push(entry);
            }
        }

        found
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    async fn binary_exists(command: &str) -> bool {
        #[cfg(target_os = "windows")]
        let result = tokio::process::Command::new("where")
            .arg(command)
            .output()
            .await;

        #[cfg(not(target_os = "windows"))]
        let result = tokio::process::Command::new("which")
            .arg(command)
            .output()
            .await;

        result.map(|o| o.status.success()).unwrap_or(false)
    }
}

impl Default for ServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
