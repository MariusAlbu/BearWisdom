// =============================================================================
// connectors/tauri_ipc.rs  —  Tauri IPC connector
//
// Matches Tauri command definitions (Rust) to invoke() call sites (TypeScript),
// and Tauri event emitters (Rust app.emit / window.emit) to event listeners
// (TypeScript listen / appWindow.listen).
//
// Command matching:
//   Rust:       #[tauri::command] or #[command] attribute before fn definitions
//   TypeScript: invoke("command_name", ...) or invoke<T>("command_name", ...)
//
// Event matching:
//   Rust:       app.emit("event-name", payload) or window.emit("event-name", payload)
//   TypeScript: listen("event-name", handler) or appWindow.listen("event-name", ...)
//
// All detection is regex-based.  Line-by-line scanning handles the attribute
// case (attribute on line N, fn declaration on line N+1) via a look-ahead
// carried in a `pending_command` bool.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A `#[tauri::command]` attributed function in a Rust file.
#[derive(Debug, Clone)]
pub struct TauriCommand {
    /// `symbols.id` if the function was indexed as a symbol.
    pub symbol_id: Option<i64>,
    /// The Rust function name — also the IPC command name as Tauri exposes it
    /// by default (snake_case → snake_case; Tauri does not rename by default).
    pub command_name: String,
    /// `files.id` of the containing Rust file.
    pub file_id: i64,
    /// 1-based line of the `fn` declaration.
    pub line: u32,
}

/// An `invoke("command_name")` call site in a TypeScript / JavaScript file.
#[derive(Debug, Clone)]
pub struct InvokeCall {
    /// `files.id` of the containing TS/JS file.
    pub file_id: i64,
    /// 1-based line of the call.
    pub line: u32,
    /// The command name string passed to invoke().
    pub command_name: String,
}

/// A Tauri event emission site in a Rust file.
#[derive(Debug, Clone)]
struct EmitSite {
    file_id: i64,
    line: u32,
    event_name: String,
}

/// A Tauri event listener in a TypeScript file.
#[derive(Debug, Clone)]
struct ListenSite {
    file_id: i64,
    line: u32,
    event_name: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Find all `#[tauri::command]` attributed functions in indexed Rust files.
///
/// Files are read from disk via `project_root`.
pub fn find_tauri_commands(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<TauriCommand>> {
    let re_attr = build_command_attr_regex();
    let re_fn = build_fn_decl_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'rust'")
        .context("Failed to prepare Rust files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Rust files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Rust file rows")?;

    let mut commands: Vec<TauriCommand> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Rust file");
                continue;
            }
        };

        extract_commands_from_source(
            conn, &source, file_id, &rel_path, &re_attr, &re_fn, &mut commands,
        );
    }

    debug!(count = commands.len(), "Tauri commands found");
    Ok(commands)
}

/// Find all `invoke("command_name")` call sites in indexed TS/JS files.
pub fn find_invoke_calls(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<InvokeCall>> {
    let re_invoke = build_invoke_regex();

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
        )
        .context("Failed to prepare TS/JS files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS/JS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS/JS file rows")?;

    let mut calls: Vec<InvokeCall> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TS file");
                continue;
            }
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_invoke.captures_iter(line_text) {
                let name = cap
                    .name("name1")
                    .or_else(|| cap.name("name2"))
                    .or_else(|| cap.name("name3"))
                    .map(|m| m.as_str().to_string());

                if let Some(command_name) = name {
                    calls.push(InvokeCall {
                        file_id,
                        line: line_no,
                        command_name,
                    });
                }
            }
        }
    }

    debug!(count = calls.len(), "invoke() calls found");
    Ok(calls)
}

