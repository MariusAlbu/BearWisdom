// =============================================================================
// connectors/grpc_connector.rs — gRPC connector (new architecture)
//
// Wraps the existing grpc.rs logic.
//
// Stop points: RPC method implementations in C# (or other lang) services.
// Start points: RPC definitions in .proto files (the "declaration" side that
//               drives client generation).
//
// The matching key is "ServiceName.RpcName" — the ProtocolMatcher does exact
// key matching.  Custom matching is not needed because we normalise both sides
// to the same key format.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::grpc::{self, ProtoService};
use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

pub struct GrpcConnector;

impl Connector for GrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "grpc",
            protocols: &[Protocol::Grpc],
            languages: &["protobuf", "csharp", "go", "java", "python", "rust"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        // Detection is cheap (just checking if proto files exist) so always run.
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();

        // Reuse existing proto parsing.
        let proto_files = load_proto_files(conn)?;
        if proto_files.is_empty() {
            return Ok(points);
        }

        let re_service = regex::Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#)
            .expect("service regex");
        let re_rpc = regex::Regex::new(
            r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
        )
        .expect("rpc regex");

        let mut all_services: Vec<ProtoService> = Vec::new();
        for (file_id, abs_path) in &proto_files {
            let source = match std::fs::read_to_string(abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            all_services.extend(grpc::parse_proto_services_pub(
                &source, *file_id, &re_service, &re_rpc,
            ));
        }

        // Start points: proto RPC definitions (the "caller" side — protos define
        // what gets called).
        for service in &all_services {
            for rpc in &service.rpcs {
                let key = format!("{}.{}", service.service_name, rpc.name);
                let metadata = serde_json::json!({
                    "input_type": rpc.input_type,
                    "output_type": rpc.output_type,
                })
                .to_string();

                points.push(ConnectionPoint {
                    file_id: service.file_id,
                    symbol_id: None,
                    line: rpc.line,
                    protocol: Protocol::Grpc,
                    direction: FlowDirection::Start,
                    key: key.clone(),
                    method: String::new(),
                    framework: String::new(),
                    metadata: Some(metadata),
                });
            }
        }

        // Stop points: C# (or other lang) implementations matching service.rpc.
        for service in &all_services {
            extract_grpc_impl_stops(conn, service, &mut points)?;
        }

        let _ = project_root; // used by load_proto_files via abs paths
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

fn extract_grpc_impl_stops(
    conn: &Connection,
    service: &ProtoService,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let base_name = format!("{}Base", service.service_name);

    // Find the implementation class.
    let cs_file_id: Option<i64> = conn
        .query_row(
            "SELECT f.id
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.kind = 'class'
               AND (s.name = ?1 OR s.name = ?2)
             LIMIT 1",
            rusqlite::params![base_name, service.service_name],
            |row| row.get(0),
        )
        .optional_ext()?;

    let cs_file_id = match cs_file_id {
        Some(id) => id,
        None => return Ok(()),
    };

    for rpc in &service.rpcs {
        let method_info: Option<(i64, u32)> = conn
            .query_row(
                "SELECT s.id, s.line
                 FROM symbols s
                 WHERE s.kind = 'method'
                   AND s.name = ?1
                   AND s.file_id = ?2
                 LIMIT 1",
                rusqlite::params![rpc.name, cs_file_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional_ext()?;

        if let Some((sym_id, line)) = method_info {
            let key = format!("{}.{}", service.service_name, rpc.name);
            out.push(ConnectionPoint {
                file_id: cs_file_id,
                symbol_id: Some(sym_id),
                line,
                protocol: Protocol::Grpc,
                direction: FlowDirection::Stop,
                key,
                method: String::new(),
                framework: String::new(),
                metadata: None,
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional_ext(self) -> Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional_ext(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
