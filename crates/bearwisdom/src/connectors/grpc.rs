// =============================================================================
// connectors/grpc.rs  —  gRPC .proto → C# service/client connector
//
// Parses .proto files found in the index, extracts service and RPC definitions,
// then matches each RPC to the corresponding C# service implementation.
//
// For each matched pair we insert a `flow_edges` row:
//   source = proto file  →  target = C# implementation file
//   edge_type = 'grpc_call', protocol = 'grpc'
//
// Proto parsing uses regex — tree-sitter-protobuf is not in the workspace
// dependency set, and proto syntax is regular enough for our needs.
// =============================================================================

use anyhow::{Context, Result};
use regex::Regex;
use tracing::debug;

use crate::db::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A proto service with its RPC definitions.
#[derive(Debug, Clone)]
pub struct ProtoService {
    pub file_id: i64,
    pub service_name: String,
    pub rpcs: Vec<ProtoRpc>,
}

/// A single RPC method inside a proto service.
#[derive(Debug, Clone)]
pub struct ProtoRpc {
    pub name: String,
    pub input_type: String,
    pub output_type: String,
    pub line: u32,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse .proto files in the index and create flow edges to C# implementations.
///
/// Steps:
///   1. Query all files with language = 'protobuf' or path ending in '.proto'.
///   2. Read and parse each file with regex to extract services + RPCs.
///   3. For each service, find a matching C# class (name ends in "Base" or
///      matches the service name directly).
///   4. For each RPC, find a matching C# method and insert a flow_edge.
pub fn connect(db: &Database) -> Result<()> {
    let conn = db.conn();

    // 1. Load proto files.
    let proto_files = load_proto_files(conn)?;
    if proto_files.is_empty() {
        debug!("No proto files found; gRPC connector has nothing to do");
        return Ok(());
    }

    // Compile regexes once.
    let re_service = Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#)
        .expect("service regex is valid");
    let re_rpc = Regex::new(
        r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
    )
    .expect("rpc regex is valid");

    // 2. Parse each proto file.
    let mut all_services: Vec<ProtoService> = Vec::new();

    for (file_id, abs_path) in &proto_files {
        let source = match std::fs::read_to_string(abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path, err = %e, "Skipping unreadable proto file");
                continue;
            }
        };

        let services = parse_proto_services(&source, *file_id, &re_service, &re_rpc);
        debug!(
            path = %abs_path,
            services = services.len(),
            "Parsed proto file"
        );
        all_services.extend(services);
    }

    if all_services.is_empty() {
        return Ok(());
    }

    // 3 + 4. Match services to C# implementations and insert flow_edges.
    let mut edges_created: u32 = 0;
    for service in &all_services {
        edges_created += match_service_to_csharp(conn, service)?;
    }

    debug!(edges_created, "gRPC connector complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Query the DB for all files that look like proto files.
fn load_proto_files(conn: &rusqlite::Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'protobuf' OR path LIKE '%.proto'",
        )
        .context("Failed to prepare proto files query")?;

    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to execute proto files query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect proto file rows")?;

    Ok(rows)
}

/// Parse service and RPC definitions from proto source text.
///
/// Line numbers are 1-based and approximate — we count newlines up to each
/// regex match position.
///
/// Public alias for use by the new `GrpcConnector`.
pub(super) fn parse_proto_services_pub(
    source: &str,
    file_id: i64,
    re_service: &Regex,
    re_rpc: &Regex,
) -> Vec<ProtoService> {
    parse_proto_services(source, file_id, re_service, re_rpc)
}

fn parse_proto_services(
    source: &str,
    file_id: i64,
    re_service: &Regex,
    re_rpc: &Regex,
) -> Vec<ProtoService> {
    let mut services: Vec<ProtoService> = Vec::new();

    for service_cap in re_service.captures_iter(source) {
        let service_name = service_cap[1].to_string();
        let service_start = service_cap.get(0).map(|m| m.start()).unwrap_or(0);

        // Find the matching closing brace to bound the service block.
        let block_end = find_closing_brace(source, service_start);
        let service_block = &source[service_start..block_end];

        let mut rpcs: Vec<ProtoRpc> = Vec::new();
        for rpc_cap in re_rpc.captures_iter(service_block) {
            let rpc_start_in_block = rpc_cap.get(0).map(|m| m.start()).unwrap_or(0);
            let abs_offset = service_start + rpc_start_in_block;
            let line = line_number_at(source, abs_offset);

            rpcs.push(ProtoRpc {
                name: rpc_cap[1].to_string(),
                input_type: rpc_cap[2].to_string(),
                output_type: rpc_cap[3].to_string(),
                line,
            });
        }

        services.push(ProtoService {
            file_id,
            service_name,
            rpcs,
        });
    }

    services
}

