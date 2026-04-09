// =============================================================================
// languages/rust_lang/connectors.rs — Rust language connectors
//
// Contains:
//   - TauriIpcConnector (Rust stop-side: #[tauri::command] handlers)
//
// The TypeScript start-side (invoke() calls) lives in
// languages/typescript/connectors.rs. Both halves emit ConnectionPoints
// that the matcher joins into flow_edges.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// TauriIpcConnector — Rust handler side
// ===========================================================================

pub struct TauriIpcConnector;

impl Connector for TauriIpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "tauri_ipc_rust",
            protocols: &[Protocol::Ipc],
            languages: &["rust"],
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
        let mut points = Vec::new();

        // #[tauri::command] attributed functions → Stop points
        let commands = rust_find_tauri_commands(conn, project_root)
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

        // Rust app.emit() / window.emit() sites → Start points for events
        extract_tauri_emit_events(conn, project_root, &mut points)?;

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Tauri IPC Rust-side helpers (inlined from connectors/tauri_ipc.rs)
// ---------------------------------------------------------------------------

struct RustTauriCommand {
    symbol_id: Option<i64>,
    command_name: String,
    file_id: i64,
    line: u32,
}

fn rust_find_tauri_commands(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<RustTauriCommand>> {
    let re_attr = regex::Regex::new(r"#\[(?:tauri::)?command\]")
        .expect("command attr regex is valid");
    let re_fn = regex::Regex::new(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*[(<]")
        .expect("fn decl regex is valid");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'rust'")
        .context("Failed to prepare Rust files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Rust files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Rust file rows")?;

    let mut commands = Vec::new();
    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Rust file");
                continue;
            }
        };
        rust_extract_commands_from_source(conn, &source, file_id, &rel_path, &re_attr, &re_fn, &mut commands);
    }
    tracing::debug!(count = commands.len(), "Tauri commands found");
    Ok(commands)
}

fn rust_extract_commands_from_source(
    conn: &Connection,
    source: &str,
    file_id: i64,
    rel_path: &str,
    re_attr: &regex::Regex,
    re_fn: &regex::Regex,
    out: &mut Vec<RustTauriCommand>,
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
                    .ok();

                out.push(RustTauriCommand { symbol_id, command_name, file_id, line: line_no });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Event emit detection (Rust side)
// ---------------------------------------------------------------------------

fn extract_tauri_emit_events(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let re_emit = regex::Regex::new(
        r#"\.emit\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)')"#,
    )
    .expect("emit regex");

    scan_files_for_pattern(conn, project_root, "rust", &re_emit, FlowDirection::Start, "tauri", out)
}

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

// ===========================================================================
// RustRestConnector — HTTP client call starts + route stops for Rust
// ===========================================================================

pub struct RustRestConnector;

impl Connector for RustRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rust_rest",
            protocols: &[Protocol::Rest],
            languages: &["rust"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();
        extract_rust_rest_stops(conn, &mut points)?;
        extract_rust_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

fn extract_rust_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'rust'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Rust REST stops query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("Failed to query Rust routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Rust route row")?;
        out.push(ConnectionPoint {
            file_id,
            symbol_id,
            line: line.unwrap_or(0),
            protocol: Protocol::Rest,
            direction: FlowDirection::Stop,
            key: route,
            method: method.to_uppercase(),
            framework: String::new(),
            metadata: None,
        });
    }
    Ok(())
}

fn extract_rust_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // reqwest: client.get("url"), reqwest::get("url"), reqwest::Client::new().post("url")
    let re_reqwest = regex::Regex::new(
        r#"(?:reqwest(?:::\w+)?|client)\s*(?:::\w+)?\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("rust reqwest regex");

    // reqwest::get("url") — standalone free function
    let re_reqwest_free = regex::Regex::new(
        r#"reqwest\s*::\s*get\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("rust reqwest free fn regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'rust'")
        .context("Failed to prepare Rust files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Rust files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Rust file rows")?;

    for (file_id, rel_path) in files {
        if rust_rest_is_test_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            for cap in re_reqwest.captures_iter(line_text) {
                let raw_url = cap["url"].to_string();
                if !rust_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
                let method = cap
                    .name("method")
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                let url_pattern = rest_normalise_url_pattern_rust(&raw_url);
                out.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: line_no,
                    protocol: Protocol::Rest,
                    direction: FlowDirection::Start,
                    key: url_pattern,
                    method,
                    framework: "reqwest".to_string(),
                    metadata: None,
                });
            }

            for cap in re_reqwest_free.captures_iter(line_text) {
                let raw_url = cap["url"].to_string();
                if !rust_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
                let url_pattern = rest_normalise_url_pattern_rust(&raw_url);
                out.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: line_no,
                    protocol: Protocol::Rest,
                    direction: FlowDirection::Start,
                    key: url_pattern,
                    method: "GET".to_string(),
                    framework: "reqwest".to_string(),
                    metadata: None,
                });
            }
        }
    }
    Ok(())
}

fn rust_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("/tests/") || lower.contains("_test.rs") || lower.contains("/benches/")
}

fn rust_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() {
            return false;
        }
        return rust_rest_looks_like_api_url(path);
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/v3/")
        || s.contains("/{")
}

fn rest_normalise_url_pattern_rust(raw: &str) -> String {
    raw.split('?').next().unwrap_or(raw).to_string()
}

// ===========================================================================
// RustGrpcConnector
// ===========================================================================

/// Detects Rust tonic gRPC service implementations.
///
/// tonic generates a `{ServiceName}Server` trait.  Implementors write:
///   `#[tonic::async_trait]`
///   `impl GreeterServer for MyGreeter { ... }`
///
/// We scan for `impl {Name}Server for` patterns and emit Stop points for
/// every method in that file at or after the impl block.
pub struct RustGrpcConnector;

