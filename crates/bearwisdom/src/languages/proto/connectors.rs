// =============================================================================
// languages/proto/connectors.rs — Proto gRPC start-point connector
//
// Parses .proto files for service/RPC definitions and emits Start connection
// points for each RPC.  Stop points come from the per-language connectors in
// each consuming language (C#, Go, Java, Python, Rust).
//
// Proto parsing is regex-based — tree-sitter-proto does not give us the RPC
// structure we need for accurate line numbers, and the proto grammar is
// regular enough that regex is sufficient.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

pub struct ProtoGrpcConnector;

impl Connector for ProtoGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "proto_grpc_starts",
            protocols: &[Protocol::Grpc],
            languages: &["protobuf"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        // Always run — proto files may exist in any project.
        true
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_service = Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#)
            .expect("service regex");
        let re_rpc = Regex::new(
            r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
        )
        .expect("rpc regex");

        let proto_files = load_proto_files(conn)
            .context("Failed to load proto file list")?;
        if proto_files.is_empty() {
            return Ok(vec![]);
        }

        let mut points = Vec::new();

        for (file_id, rel_path) in &proto_files {
            let abs_path = project_root.join(rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for service_cap in re_service.captures_iter(&source) {
                let service_name = service_cap[1].to_string();
                let service_start = service_cap.get(0).map(|m| m.start()).unwrap_or(0);
                let block_end = find_closing_brace(&source, service_start);
                let service_block = &source[service_start..block_end];

                for rpc_cap in re_rpc.captures_iter(service_block) {
                    let rpc_start_in_block = rpc_cap.get(0).map(|m| m.start()).unwrap_or(0);
                    let abs_offset = service_start + rpc_start_in_block;
                    let line = line_number_at(&source, abs_offset);

                    let rpc_name = rpc_cap[1].to_string();
                    let input_type = rpc_cap[2].to_string();
                    let output_type = rpc_cap[3].to_string();
                    let key = format!("{service_name}.{rpc_name}");

                    let metadata = serde_json::json!({
                        "input_type": input_type,
                        "output_type": output_type,
                    })
                    .to_string();

                    points.push(ConnectionPoint {
                        file_id: *file_id,
                        symbol_id: None,
                        line,
                        protocol: Protocol::Grpc,
                        direction: FlowDirection::Start,
                        key,
                        method: String::new(),
                        framework: String::new(),
                        metadata: Some(metadata),
                    });
                }
            }
        }

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_proto_files(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'protobuf' OR language = 'proto' OR path LIKE '%.proto'",
        )
        .context("Failed to prepare proto files query")?;

    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to execute proto files query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect proto file rows")?;

    Ok(rows)
}

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
    bytes.len()
}

fn line_number_at(source: &str, offset: usize) -> u32 {
    let safe_offset = offset.min(source.len());
    source[..safe_offset].bytes().filter(|&b| b == b'\n').count() as u32 + 1
}