/// Match Tauri commands to invoke() calls by command name and create flow_edges.
///
/// Returns the number of flow_edges inserted.
pub fn link_commands_to_invocations(
    conn: &Connection,
    commands: &[TauriCommand],
    calls: &[InvokeCall],
) -> Result<u32> {
    if commands.is_empty() || calls.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for call in calls {
        let matching_cmd = commands.iter().find(|c| c.command_name == call.command_name);

        let cmd = match matching_cmd {
            Some(c) => c,
            None => {
                debug!(
                    command = %call.command_name,
                    "No Rust command found for invoke() call"
                );
                continue;
            }
        };

        let result = conn.execute(
            "INSERT OR IGNORE INTO flow_edges (
                source_file_id, source_line, source_symbol, source_language,
                target_file_id, target_line, target_symbol, target_language,
                edge_type, protocol, confidence
             ) VALUES (
                ?1, ?2, ?3, 'typescript',
                ?4, ?5, ?6, 'rust',
                'tauri_ipc', 'ipc', 0.95
             )",
            rusqlite::params![
                call.file_id,
                call.line,
                call.command_name,
                cmd.file_id,
                cmd.line,
                cmd.command_name,
            ],
        );

        match result {
            Ok(n) if n > 0 => created += 1,
            Ok(_) => {}
            Err(e) => {
                debug!(err = %e, "Failed to insert tauri_ipc flow_edge");
            }
        }
    }

    info!(created, "Tauri IPC: linked commands to invoke() calls");
    Ok(created)
}

/// Detect Tauri event emitters (Rust) and listeners (TypeScript) and link them.
///
/// This is a self-contained sub-connector called by the main `connect()` entry
/// point.  Returns the number of flow_edges inserted.
pub fn link_tauri_events(conn: &Connection, project_root: &Path) -> Result<u32> {
    let emitters = find_emit_sites(conn, project_root)?;
    let listeners = find_listen_sites(conn, project_root)?;

    if emitters.is_empty() || listeners.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for emitter in &emitters {
        for listener in listeners.iter().filter(|l| l.event_name == emitter.event_name) {
            let result = conn.execute(
                "INSERT OR IGNORE INTO flow_edges (
                    source_file_id, source_line, source_symbol, source_language,
                    target_file_id, target_line, target_symbol, target_language,
                    edge_type, protocol, confidence
                 ) VALUES (
                    ?1, ?2, ?3, 'rust',
                    ?4, ?5, ?6, 'typescript',
                    'tauri_event', 'ipc', 0.90
                 )",
                rusqlite::params![
                    emitter.file_id,
                    emitter.line,
                    emitter.event_name,
                    listener.file_id,
                    listener.line,
                    listener.event_name,
                ],
            );

            match result {
                Ok(n) if n > 0 => created += 1,
                Ok(_) => {}
                Err(e) => {
                    debug!(err = %e, "Failed to insert tauri_event flow_edge");
                }
            }
        }
    }

    info!(created, "Tauri events: linked emitters to listeners");
    Ok(created)
}

/// Convenience entry point: run all Tauri IPC detection and linking.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<()> {
    let commands = find_tauri_commands(conn, project_root)?;
    let calls = find_invoke_calls(conn, project_root)?;
    let ipc_edges = link_commands_to_invocations(conn, &commands, &calls)?;
    let event_edges = link_tauri_events(conn, project_root)?;
    info!(ipc_edges, event_edges, "Tauri IPC connector complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_command_attr_regex() -> Regex {
    // Matches #[tauri::command] or #[command]
    Regex::new(r"#\[(?:tauri::)?command\]").expect("command attr regex is valid")
}

