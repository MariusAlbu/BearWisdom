// =============================================================================
// connectors/event_connector.rs — Event bus connector (new architecture)
//
// Wraps the existing dotnet_events.rs logic.
//
// Start points: integration event classes (the thing that gets published).
// Stop points: event handler classes implementing IIntegrationEventHandler<T>.
//
// The matching key is the event type name.  ProtocolMatcher does exact key
// matching which is exactly how the old connector linked events to handlers.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::dotnet_events;
use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

pub struct EventBusConnector;

impl Connector for EventBusConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "event_bus",
            protocols: &[Protocol::EventBus],
            languages: &["csharp"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        // Only run if this looks like a .NET project.
        !ctx.external_prefixes.is_empty()
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();

        // Start points: integration event classes.
        let events = dotnet_events::find_integration_events(conn)
            .context("Integration event detection failed")?;

        for event in &events {
            let file_id = resolve_file_id(conn, &event.file_path);
            if let Some(file_id) = file_id {
                let line = resolve_symbol_line(conn, event.symbol_id);
                points.push(ConnectionPoint {
                    file_id,
                    symbol_id: Some(event.symbol_id),
                    line,
                    protocol: Protocol::EventBus,
                    direction: FlowDirection::Start,
                    key: event.name.clone(),
                    method: String::new(),
                    framework: String::new(),
                    metadata: None,
                });
            }
        }

        // Stop points: event handler classes.
        let handlers = dotnet_events::find_event_handlers(conn, project_root)
            .context("Event handler detection failed")?;

        for handler in &handlers {
            let file_id = resolve_file_id(conn, &handler.file_path);
            if let Some(file_id) = file_id {
                let line = resolve_symbol_line(conn, handler.symbol_id);
                points.push(ConnectionPoint {
                    file_id,
                    symbol_id: Some(handler.symbol_id),
                    line,
                    protocol: Protocol::EventBus,
                    direction: FlowDirection::Stop,
                    key: handler.event_type.clone(),
                    method: String::new(),
                    framework: String::new(),
                    metadata: Some(
                        serde_json::json!({
                            "handler": handler.name,
                        })
                        .to_string(),
                    ),
                });
            }
        }

        Ok(points)
    }
}

fn resolve_file_id(conn: &Connection, rel_path: &str) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM files WHERE path = ?1",
        [rel_path],
        |r| r.get(0),
    )
    .ok()
}

fn resolve_symbol_line(conn: &Connection, symbol_id: i64) -> u32 {
    conn.query_row(
        "SELECT line FROM symbols WHERE id = ?1",
        [symbol_id],
        |r| r.get::<_, u32>(0),
    )
    .unwrap_or(0)
}