/// Walk the source string from `start` to find the byte offset just past the
/// matching `}` for the opening `{`.  Skips string literals naively.
fn find_closing_brace(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth: i32 = 0;
    let mut i = start;

    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Unclosed block — return end of source.
    bytes.len()
}

/// Count how many newlines precede `offset` to get an approximate 1-based
/// line number.
fn line_number_at(source: &str, offset: usize) -> u32 {
    let safe_offset = offset.min(source.len());
    source[..safe_offset].bytes().filter(|&b| b == b'\n').count() as u32 + 1
}

/// For a proto service, find a matching C# class and its RPC method
/// implementations, then insert flow_edges.
fn match_service_to_csharp(
    conn: &rusqlite::Connection,
    service: &ProtoService,
) -> Result<u32> {
    // Look for a C# class named "${ServiceName}Base" (the gRPC generated base)
    // or exactly "${ServiceName}".
    let base_name = format!("{}Base", service.service_name);

    let cs_class: Option<(i64, i64, u32)> = conn
        .query_row(
            "SELECT s.id, f.id, s.line
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.kind = 'class'
               AND f.language = 'csharp'
               AND (s.name = ?1 OR s.name = ?2)
             LIMIT 1",
            rusqlite::params![base_name, service.service_name],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, u32>(2)?)),
        )
        .optional()
        .context("Failed to query C# class for proto service")?;

    let (cs_class_sym_id, cs_file_id, _cs_class_line) = match cs_class {
        Some(row) => row,
        None => {
            debug!(service = %service.service_name, "No C# class found for proto service");
            return Ok(0);
        }
    };

    let mut edges_created: u32 = 0;

    for rpc in &service.rpcs {
        // Find the C# method in the same class (by file_id + name).
        let cs_method: Option<(i64, u32)> = conn
            .query_row(
                "SELECT s.id, s.line
                 FROM symbols s
                 WHERE s.kind = 'method'
                   AND s.name = ?1
                   AND s.file_id = ?2
                 LIMIT 1",
                rusqlite::params![rpc.name, cs_file_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()
            .context("Failed to query C# method for proto RPC")?;

        let (target_sym_id, target_line) = match cs_method {
            Some(row) => row,
            None => {
                debug!(
                    service = %service.service_name,
                    rpc = %rpc.name,
                    "No C# method found for proto RPC"
                );
                continue;
            }
        };

        let _ = target_sym_id; // kept for future use (e.g. symbol-level edge)

        let metadata = serde_json::json!({
            "service": service.service_name,
            "rpc": rpc.name,
            "input_type": rpc.input_type,
            "output_type": rpc.output_type,
            "cs_class_sym_id": cs_class_sym_id,
        })
        .to_string();

        let result = conn.execute(
            "INSERT OR IGNORE INTO flow_edges (
                source_file_id, source_line, source_symbol, source_language,
                target_file_id, target_line, target_symbol, target_language,
                edge_type, protocol, confidence, metadata
             ) VALUES (
                ?1, ?2, ?3, 'protobuf',
                ?4, ?5, ?6, 'csharp',
                'grpc_call', 'grpc', 0.9, ?7
             )",
            rusqlite::params![
                service.file_id,
                rpc.line,
                rpc.name,
                cs_file_id,
                target_line,
                rpc.name,
                metadata,
            ],
        );

        match result {
            Ok(n) if n > 0 => edges_created += 1,
            Ok(_) => {}
            Err(e) => {
                debug!(err = %e, "Failed to insert grpc flow_edge");
            }
        }
    }

    Ok(edges_created)
}

// ---------------------------------------------------------------------------
// Extension trait for rusqlite::Connection
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "grpc_tests.rs"]
mod tests;
