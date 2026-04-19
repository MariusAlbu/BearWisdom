// =============================================================================
// languages/proto/connectors.rs — Proto gRPC start-point detection
//
// Parses .proto files for service/RPC definitions and emits Start connection
// points for each RPC. Stop points come from per-language consumer plugins
// (C#, Go, Java, Python, Rust) — those migrate separately.
//
// Flattened into the language plugin: `ProtoPlugin::extract_connection_points`
// calls `extract_proto_grpc_starts` at parse time. The registry-facing
// `ProtoGrpcConnector::extract` returns empty so the point isn't emitted twice.
//
// Proto parsing is regex-based — tree-sitter-proto does not give us the RPC
// structure we need for accurate line numbers, and the proto grammar is
// regular enough that regex is sufficient.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint as DbPoint, Protocol};
use crate::indexer::project_context::ProjectContext;
use crate::types::{ConnectionKind, ConnectionPoint, ConnectionRole};

pub struct ProtoGrpcConnector;

impl Connector for ProtoGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "proto_grpc_starts",
            protocols: &[Protocol::Grpc],
            languages: &["protobuf"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool { true }

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<DbPoint>> {
        // Detection moved to `extract_proto_grpc_starts` invoked at parse
        // time via `ProtoPlugin::extract_connection_points`.
        Ok(Vec::new())
    }
}

/// Scan a `.proto` source for top-level `service Foo { rpc X(…) returns (…); }`
/// blocks and emit a Start `ConnectionPoint` per RPC. Called during parse
/// from `ProtoPlugin::extract_connection_points`.
pub fn extract_proto_grpc_starts(source: &str) -> Vec<ConnectionPoint> {
    let re_service =
        Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#).expect("service regex");
    let re_rpc = Regex::new(
        r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
    )
    .expect("rpc regex");

    let mut out: Vec<ConnectionPoint> = Vec::new();
    for service_cap in re_service.captures_iter(source) {
        let service_name = service_cap[1].to_string();
        let service_start = service_cap.get(0).map(|m| m.start()).unwrap_or(0);
        let block_end = find_closing_brace(source, service_start);
        let service_block = &source[service_start..block_end];

        for rpc_cap in re_rpc.captures_iter(service_block) {
            let rpc_start_in_block = rpc_cap.get(0).map(|m| m.start()).unwrap_or(0);
            let abs_offset = service_start + rpc_start_in_block;
            let line = line_number_at(source, abs_offset);

            let rpc_name = rpc_cap[1].to_string();
            let input_type = rpc_cap[2].to_string();
            let output_type = rpc_cap[3].to_string();
            let key = format!("{service_name}.{rpc_name}");

            let mut meta = HashMap::new();
            meta.insert("input_type".to_string(), input_type);
            meta.insert("output_type".to_string(), output_type);

            out.push(ConnectionPoint {
                kind: ConnectionKind::Grpc,
                role: ConnectionRole::Start,
                key,
                line,
                col: 1,
                symbol_qname: String::new(),
                meta,
            });
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_one_start_per_rpc() {
        let src = r#"
syntax = "proto3";

service UserService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc ListUsers(ListUsersRequest) returns (ListUsersResponse);
}

service BillingService {
  rpc Charge(ChargeRequest) returns (ChargeResponse);
}
"#;
        let points = extract_proto_grpc_starts(src);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].key, "UserService.GetUser");
        assert_eq!(
            points[0].meta.get("input_type").map(String::as_str),
            Some("GetUserRequest"),
        );
        assert_eq!(points[2].key, "BillingService.Charge");
    }

    #[test]
    fn empty_source_produces_no_points() {
        assert!(extract_proto_grpc_starts("").is_empty());
    }
}
