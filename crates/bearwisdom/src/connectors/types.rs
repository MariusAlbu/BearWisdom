// =============================================================================
// connectors/types.rs — Core types for the connector architecture
//
// ConnectionPoint is the unit of work: one row in connection_points table.
// ResolvedFlow is the output of matching: one row in flow_edges table.
// =============================================================================

use serde::{Deserialize, Serialize};

/// Protocol family for a connection point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Rest,
    Grpc,
    GraphQl,
    MessageQueue,
    EventBus,
    WebSocket,
    Ffi,
    Ipc,
    Di,
}

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Grpc => "grpc",
            Self::GraphQl => "graphql",
            Self::MessageQueue => "message_queue",
            Self::EventBus => "event_bus",
            Self::WebSocket => "websocket",
            Self::Ffi => "ffi",
            Self::Ipc => "ipc",
            Self::Di => "di",
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a connection point is a caller/producer or a handler/consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowDirection {
    /// Caller, producer, or emitter side.
    Start,
    /// Handler, consumer, or listener side.
    Stop,
}

impl FlowDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

impl std::fmt::Display for FlowDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single extracted call site or handler binding.
///
/// Maps 1:1 to a row in the `connection_points` table.
#[derive(Debug, Clone)]
pub struct ConnectionPoint {
    /// FK into `files`.
    pub file_id: i64,
    /// Optional FK into `symbols`.
    pub symbol_id: Option<i64>,
    /// Source line (1-based).
    pub line: u32,
    pub protocol: Protocol,
    pub direction: FlowDirection,
    /// The identifying key: URL pattern, topic name, event name, symbol name, etc.
    pub key: String,
    /// HTTP method, GraphQL operation type, or empty string when not applicable.
    pub method: String,
    /// Framework discriminator: "kafka", "rabbitmq", "tauri", etc. Empty when N/A.
    pub framework: String,
    /// Optional JSON blob for protocol-specific details.
    pub metadata: Option<String>,
}

/// A matched start→stop pair, ready to be written as a `flow_edges` row.
#[derive(Debug, Clone)]
pub struct ResolvedFlow {
    pub start: ConnectionPoint,
    pub stop: ConnectionPoint,
    /// 0.0–1.0 matching confidence.
    pub confidence: f64,
    /// Becomes `edge_type` in `flow_edges`: "http_call", "grpc_call", "message_queue", etc.
    pub edge_type: String,
}
