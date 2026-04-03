// =============================================================================
// connectors/mq_connector.rs — Message queue connector (new architecture)
//
// Wraps the existing message_queue.rs logic.
//
// Start points: producer sites (producer.send, kafkaTemplate.send, etc.)
// Stop points: consumer sites (@KafkaListener, @RabbitListener, etc.)
//
// The matching key is the topic/queue name.  ProtocolMatcher uses exact
// matching with `require_framework = true` so Kafka producers only match
// Kafka consumers, not RabbitMQ consumers on the same topic name.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::message_queue;
use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

pub struct MessageQueueConnector;

impl Connector for MessageQueueConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "message_queue",
            protocols: &[Protocol::MessageQueue],
            languages: &[
                "java", "kotlin", "python", "typescript", "tsx",
                "javascript", "jsx", "go", "csharp", "rust",
            ],
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
        let endpoints = message_queue::detect_queue_endpoints(conn, project_root)
            .context("Message queue endpoint detection failed")?;

        let points = endpoints
            .into_iter()
            .map(|ep| {
                let direction = if ep.role == "producer" {
                    FlowDirection::Start
                } else {
                    FlowDirection::Stop
                };

                ConnectionPoint {
                    file_id: ep.file_id,
                    symbol_id: None,
                    line: ep.line,
                    protocol: Protocol::MessageQueue,
                    direction,
                    key: ep.topic_or_queue,
                    method: String::new(),
                    framework: ep.framework,
                    metadata: None,
                }
            })
            .collect();

        Ok(points)
    }
}
