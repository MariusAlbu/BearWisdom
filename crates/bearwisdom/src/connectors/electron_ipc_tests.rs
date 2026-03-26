use super::*;
use crate::db::Database;
use std::io::Write;
use tempfile::NamedTempFile;

fn make_ts_file(content: &str) -> NamedTempFile {
    let mut f = tempfile::Builder::new().suffix(".ts").tempfile().unwrap();
    write!(f, "{}", content).unwrap();
    f
}

fn insert_ts_file(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'typescript', 0)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

// -----------------------------------------------------------------------
// Regex unit tests
// -----------------------------------------------------------------------

#[test]
fn ipc_main_regex_matches_handle() {
    let re = build_ipc_main_regex();
    let line = r#"ipcMain.handle("read-file", async (event, path) => {});"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap.name("ch1").unwrap().as_str(), "read-file");
}

#[test]
fn ipc_main_regex_matches_on() {
    let re = build_ipc_main_regex();
    let line = r#"ipcMain.on('write-file', (event, data) => {});"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap.name("ch2").unwrap().as_str(), "write-file");
}

#[test]
fn ipc_renderer_regex_matches_invoke() {
    let re = build_ipc_renderer_regex();
    let line = r#"const result = await ipcRenderer.invoke("read-file", filePath);"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap.name("ch1").unwrap().as_str(), "read-file");
}

#[test]
fn ipc_renderer_regex_matches_send() {
    let re = build_ipc_renderer_regex();
    let line = r#"ipcRenderer.send('write-file', { content });"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap.name("ch2").unwrap().as_str(), "write-file");
}

#[test]
fn context_bridge_regex_extracts_api_name() {
    let re = build_context_bridge_regex();
    let line = r#"contextBridge.exposeInMainWorld("electronAPI", { readFile });"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap.name("n1").unwrap().as_str(), "electronAPI");
}

// -----------------------------------------------------------------------
// Integration tests
// -----------------------------------------------------------------------

#[test]
fn find_handlers_detects_ipc_main() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let main_file = make_ts_file(
        "ipcMain.handle(\"read-file\", async (event, path) => readFileSync(path));\n",
    );
    let root = main_file.path().parent().unwrap();
    let file_name = main_file.path().file_name().unwrap().to_str().unwrap();

    insert_ts_file(conn, file_name);

    let handlers = find_ipc_handlers(conn, root).unwrap();
    assert_eq!(handlers.len(), 1);
    assert_eq!(handlers[0].channel, "read-file");
}

#[test]
fn link_channels_creates_flow_edge() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let main_file_id = {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('main/index.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    };

    let renderer_file_id = {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('renderer/app.ts', 'h2', 'typescript', 0)",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    };

    let handlers = vec![IpcHandler {
        file_id: main_file_id,
        line: 5,
        channel: "open-dialog".to_string(),
    }];

    let invocations = vec![IpcInvocation {
        file_id: renderer_file_id,
        line: 12,
        channel: "open-dialog".to_string(),
    }];

    let created = link_channels(conn, &handlers, &invocations).unwrap();
    assert_eq!(created, 1, "Expected one electron_ipc flow_edge");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'electron_ipc'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn mismatched_channels_creates_no_edge() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('main.ts', 'h', 'typescript', 0)",
        [],
    )
    .unwrap();
    let fid: i64 = conn.last_insert_rowid();

    let handlers = vec![IpcHandler {
        file_id: fid,
        line: 1,
        channel: "channel-a".to_string(),
    }];

    let invocations = vec![IpcInvocation {
        file_id: fid,
        line: 2,
        channel: "channel-b".to_string(),
    }];

    let created = link_channels(conn, &handlers, &invocations).unwrap();
    assert_eq!(created, 0);
}
