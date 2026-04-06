// =============================================================================
// connectors/electron_ipc.rs  —  Electron IPC connector
//
// Matches Electron IPC channel definitions and call sites across the main and
// renderer processes:
//
//   Main process handlers:
//     ipcMain.handle("channel", handler)
//     ipcMain.on("channel", handler)
//
//   Renderer invocations:
//     ipcRenderer.invoke("channel", ...args)
//     ipcRenderer.send("channel", ...args)
//
//   Context bridge exposures:
//     contextBridge.exposeInMainWorld("apiName", { ... })
//
// Channel names are matched by string equality.  Each main→renderer pair
// produces a `flow_edges` row with edge_type = 'electron_ipc', protocol = 'ipc'.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct IpcHandler {
    file_id: i64,
    line: u32,
    channel: String,
}

#[derive(Debug, Clone)]
struct IpcInvocation {
    file_id: i64,
    line: u32,
    channel: String,
}

// Fields are populated for observability / future use (e.g. linking bridge
// names to ipcRenderer call sites).
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ContextBridgeExposure {
    file_id: i64,
    line: u32,
    api_name: String,
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

fn build_ipc_main_regex() -> Regex {
    // Matches: ipcMain.handle("channel", ...) or ipcMain.on("channel", ...)
    Regex::new(
        r#"ipcMain\s*\.\s*(?:handle|on)\s*\(\s*(?:"(?P<ch1>[^"]+)"|'(?P<ch2>[^']+)'|`(?P<ch3>[^`]+)`)"#,
    )
    .expect("ipc_main regex is valid")
}

fn build_ipc_renderer_regex() -> Regex {
    // Matches: ipcRenderer.invoke("channel", ...) or ipcRenderer.send("channel", ...)
    Regex::new(
        r#"ipcRenderer\s*\.\s*(?:invoke|send)\s*\(\s*(?:"(?P<ch1>[^"]+)"|'(?P<ch2>[^']+)'|`(?P<ch3>[^`]+)`)"#,
    )
    .expect("ipc_renderer regex is valid")
}

fn build_context_bridge_regex() -> Regex {
    // Matches: contextBridge.exposeInMainWorld("apiName", ...)
    Regex::new(
        r#"contextBridge\s*\.\s*exposeInMainWorld\s*\(\s*(?:"(?P<n1>[^"]+)"|'(?P<n2>[^']+)')"#,
    )
    .expect("context_bridge regex is valid")
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn extract_channel_name(cap: &regex::Captures<'_>) -> Option<String> {
    cap.name("ch1")
        .or_else(|| cap.name("ch2"))
        .or_else(|| cap.name("ch3"))
        .map(|m| m.as_str().to_string())
}

fn query_ts_js_files(conn: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, path FROM files
         WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
    )?;
    let rows: rusqlite::Result<Vec<(i64, String)>> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
        .collect();
    rows
}

// ---------------------------------------------------------------------------
// Detection functions
// ---------------------------------------------------------------------------

fn find_ipc_handlers(conn: &Connection, project_root: &Path) -> Result<Vec<IpcHandler>> {
    let re = build_ipc_main_regex();

    let files = query_ts_js_files(conn)
        .context("Failed to query TS/JS files for ipcMain handlers")?;

    let mut handlers: Vec<IpcHandler> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable file");
                continue;
            }
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re.captures_iter(line_text) {
                if let Some(channel) = extract_channel_name(&cap) {
                    handlers.push(IpcHandler { file_id, line: line_no, channel });
                }
            }
        }
    }

    debug!(count = handlers.len(), "ipcMain handlers found");
    Ok(handlers)
}

fn find_ipc_invocations(conn: &Connection, project_root: &Path) -> Result<Vec<IpcInvocation>> {
    let re = build_ipc_renderer_regex();

    let files = query_ts_js_files(conn)
        .context("Failed to query TS/JS files for ipcRenderer calls")?;

    let mut invocations: Vec<IpcInvocation> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re.captures_iter(line_text) {
                if let Some(channel) = extract_channel_name(&cap) {
                    invocations.push(IpcInvocation { file_id, line: line_no, channel });
                }
            }
        }
    }

    debug!(count = invocations.len(), "ipcRenderer calls found");
    Ok(invocations)
}

fn find_context_bridge_exposures(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<ContextBridgeExposure>> {
    let re = build_context_bridge_regex();

    let files = query_ts_js_files(conn)
        .context("Failed to query TS/JS files for contextBridge exposures")?;

    let mut exposures: Vec<ContextBridgeExposure> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re.captures_iter(line_text) {
                let api_name = cap
                    .name("n1")
                    .or_else(|| cap.name("n2"))
                    .map(|m| m.as_str().to_string());
                if let Some(name) = api_name {
                    exposures.push(ContextBridgeExposure {
                        file_id,
                        line: line_no,
                        api_name: name,
                    });
                }
            }
        }
    }

    debug!(count = exposures.len(), "contextBridge exposures found");
    Ok(exposures)
}

// ---------------------------------------------------------------------------
// Linking
// ---------------------------------------------------------------------------

fn link_channels(
    conn: &Connection,
    handlers: &[IpcHandler],
    invocations: &[IpcInvocation],
) -> Result<u32> {
    let mut created: u32 = 0;

    for invocation in invocations {
        for handler in handlers.iter().filter(|h| h.channel == invocation.channel) {
            let result = conn.execute(
                "INSERT OR IGNORE INTO flow_edges (
                    source_file_id, source_line, source_symbol, source_language,
                    target_file_id, target_line, target_symbol, target_language,
                    edge_type, protocol, confidence
                 ) VALUES (
                    ?1, ?2, ?3, 'typescript',
                    ?4, ?5, ?6, 'typescript',
                    'electron_ipc', 'ipc', 0.90
                 )",
                rusqlite::params![
                    invocation.file_id,
                    invocation.line,
                    invocation.channel,
                    handler.file_id,
                    handler.line,
                    handler.channel,
                ],
            );

            match result {
                Ok(n) if n > 0 => created += 1,
                Ok(_) => {}
                Err(e) => {
                    debug!(
                        err = %e,
                        channel = %invocation.channel,
                        "Failed to insert electron_ipc flow_edge"
                    );
                }
            }
        }
    }

    info!(created, "Electron IPC: channel edges created");
    Ok(created)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run all Electron IPC detection passes and write results to the database.
pub fn connect(db: &Database, project_root: &Path) -> Result<()> {
    let conn = db.conn();

    let handlers = find_ipc_handlers(conn, project_root)
        .context("Electron IPC handler detection failed")?;

    let invocations = find_ipc_invocations(conn, project_root)
        .context("Electron IPC renderer call detection failed")?;

    let _exposures = find_context_bridge_exposures(conn, project_root)
        .context("Electron contextBridge detection failed")?;

    let edges = link_channels(conn, &handlers, &invocations)
        .context("Electron IPC channel linking failed")?;

    info!(
        handlers = handlers.len(),
        invocations = invocations.len(),
        edges,
        "Electron IPC connector complete"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "electron_ipc_tests.rs"]
mod tests;
