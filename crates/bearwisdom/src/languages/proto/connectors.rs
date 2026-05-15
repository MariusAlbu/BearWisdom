// =============================================================================
// languages/proto/connectors.rs — Proto gRPC FlowEmission detection
//
// Parses .proto files for service / RPC definitions and emits Consumer
// `FlowEmission::NamedChannel { kind: RpcCall, .. }` entries keyed by
// `Service.Rpc`. Each `.proto` service block describes the server-side
// stub the receiver registers — the Consumer role lines it up with
// per-language client call sites that the resolver emits as Producer.
//
// Proto parsing is regex-based — tree-sitter-proto does not give us the RPC
// structure we need for accurate line numbers, and the proto grammar is
// regular enough that regex is sufficient.
// =============================================================================

use regex::Regex;

use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind, StreamKind};

/// Scan a `.proto` source for top-level `service Foo { rpc X(…) returns (…); }`
/// blocks and emit a Consumer `FlowEmission::NamedChannel { kind: RpcCall, .. }`
/// per RPC. The pairing key is `Service.Rpc`; streaming kind is derived from
/// the `stream` keyword on either side of the RPC declaration.
pub fn extract_proto_grpc_starts(source: &str) -> Vec<(u32, FlowEmission)> {
    let re_service =
        Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#).expect("service regex");
    // Allow optional `stream` modifier on request and/or response sides.
    let re_rpc = Regex::new(
        r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(stream\s+)?\w+\s*\)\s+returns\s+\(\s*(stream\s+)?\w+\s*\)"#,
    )
    .expect("rpc regex");

    let mut out: Vec<(u32, FlowEmission)> = Vec::new();
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
            let client_stream = rpc_cap.get(2).is_some();
            let server_stream = rpc_cap.get(3).is_some();
            let streaming = match (client_stream, server_stream) {
                (true, true) => Some(StreamKind::BidiStreaming),
                (true, false) => Some(StreamKind::ClientStreaming),
                (false, true) => Some(StreamKind::ServerStreaming),
                (false, false) => None,
            };
            let name = format!("{service_name}.{rpc_name}");

            out.push((
                line,
                FlowEmission::NamedChannel {
                    kind: NamedChannelKind::RpcCall,
                    name,
                    role: ChannelRole::Consumer,
                    method: None,
                    streaming,
                },
            ));
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
#[path = "connectors_tests.rs"]
mod tests;
