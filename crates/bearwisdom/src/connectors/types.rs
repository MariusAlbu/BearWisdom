// =============================================================================
// connectors/types.rs — Protocol enum
//
// Phase H removed the `ConnectionPoint` / `ResolvedFlow` / matcher pipeline.
// The `Protocol` enum stays because dockerfile / HCL infrastructure connectors
// still write directly to `flow_edges` and use the enum's string form as the
// `protocol` column value.
// =============================================================================

use serde::{Deserialize, Serialize};

/// Protocol family for a flow edge.
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
    /// Infrastructure-level relationships: Docker Compose service dependencies,
    /// Kubernetes deployments, etc.
    Infrastructure,
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
            Self::Infrastructure => "infrastructure",
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