impl Connector for RustGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rust_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["rust"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.rust_crates.contains("tonic")
            || ctx.rust_crates.contains("grpcio")
            || ctx.rust_crates.contains("prost")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_impl = regex::Regex::new(r#"impl\s+(\w+Server)\s+for\s+(\w+)"#)
            .expect("rust tonic impl regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'rust'")
            .context("Failed to prepare Rust files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query Rust files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Rust file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in &files {
            let abs_path = project_root.join(rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                if let Some(cap) = re_impl.captures(line_text) {
                    let trait_name = &cap[1];
                    let service_name = trait_name
                        .strip_suffix("Server")
                        .unwrap_or(trait_name)
                        .to_string();

                    if service_name.is_empty() {
                        continue;
                    }

                    let mut method_stmt = conn
                        .prepare(
                            "SELECT s.id, s.name, s.line
                             FROM symbols s
                             WHERE s.file_id = ?1 AND s.kind = 'method'
                               AND s.line >= ?2",
                        )
                        .context("Failed to prepare Rust gRPC method query")?;

                    let methods: Vec<(i64, String, u32)> = method_stmt
                        .query_map(rusqlite::params![file_id, line_no], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, u32>(2)?,
                            ))
                        })
                        .context("Failed to query Rust gRPC methods")?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .context("Failed to collect Rust gRPC method rows")?;

                    for (sym_id, method_name, line) in methods {
                        let key = format!("{service_name}.{method_name}");
                        points.push(ConnectionPoint {
                            file_id: *file_id,
                            symbol_id: Some(sym_id),
                            line,
                            protocol: Protocol::Grpc,
                            direction: FlowDirection::Stop,
                            key,
                            method: String::new(),
                            framework: "tonic".to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// RustMqConnector
// ===========================================================================

/// Detects Rust message queue patterns:
///   - rdkafka: `FutureRecord::to("topic")` (producer)
///              `consumer.subscribe(&["topic"])` (consumer)
///   - lapin (RabbitMQ): `basic_publish("exchange", "routing_key", ...)` (producer)
///                        `basic_consume("queue", ...)` (consumer)
///   - async-nats: `client.subscribe("subject")` (consumer)
///                 `client.publish("subject", ...)` (producer)
pub struct RustMqConnector;

impl Connector for RustMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rust_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["rust"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.rust_crates.contains("rdkafka")
            || ctx.rust_crates.contains("kafka")
            || ctx.rust_crates.contains("lapin")
            || ctx.rust_crates.contains("async-nats")
            || ctx.rust_crates.contains("nats")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // rdkafka: FutureRecord::to("topic")  or  BaseRecord::to("topic")
        let re_rdkafka_send = regex::Regex::new(
            r#"(?:FutureRecord|BaseRecord)::to\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("rust rdkafka send regex");

        // rdkafka: consumer.subscribe(&["topic"])
        let re_rdkafka_subscribe = regex::Regex::new(
            r#"\.subscribe\s*\(\s*&\s*\[\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("rust rdkafka subscribe regex");

        // lapin: channel.basic_publish("exchange", "routing_key", ...)
        let re_lapin_publish = regex::Regex::new(
            r#"basic_publish\s*\(\s*['"`]([^'"`]+)['"`]\s*,\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("rust lapin publish regex");

        // lapin: channel.basic_consume("queue", ...)
        let re_lapin_consume = regex::Regex::new(
            r#"basic_consume\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("rust lapin consume regex");

        // async-nats / nats: client.subscribe("subject")
        let re_nats_subscribe = regex::Regex::new(
            r#"\.subscribe\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("rust nats subscribe regex");

        // async-nats / nats: client.publish("subject", ...)
        let re_nats_publish = regex::Regex::new(
            r#"\.publish\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("rust nats publish regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'rust'")
            .context("Failed to prepare Rust files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query Rust files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Rust file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                for cap in re_rdkafka_send.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_rdkafka_subscribe.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_lapin_publish.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[2].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_lapin_consume.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_nats_subscribe.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "nats".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_nats_publish.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "nats".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tauri_ipc_tests {
    use super::*;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_rs_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    #[test]
    fn command_attr_regex_matches_full_path() {
        let re = regex::Regex::new(r"#\[(?:tauri::)?command\]").unwrap();
        assert!(re.is_match("#[tauri::command]"));
    }

    #[test]
    fn command_attr_regex_matches_short_form() {
        let re = regex::Regex::new(r"#\[(?:tauri::)?command\]").unwrap();
        assert!(re.is_match("#[command]"));
    }

    #[test]
    fn fn_decl_regex_extracts_name() {
        let re = regex::Regex::new(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*[(<]").unwrap();
        let caps = re.captures("pub async fn read_file(path: String) -> String {").unwrap();
        assert_eq!(&caps[1], "read_file");
    }

    #[test]
    fn fn_decl_regex_extracts_simple_fn() {
        let re = regex::Regex::new(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*[(<]").unwrap();
        let caps = re.captures("fn close_splashscreen(window: Window) {").unwrap();
        assert_eq!(&caps[1], "close_splashscreen");
    }

    #[test]
    fn find_commands_detects_attribute() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        let rs_file = make_rs_file(
            "#[tauri::command]\npub async fn greet(name: String) -> String {\n    format!(\"Hello {}!\", name)\n}\n",
        );
        let root = rs_file.path().parent().unwrap();
        let file_name = rs_file.path().file_name().unwrap().to_str().unwrap();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'rust', 0)",
            [file_name],
        ).unwrap();

        let commands = rust_find_tauri_commands(conn, root).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command_name, "greet");
        assert_eq!(commands[0].line, 2, "fn is on line 2");
    }
}