fn build_fn_decl_regex() -> Regex {
    // Matches the function name from lines like:
    //   pub async fn read_file(...)
    //   pub fn write_file(...)
    //   fn internal_handler(...)
    // Captures group 1: function name
    Regex::new(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*[(<]")
        .expect("fn decl regex is valid")
}

fn build_invoke_regex() -> Regex {
    // Matches:
    //   invoke("name", ...)
    //   invoke('name', ...)
    //   invoke<ReturnType>("name", ...)
    //   invoke<ReturnType>('name', ...)
    //   invoke(`name`, ...)  (template literal — unusual but possible)
    Regex::new(
        r#"invoke\s*(?:<[^>]*>)?\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
    )
    .expect("invoke regex is valid")
}

fn build_emit_regex() -> Regex {
    // Matches: app.emit("event-name", ...) or window.emit("event-name", ...)
    // or handle.emit("event-name", ...) — any receiver
    Regex::new(
        r#"\.emit\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)')"#,
    )
    .expect("emit regex is valid")
}

fn build_listen_regex() -> Regex {
    // Matches: listen("event-name", ...) or appWindow.listen("event-name", ...)
    Regex::new(
        r#"(?:\.\s*)?listen\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
    )
    .expect("listen regex is valid")
}

fn extract_commands_from_source(
    conn: &Connection,
    source: &str,
    file_id: i64,
    rel_path: &str,
    re_attr: &Regex,
    re_fn: &Regex,
    out: &mut Vec<TauriCommand>,
) {
    let mut next_line_is_command = false;

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        if re_attr.is_match(line_text) {
            next_line_is_command = true;
            continue;
        }

        if next_line_is_command {
            next_line_is_command = false;

            if let Some(cap) = re_fn.captures(line_text) {
                let command_name = cap[1].to_string();

                // Try to find the symbol in the DB.
                let symbol_id: Option<i64> = conn
                    .query_row(
                        "SELECT s.id FROM symbols s
                         JOIN files f ON f.id = s.file_id
                         WHERE s.name = ?1 AND f.path = ?2
                           AND s.kind IN ('function', 'method')
                         LIMIT 1",
                        rusqlite::params![command_name, rel_path],
                        |r| r.get(0),
                    )
                    .optional();

                out.push(TauriCommand {
                    symbol_id,
                    command_name,
                    file_id,
                    line: line_no,
                });
            }
            // If the line after the attribute is not a fn, ignore and continue.
            // (e.g. a comment between attribute and fn)
        }
    }
}

fn find_emit_sites(conn: &Connection, project_root: &Path) -> Result<Vec<EmitSite>> {
    let re_emit = build_emit_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'rust'")
        .context("Failed to prepare Rust files query for emit sites")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Rust files for emit sites")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Rust file rows")?;

    let mut sites: Vec<EmitSite> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_emit.captures_iter(line_text) {
                let name = cap
                    .name("name1")
                    .or_else(|| cap.name("name2"))
                    .map(|m| m.as_str().to_string());
                if let Some(event_name) = name {
                    sites.push(EmitSite { file_id, line: line_no, event_name });
                }
            }
        }
    }

    Ok(sites)
}

fn find_listen_sites(conn: &Connection, project_root: &Path) -> Result<Vec<ListenSite>> {
    let re_listen = build_listen_regex();

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
        )
        .context("Failed to prepare TS files query for listen sites")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS files for listen sites")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS file rows")?;

    let mut sites: Vec<ListenSite> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_listen.captures_iter(line_text) {
                let name = cap
                    .name("name1")
                    .or_else(|| cap.name("name2"))
                    .or_else(|| cap.name("name3"))
                    .map(|m| m.as_str().to_string());
                if let Some(event_name) = name {
                    sites.push(ListenSite { file_id, line: line_no, event_name });
                }
            }
        }
    }

    Ok(sites)
}

