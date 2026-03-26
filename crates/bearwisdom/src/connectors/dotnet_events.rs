// =============================================================================
// connectors/dotnet_events.rs  —  .NET integration event connector
//
// Detects integration events (classes inheriting IntegrationEvent) and their
// handlers (classes implementing IIntegrationEventHandler<T>).  Creates
// flow_edges linking each event class to every handler that processes it.
//
// Detection strategy:
//   Events:   query the edges table for symbols with an `inherits` edge
//             pointing to a symbol named `IntegrationEvent`.
//   Handlers: search C# source files for the pattern
//             `IIntegrationEventHandler<EventType>` in class declarations.
//             The generic argument is extracted as the event type name.
//
// The file scan for handlers is done via regex on the raw source text.
// A tree-sitter pass would be more accurate but is not necessary for the
// `IIntegrationEventHandler<T>` pattern which is highly distinctive.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A class that inherits from `IntegrationEvent`.
#[derive(Debug, Clone)]
pub struct IntegrationEvent {
    /// `symbols.id` of the event class.
    pub symbol_id: i64,
    /// Simple class name (e.g. `OrderCreatedIntegrationEvent`).
    pub name: String,
    /// Relative path of the file containing the class.
    pub file_path: String,
}

/// A class that implements `IIntegrationEventHandler<T>`.
#[derive(Debug, Clone)]
pub struct EventHandler {
    /// `symbols.id` of the handler class.
    pub symbol_id: i64,
    /// Simple class name of the handler.
    pub name: String,
    /// The `T` in `IIntegrationEventHandler<T>`.
    pub event_type: String,
    /// Relative path of the file containing the handler.
    pub file_path: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Find all symbols that have an `inherits` edge pointing to `IntegrationEvent`.
pub fn find_integration_events(conn: &Connection) -> Result<Vec<IntegrationEvent>> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.kind = 'class'
               AND EXISTS (
                   SELECT 1 FROM edges e
                   JOIN symbols tgt ON e.target_id = tgt.id
                   WHERE e.source_id = s.id
                     AND e.kind = 'inherits'
                     AND (tgt.name = 'IntegrationEvent'
                          OR tgt.qualified_name LIKE '%IntegrationEvent')
               )",
        )
        .context("Failed to prepare integration events query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok(IntegrationEvent {
                symbol_id: row.get(0)?,
                name: row.get(1)?,
                file_path: row.get(2)?,
            })
        })
        .context("Failed to execute integration events query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect integration event rows")?;

    debug!(count = rows.len(), "Integration events found via edges");
    Ok(rows)
}

/// Find all classes that implement `IIntegrationEventHandler<T>` in C# files.
///
/// Each handler record includes the event type name extracted from the generic
/// argument.  Files are read from disk using `project_root`.
pub fn find_event_handlers(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<EventHandler>> {
    let re_handler = build_handler_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'csharp'")
        .context("Failed to prepare C# files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query C# files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect C# file rows")?;

    let mut handlers: Vec<EventHandler> = Vec::new();

    for (_file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable C# file");
                continue;
            }
        };

        extract_handlers_from_source(conn, &source, &rel_path, &re_handler, &mut handlers);
    }

    debug!(count = handlers.len(), "Event handlers found");
    Ok(handlers)
}

