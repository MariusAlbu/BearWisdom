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