// ---------------------------------------------------------------------------
// Extension trait for rusqlite::Connection
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> Option<T>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // -----------------------------------------------------------------------
    // Unit tests for regex helpers
    // -----------------------------------------------------------------------

    #[test]
    fn invoke_regex_matches_double_quotes() {
        let re = build_invoke_regex();
        let line = r#"await invoke("read_file", { path });"#;
        let caps = re.captures(line).unwrap();
        assert_eq!(caps.name("name1").unwrap().as_str(), "read_file");
    }

    #[test]
    fn invoke_regex_matches_single_quotes() {
        let re = build_invoke_regex();
        let line = r#"invoke('write_config', args)"#;
        let caps = re.captures(line).unwrap();
        assert_eq!(caps.name("name2").unwrap().as_str(), "write_config");
    }

    #[test]
    fn invoke_regex_matches_generic_form() {
        let re = build_invoke_regex();
        let line = r#"const r = await invoke<string>("get_version");"#;
        let caps = re.captures(line).unwrap();
        assert_eq!(caps.name("name1").unwrap().as_str(), "get_version");
    }

    #[test]
    fn command_attr_regex_matches_full_path() {
        let re = build_command_attr_regex();
        assert!(re.is_match("#[tauri::command]"));
    }

    #[test]
    fn command_attr_regex_matches_short_form() {
        let re = build_command_attr_regex();
        assert!(re.is_match("#[command]"));
    }

    #[test]
    fn fn_decl_regex_extracts_name() {
        let re = build_fn_decl_regex();
        let caps = re.captures("pub async fn read_file(path: String) -> String {").unwrap();
        assert_eq!(&caps[1], "read_file");
    }

    #[test]
    fn fn_decl_regex_extracts_simple_fn() {
        let re = build_fn_decl_regex();
        let caps = re.captures("fn close_splashscreen(window: Window) {").unwrap();
        assert_eq!(&caps[1], "close_splashscreen");
    }

    #[test]
    fn emit_regex_matches_app_emit() {
        let re = build_emit_regex();
        let line = r#"app.emit("file-changed", payload)?;"#;
        let caps = re.captures(line).unwrap();
        assert_eq!(caps.name("name1").unwrap().as_str(), "file-changed");
    }

    #[test]
    fn listen_regex_matches_listen_call() {
        let re = build_listen_regex();
        let line = r#"await listen("file-changed", (event) => {});"#;
        let caps = re.captures(line).unwrap();
        assert_eq!(caps.name("name1").unwrap().as_str(), "file-changed");
    }

    #[test]
    fn listen_regex_matches_window_listen() {
        let re = build_listen_regex();
        let line = r#"appWindow.listen("status-update", handler);"#;
        let caps = re.captures(line).unwrap();
        assert_eq!(caps.name("name1").unwrap().as_str(), "status-update");
    }

    // -----------------------------------------------------------------------
    // Integration tests against in-memory DB + temp files
    // -----------------------------------------------------------------------

    fn make_rs_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    fn make_ts_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    #[test]
    fn find_commands_detects_attribute() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let rs_file = make_rs_file(
            "#[tauri::command]\npub async fn greet(name: String) -> String {\n    format!(\"Hello {}!\", name)\n}\n",
        );
        let root = rs_file.path().parent().unwrap();
        let file_name = rs_file.path().file_name().unwrap().to_str().unwrap();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'rust', 0)",
            [file_name],
        )
        .unwrap();

        let commands = find_tauri_commands(conn, root).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command_name, "greet");
        assert_eq!(commands[0].line, 2, "fn is on line 2");
    }

    #[test]
    fn find_invoke_calls_detects_call() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let ts_file = make_ts_file(r#"const result = await invoke("greet", { name: "World" });"#);
        let root = ts_file.path().parent().unwrap();
        let file_name = ts_file.path().file_name().unwrap().to_str().unwrap();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'typescript', 0)",
            [file_name],
        )
        .unwrap();

        let calls = find_invoke_calls(conn, root).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].command_name, "greet");
    }

    #[test]
    fn link_commands_to_invocations_creates_flow_edge() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src-tauri/src/commands.rs', 'h1', 'rust', 0)",
            [],
        )
        .unwrap();
        let rs_file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/api.ts', 'h2', 'typescript', 0)",
            [],
        )
        .unwrap();
        let ts_file_id: i64 = conn.last_insert_rowid();

        let commands = vec![TauriCommand {
            symbol_id: None,
            command_name: "read_file".to_string(),
            file_id: rs_file_id,
            line: 5,
        }];

        let calls = vec![InvokeCall {
            file_id: ts_file_id,
            line: 12,
            command_name: "read_file".to_string(),
        }];

        let created = link_commands_to_invocations(conn, &commands, &calls).unwrap();
        assert_eq!(created, 1);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'tauri_ipc'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify languages are set correctly.
        let (src_lang, tgt_lang): (String, String) = conn
            .query_row(
                "SELECT source_language, target_language FROM flow_edges WHERE edge_type = 'tauri_ipc'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(src_lang, "typescript");
        assert_eq!(tgt_lang, "rust");
    }

    #[test]
    fn unmatched_invoke_creates_no_edge() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/api.ts', 'h', 'typescript', 0)",
            [],
        )
        .unwrap();
        let ts_file_id: i64 = conn.last_insert_rowid();

        let commands: Vec<TauriCommand> = Vec::new();
        let calls = vec![InvokeCall {
            file_id: ts_file_id,
            line: 1,
            command_name: "nonexistent_command".to_string(),
        }];

        let created = link_commands_to_invocations(conn, &commands, &calls).unwrap();
        assert_eq!(created, 0);
    }
}
