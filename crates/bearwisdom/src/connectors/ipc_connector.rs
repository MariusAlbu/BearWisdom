// =============================================================================
// connectors/ipc_connector.rs — IPC connector (new architecture)
//
// Covers Tauri IPC (command + event) and Electron IPC (channel-based).
// Each framework gets its own connector instance since their detection
// patterns are completely different.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// Tauri IPC
// ===========================================================================

pub struct TauriIpcConnector;

impl Connector for TauriIpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "tauri_ipc",
            protocols: &[Protocol::Ipc],
            languages: &["rust", "typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.rust_crates.contains("tauri")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        use super::tauri_ipc;

        let mut points = Vec::new();

        // --- Commands: Rust handlers (stop) + TS invoke calls (start) ---
        let commands = tauri_ipc::find_tauri_commands(conn, project_root)
            .context("Tauri command detection failed")?;

        for cmd in &commands {
            points.push(ConnectionPoint {
                file_id: cmd.file_id,
                symbol_id: cmd.symbol_id,
                line: cmd.line,
                protocol: Protocol::Ipc,
                direction: FlowDirection::Stop,
                key: cmd.command_name.clone(),
                method: String::new(),
                framework: "tauri".to_string(),
                metadata: None,
            });
        }

        let calls = tauri_ipc::find_invoke_calls(conn, project_root)
            .context("Tauri invoke call detection failed")?;

        for call in &calls {
            points.push(ConnectionPoint {
                file_id: call.file_id,
                symbol_id: None,
                line: call.line,
                protocol: Protocol::Ipc,
                direction: FlowDirection::Start,
                key: call.command_name.clone(),
                method: String::new(),
                framework: "tauri".to_string(),
                metadata: None,
            });
        }

        // --- Events: Rust emit (start) + TS listen (stop) ---
        // Reuse the public event detection if available, otherwise skip.
        // The existing link_tauri_events does its own detect+link; we replicate
        // the detection half here.
        extract_tauri_events(conn, project_root, &mut points)?;

        Ok(points)
    }
}

fn extract_tauri_events(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let re_emit = regex::Regex::new(
        r#"\.emit\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)')"#,
    )
    .expect("emit regex");
    let re_listen = regex::Regex::new(
        r#"(?:\.\s*)?listen\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
    )
    .expect("listen regex");

    // Rust emit sites → Start
    scan_files_for_pattern(conn, project_root, "rust", &re_emit, FlowDirection::Start, "tauri", out)?;

    // TS/JS listen sites → Stop
    for lang in &["typescript", "tsx", "javascript", "jsx"] {
        scan_files_for_pattern(conn, project_root, lang, &re_listen, FlowDirection::Stop, "tauri", out)?;
    }

    Ok(())
}

// ===========================================================================
// Electron IPC
// ===========================================================================

pub struct ElectronIpcConnector;

impl Connector for ElectronIpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "electron_ipc",
            protocols: &[Protocol::Ipc],
            languages: &["typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("electron")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let re_main = regex::Regex::new(
            r#"ipcMain\s*\.\s*(?:handle|on)\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
        )
        .expect("ipcMain regex");
        let re_renderer = regex::Regex::new(
            r#"ipcRenderer\s*\.\s*(?:invoke|send)\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
        )
        .expect("ipcRenderer regex");

        let mut points = Vec::new();

        // ipcMain.handle/on → Stop (handler side)
        for lang in &["typescript", "tsx", "javascript", "jsx"] {
            scan_files_for_pattern(conn, project_root, lang, &re_main, FlowDirection::Stop, "electron", &mut points)?;
        }

        // ipcRenderer.invoke/send → Start (caller side)
        for lang in &["typescript", "tsx", "javascript", "jsx"] {
            scan_files_for_pattern(conn, project_root, lang, &re_renderer, FlowDirection::Start, "electron", &mut points)?;
        }

        Ok(points)
    }
}

// ===========================================================================
// Shared helper
// ===========================================================================

/// Scan all files of a given language for a regex pattern that captures a name
/// (via named groups name1/name2/name3) and emit ConnectionPoints.
fn scan_files_for_pattern(
    conn: &Connection,
    project_root: &Path,
    language: &str,
    re: &regex::Regex,
    direction: FlowDirection,
    framework: &str,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = ?1")
        .context("Failed to prepare file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([language], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to query files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect file rows")?;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re.captures_iter(line_text) {
                let name = cap
                    .name("name1")
                    .or_else(|| cap.name("name2"))
                    .or_else(|| cap.name("name3"))
                    .map(|m| m.as_str().to_string());

                if let Some(key) = name {
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Ipc,
                        direction,
                        key,
                        method: String::new(),
                        framework: framework.to_string(),
                        metadata: None,
                    });
                }
            }
        }
    }

    Ok(())
}