/// Match events to their handlers and create flow_edges.
///
/// Returns the number of flow_edges inserted.
pub fn link_events_to_handlers(
    conn: &Connection,
    events: &[IntegrationEvent],
    handlers: &[EventHandler],
) -> Result<u32> {
    if events.is_empty() || handlers.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for handler in handlers {
        // Find the event whose name matches the handler's generic argument.
        let matching_event = events.iter().find(|e| e.name == handler.event_type);

        let event = match matching_event {
            Some(e) => e,
            None => {
                debug!(
                    handler = %handler.name,
                    event_type = %handler.event_type,
                    "No matching integration event found for handler"
                );
                continue;
            }
        };

        // Resolve file IDs from the event and handler paths.
        let event_file_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                [&event.file_path],
                |r| r.get(0),
            )
            .optional();

        let handler_file_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                [&handler.file_path],
                |r| r.get(0),
            )
            .optional();

        let (event_file_id, handler_file_id) = match (event_file_id, handler_file_id) {
            (Some(e), Some(h)) => (e, h),
            _ => {
                debug!(
                    event = %event.name,
                    handler = %handler.name,
                    "Could not resolve file IDs for event/handler pair"
                );
                continue;
            }
        };

        let metadata = serde_json::json!({
            "event": event.name,
            "handler": handler.name,
        })
        .to_string();

        let result = conn.execute(
            "INSERT OR IGNORE INTO flow_edges (
                source_file_id, source_line, source_symbol, source_language,
                target_file_id, target_line, target_symbol, target_language,
                edge_type, confidence, metadata
             ) VALUES (
                ?1, NULL, ?2, 'csharp',
                ?3, NULL, ?4, 'csharp',
                'event_handler', 0.90, ?5
             )",
            rusqlite::params![
                event_file_id,
                event.name,
                handler_file_id,
                handler.name,
                metadata,
            ],
        );

        match result {
            Ok(n) if n > 0 => created += 1,
            Ok(_) => {}
            Err(e) => {
                debug!(err = %e, "Failed to insert event_handler flow_edge");
            }
        }
    }

    info!(created, "Events connector: linked events to handlers");
    Ok(created)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Regex matching `IIntegrationEventHandler<TypeName>` in class declarations.
fn build_handler_regex() -> Regex {
    // Matches the interface in a `class Foo : ..., IIntegrationEventHandler<Bar>` context.
    // Group 1: the event type name.
    Regex::new(r"IIntegrationEventHandler\s*<\s*(\w+)\s*>")
        .expect("handler regex is valid")
}

/// Scan source text for handler classes and cross-reference with the symbols table.
fn extract_handlers_from_source(
    conn: &Connection,
    source: &str,
    rel_path: &str,
    re_handler: &Regex,
    out: &mut Vec<EventHandler>,
) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        for cap in re_handler.captures_iter(line_text) {
            let event_type = cap[1].to_string();

            // Extract the class name from the same line — look for `class ClassName`.
            let class_name = extract_class_name_from_line(line_text);

            // Try to find the symbol in the DB for this file.
            let symbol_id: Option<i64> = class_name.as_deref().and_then(|cn| {
                conn.query_row(
                    "SELECT s.id FROM symbols s
                     JOIN files f ON f.id = s.file_id
                     WHERE s.name = ?1 AND f.path = ?2 AND s.kind = 'class'
                     LIMIT 1",
                    rusqlite::params![cn, rel_path],
                    |r| r.get(0),
                )
                .optional()
            });

            // If we couldn't match via the same line, look nearby (within 5 lines).
            let (name, symbol_id) = if let (Some(cn), Some(sid)) =
                (class_name.clone(), symbol_id)
            {
                (cn, sid)
            } else {
                // Fall back: find a class symbol near this line in this file.
                let nearby: Option<(String, i64)> = conn
                    .query_row(
                        "SELECT s.name, s.id FROM symbols s
                         JOIN files f ON f.id = s.file_id
                         WHERE f.path = ?1 AND s.kind = 'class'
                           AND s.line BETWEEN ?2 AND ?3
                         ORDER BY ABS(s.line - ?4) LIMIT 1",
                        rusqlite::params![rel_path, line_no.saturating_sub(5), line_no + 5, line_no],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .optional();

                match nearby {
                    Some((n, sid)) => (n, sid),
                    None => continue,
                }
            };

            out.push(EventHandler {
                symbol_id,
                name,
                event_type,
                file_path: rel_path.to_string(),
            });
        }
    }
}

/// Extract the class name from a line that contains `class ClassName`.
fn extract_class_name_from_line(line: &str) -> Option<String> {
    let re = Regex::new(r"\bclass\s+(\w+)").expect("class name regex is valid");
    re.captures(line).map(|c| c[1].to_string())
}

// ---------------------------------------------------------------------------
// Extension trait for rusqlite::Connection
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> Option<T>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "dotnet_events_tests.rs"]
mod tests;
