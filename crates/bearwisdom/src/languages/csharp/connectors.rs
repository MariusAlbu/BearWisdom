// =============================================================================
// languages/csharp/connectors.rs — C#-specific flow connectors
//
// Owns both the .NET DI connector and the integration event bus connector.
// All detection helpers (regex scanning, DB queries) live here alongside the
// connector implementations so the language plugin is fully self-contained.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// DotnetDiConnector
// ===========================================================================

pub struct DotnetDiConnector;

impl Connector for DotnetDiConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "dotnet_di",
            protocols: &[Protocol::Di],
            languages: &["csharp"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        !ctx.external_prefixes.is_empty()
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let registrations = detect_di_registrations(conn, project_root)
            .context(".NET DI registration detection failed")?;

        let mut points = Vec::new();

        for reg in &registrations {
            // Skip self-registrations — no interface/impl distinction.
            if reg.interface_type == reg.implementation_type {
                continue;
            }

            let metadata = serde_json::json!({
                "lifetime": reg.lifetime,
                "implementation": reg.implementation_type,
            })
            .to_string();

            // Resolve interface → its definition site in the symbols table.
            let iface_def = resolve_symbol_def(conn, &reg.interface_type);
            // Resolve concrete impl → its definition site.
            let impl_def = resolve_symbol_def(conn, &reg.implementation_type);

            // Start: the interface definition (the dependency being requested).
            // Fall back to registration site if the interface isn't in the symbol table.
            let (iface_file, iface_sym, iface_line) =
                iface_def.unwrap_or((reg.file_id, None, reg.line));

            points.push(ConnectionPoint {
                file_id: iface_file,
                symbol_id: iface_sym,
                line: iface_line,
                protocol: Protocol::Di,
                direction: FlowDirection::Start,
                key: reg.interface_type.clone(),
                method: String::new(),
                framework: "dotnet".to_string(),
                metadata: Some(metadata.clone()),
            });

            // Stop: the implementation definition (the type that fulfills the binding).
            let (impl_file, impl_sym, impl_line) =
                impl_def.unwrap_or((reg.file_id, None, reg.line));

            points.push(ConnectionPoint {
                file_id: impl_file,
                symbol_id: impl_sym,
                line: impl_line,
                protocol: Protocol::Di,
                direction: FlowDirection::Stop,
                key: reg.interface_type.clone(),
                method: String::new(),
                framework: "dotnet".to_string(),
                metadata: Some(metadata),
            });
        }

        Ok(points)
    }
}

// ===========================================================================
// EventBusConnector
// ===========================================================================

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

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();

        // Start points: integration event classes.
        let events =
            find_integration_events(conn).context("Integration event detection failed")?;

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
        let handlers =
            find_event_handlers(conn, project_root).context("Event handler detection failed")?;

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

// ===========================================================================
// DI helpers (from connectors/dotnet_di.rs)
// ===========================================================================

/// A single DI registration detected in a C# file.
#[derive(Debug, Clone)]
pub struct DiRegistration {
    /// `files.id` of the file containing the registration call.
    pub file_id: i64,
    /// 1-based line number of the call.
    pub line: u32,
    /// Lifetime of the registration.
    pub lifetime: String,
    /// The interface type — for the single-type form this equals `implementation_type`.
    pub interface_type: String,
    /// The concrete implementation type.
    pub implementation_type: String,
}

/// Scan all indexed C# files for DI registrations.
///
/// Returns detected registrations with file_id and line metadata.
/// Files are read from disk via `project_root`.
pub fn detect_di_registrations(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<DiRegistration>> {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'csharp'")
        .context("Failed to prepare C# files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query C# files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect C# file rows")?;

    let mut registrations: Vec<DiRegistration> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable C# file");
                continue;
            }
        };

        detect_in_source(&source, file_id, &re_two, &re_one, &mut registrations);
    }

    debug!(count = registrations.len(), "DI registrations detected");
    Ok(registrations)
}

/// Link detected DI registrations to the symbol graph.
///
/// For two-type registrations: creates an `implements` edge from the
/// implementation class to the interface.  For single-type registrations
/// (self-registration) no edge is created.
///
/// Returns the number of edges inserted.
pub fn link_di_registrations(
    conn: &Connection,
    registrations: &[DiRegistration],
) -> Result<u32> {
    let mut created: u32 = 0;

    for reg in registrations {
        if reg.interface_type == reg.implementation_type {
            continue;
        }

        let iface_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM symbols WHERE name = ?1 AND kind = 'interface' LIMIT 1",
                [&reg.interface_type],
                |r| r.get(0),
            )
            .optional();

        let impl_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM symbols WHERE name = ?1 AND kind = 'class' LIMIT 1",
                [&reg.implementation_type],
                |r| r.get(0),
            )
            .optional();

        let (iface_id, impl_id) = match (iface_id, impl_id) {
            (Some(i), Some(c)) => (i, c),
            _ => {
                debug!(
                    interface = %reg.interface_type,
                    implementation = %reg.implementation_type,
                    "Skipping DI registration — symbol(s) not found"
                );
                continue;
            }
        };

        let metadata = serde_json::json!({
            "lifetime": reg.lifetime,
            "source": "di_registration",
        })
        .to_string();

        let result = conn.execute(
            "INSERT OR IGNORE INTO edges
                (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'implements', ?3, 0.90)",
            rusqlite::params![impl_id, iface_id, reg.line],
        );

        match result {
            Ok(n) if n > 0 => {
                created += 1;
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, confidence, metadata
                     ) SELECT
                        si.file_id, ?3, ?4, 'csharp',
                        sf.file_id, NULL, ?5, 'csharp',
                        'di_binding', 0.90, ?6
                     FROM symbols si, symbols sf
                     WHERE si.id = ?1 AND sf.id = ?2",
                    rusqlite::params![
                        impl_id,
                        iface_id,
                        reg.line,
                        reg.implementation_type,
                        reg.interface_type,
                        metadata
                    ],
                );
            }
            Ok(_) => {}
            Err(e) => {
                debug!(err = %e, "Failed to insert implements edge for DI registration");
            }
        }
    }

    info!(created, "DI connector: linked registrations to symbol graph");
    Ok(created)
}

/// Regex for the two-type form: `Add{Lifetime}<Interface, Implementation>`.
fn build_two_type_regex() -> Regex {
    Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*,\s*(\w+)\s*>")
        .expect("two-type DI regex is valid")
}

/// Regex for the one-type form: `Add{Lifetime}<Implementation>`.
fn build_one_type_regex() -> Regex {
    Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*>")
        .expect("one-type DI regex is valid")
}

fn detect_in_source(
    source: &str,
    file_id: i64,
    re_two: &Regex,
    re_one: &Regex,
    out: &mut Vec<DiRegistration>,
) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        if let Some(cap) = re_two.captures(line_text) {
            out.push(DiRegistration {
                file_id,
                line: line_no,
                lifetime: cap[1].to_lowercase(),
                interface_type: cap[2].to_string(),
                implementation_type: cap[3].to_string(),
            });
            continue;
        }

        if let Some(cap) = re_one.captures(line_text) {
            let type_name = cap[2].to_string();
            out.push(DiRegistration {
                file_id,
                line: line_no,
                lifetime: cap[1].to_lowercase(),
                interface_type: type_name.clone(),
                implementation_type: type_name,
            });
        }
    }
}

// ===========================================================================
// Event bus helpers (from connectors/dotnet_events.rs)
// ===========================================================================

/// A class that inherits from `IntegrationEvent`.
#[derive(Debug, Clone)]
pub struct IntegrationEvent {
    pub symbol_id: i64,
    pub name: String,
    pub file_path: String,
}

/// A class that implements `IIntegrationEventHandler<T>`.
#[derive(Debug, Clone)]
pub struct EventHandler {
    pub symbol_id: i64,
    pub name: String,
    /// The `T` in `IIntegrationEventHandler<T>`.
    pub event_type: String,
    pub file_path: String,
}

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
pub fn find_event_handlers(conn: &Connection, project_root: &Path) -> Result<Vec<EventHandler>> {
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
        let event = match events.iter().find(|e| e.name == handler.event_type) {
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

fn build_handler_regex() -> Regex {
    Regex::new(r"IIntegrationEventHandler\s*<\s*(\w+)\s*>").expect("handler regex is valid")
}

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

            let class_name = extract_class_name_from_line(line_text);

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

            let (name, symbol_id) =
                if let (Some(cn), Some(sid)) = (class_name.clone(), symbol_id) {
                    (cn, sid)
                } else {
                    let nearby: Option<(String, i64)> = conn
                        .query_row(
                            "SELECT s.name, s.id FROM symbols s
                             JOIN files f ON f.id = s.file_id
                             WHERE f.path = ?1 AND s.kind = 'class'
                               AND s.line BETWEEN ?2 AND ?3
                             ORDER BY ABS(s.line - ?4) LIMIT 1",
                            rusqlite::params![
                                rel_path,
                                line_no.saturating_sub(5),
                                line_no + 5,
                                line_no
                            ],
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

fn extract_class_name_from_line(line: &str) -> Option<String> {
    let re = Regex::new(r"\bclass\s+(\w+)").expect("class name regex is valid");
    re.captures(line).map(|c| c[1].to_string())
}

// ===========================================================================
// Shared private helpers
// ===========================================================================

fn resolve_symbol_def(conn: &Connection, type_name: &str) -> Option<(i64, Option<i64>, u32)> {
    conn.query_row(
        "SELECT id, file_id, line FROM symbols
         WHERE name = ?1
         ORDER BY CASE kind
             WHEN 'interface' THEN 0
             WHEN 'class' THEN 1
             WHEN 'struct' THEN 2
             ELSE 3 END,
             id ASC
         LIMIT 1",
        [type_name],
        |row| {
            Ok((
                row.get::<_, i64>(1)?, // file_id
                Some(row.get::<_, i64>(0)?), // symbol_id
                row.get::<_, u32>(2)?, // line
            ))
        },
    )
    .ok()
}

fn resolve_file_id(conn: &Connection, rel_path: &str) -> Option<i64> {
    conn.query_row("SELECT id FROM files WHERE path = ?1", [rel_path], |r| {
        r.get(0)
    })
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

// ===========================================================================
// CSharpGrpcConnector — gRPC service implementation stops
// ===========================================================================

/// Emits gRPC Stop connection points for C# service implementations.
///
/// Looks for methods in classes named `{ServiceName}Base` or `{ServiceName}`
/// that match RPC names extracted from .proto files (Start points come from
/// the proto language plugin via ProtoGrpcConnector).
///
/// The matching key is "ServiceName.RpcName", consistent with the proto side.
pub struct CSharpGrpcConnector;

impl Connector for CSharpGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "csharp_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["csharp"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        // Detect if there are any proto files or gRPC NuGet deps.
        // We run cheaply; the extract step returns nothing when there are no
        // proto services in the index, so false positives are fine.
        true
    }

    fn extract(&self, conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();

        // Find all (service_name, rpc_name) pairs from proto services in the DB
        // by querying symbols of kind 'method' under files whose language is 'protobuf'.
        // We don't re-parse proto files here; instead we look for C# classes
        // matching the known pattern and query their methods.
        //
        // Strategy: find classes named *Base or *Service in C# files, then find
        // their methods. The ProtocolMatcher will key-match to proto start points.
        let services = grpc_find_csharp_service_classes(conn)?;

        for (class_name, cs_file_id) in &services {
            // Strip the conventional "Base" suffix to get the proto service name.
            let service_name = class_name
                .strip_suffix("Base")
                .unwrap_or(class_name.as_str());

            grpc_emit_csharp_rpc_stops(conn, service_name, *cs_file_id, &mut points)?;
        }

        Ok(points)
    }
}

/// Find C# classes that look like gRPC service implementations.
///
/// Matches classes whose name ends with "Base" or "GrpcService" (Grpc.AspNetCore pattern).
/// Returns (class_name, file_id) pairs.
fn grpc_find_csharp_service_classes(
    conn: &Connection,
) -> Result<Vec<(String, i64)>> {
    // Look for classes that appear to inherit from a gRPC generated base.
    // We check the proto services registered in the DB and look for matching C# classes.
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.name, s.file_id
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.language = 'csharp'
               AND s.kind = 'class'
               AND (s.name LIKE '%Base' OR s.name LIKE '%GrpcService'
                    OR EXISTS (
                        SELECT 1 FROM edges e
                        JOIN symbols tgt ON tgt.id = e.target_id
                        WHERE e.source_id = s.id
                          AND e.kind = 'inherits'
                          AND tgt.name LIKE '%Base'
                    ))",
        )
        .context("Failed to prepare gRPC service class query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .context("Failed to execute gRPC service class query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect gRPC service class rows")?;

    Ok(rows)
}

/// Emit a Stop connection point for each method in a C# gRPC service class.
fn grpc_emit_csharp_rpc_stops(
    conn: &Connection,
    service_name: &str,
    cs_file_id: i64,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, s.line
             FROM symbols s
             WHERE s.file_id = ?1 AND s.kind = 'method'",
        )
        .context("Failed to prepare C# RPC method query")?;

    let methods: Vec<(i64, String, u32)> = stmt
        .query_map(rusqlite::params![cs_file_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })
        .context("Failed to query C# RPC methods")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect C# RPC method rows")?;

    for (sym_id, method_name, line) in methods {
        // Skip constructors, Dispose, and other non-RPC methods heuristically.
        if method_name.starts_with('.') || method_name == "Dispose" || method_name == "ToString" {
            continue;
        }

        let key = format!("{service_name}.{method_name}");
        out.push(ConnectionPoint {
            file_id: cs_file_id,
            symbol_id: Some(sym_id),
            line,
            protocol: Protocol::Grpc,
            direction: FlowDirection::Stop,
            key,
            method: String::new(),
            framework: "grpc_aspnetcore".to_string(),
            metadata: None,
        });
    }

    Ok(())
}

// ===========================================================================
// CSharpMqConnector — Message queue producer/consumer stops
// ===========================================================================

/// Detects C# message queue producers and consumers using common .NET MQ
/// library patterns: MassTransit, NServiceBus, Azure Service Bus, RabbitMQ.Client,
/// Confluent.Kafka.
pub struct CSharpMqConnector;

impl Connector for CSharpMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "csharp_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["csharp"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        // Detect known .NET MQ packages.
        let deps = &ctx.external_prefixes;
        deps.contains("MassTransit")
            || deps.contains("NServiceBus")
            || deps.contains("Azure.Messaging.ServiceBus")
            || deps.contains("RabbitMQ")
            || deps.contains("Confluent")
            || deps.contains("Confluent.Kafka")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Producer patterns:
        //   await publishEndpoint.Publish<T>(...)  or  await bus.Publish<T>(...)  — MassTransit
        //   context.Send(new QueueAddress("..."))
        //   serviceBusSender.SendMessageAsync(...)
        //   channel.BasicPublish(exchange: "name", routingKey: "key", ...)
        //   producer.ProduceAsync("topic", ...)   — Confluent.Kafka
        //
        // Consumer patterns (class-level attributes or interface implementation):
        //   : IConsumer<T>           — MassTransit consumer
        //   : IHandleMessages<T>     — NServiceBus
        //   [ServiceBusTrigger("queue")] — Azure Functions
        //   channel.BasicConsume("queue-name", ...)

        let re_publish = regex::Regex::new(
            r#"(?:publishEndpoint|bus|_bus|endpoint|sender|_sender)\.(?:Publish|Send)\s*[<(]"#,
        )
        .expect("csharp mq publish regex");

        let re_service_bus_send = regex::Regex::new(
            r#"\.SendMessageAsync\s*\(|\.SendAsync\s*\("#,
        )
        .expect("csharp service bus send regex");

        let re_rabbit_publish = regex::Regex::new(
            r#"\.BasicPublish\s*\([^)]*(?:exchange|routingKey)\s*[=:]\s*['"]([^'"]+)['"]"#,
        )
        .expect("csharp rabbit publish regex");

        let re_kafka_produce = regex::Regex::new(
            r#"\.ProduceAsync\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("csharp kafka produce regex");

        let re_iconsumer = regex::Regex::new(
            r#":\s*IConsumer\s*<\s*(\w+)\s*>"#,
        )
        .expect("csharp iconsumer regex");

        let re_service_bus_trigger = regex::Regex::new(
            r#"\[ServiceBusTrigger\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("csharp service bus trigger regex");

        let re_rabbit_consume = regex::Regex::new(
            r#"\.BasicConsume\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("csharp rabbit consume regex");

        let re_kafka_subscribe = regex::Regex::new(
            r#"\.Subscribe\s*\(\s*(?:new\s*\[\s*\]\s*\{)?\s*['"]([^'"]+)['"]"#,
        )
        .expect("csharp kafka subscribe regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'csharp'")
            .context("Failed to prepare C# files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query C# files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect C# file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                // Generic publish/send (MassTransit, NServiceBus)
                if re_publish.is_match(line_text) || re_service_bus_send.is_match(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: "message".to_string(),
                        method: String::new(),
                        framework: "dotnet_mq".to_string(),
                        metadata: None,
                    });
                }

                // RabbitMQ BasicPublish
                for cap in re_rabbit_publish.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                // Confluent.Kafka ProduceAsync
                for cap in re_kafka_produce.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }

                // MassTransit IConsumer<T>
                for cap in re_iconsumer.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "dotnet_mq".to_string(),
                        metadata: None,
                    });
                }

                // Azure Service Bus trigger
                for cap in re_service_bus_trigger.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "azure_service_bus".to_string(),
                        metadata: None,
                    });
                }

                // RabbitMQ BasicConsume
                for cap in re_rabbit_consume.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                // Confluent.Kafka Subscribe
                for cap in re_kafka_subscribe.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// CSharpGraphQlConnector — GraphQL resolver stops
// ===========================================================================

/// Detects C# Hot Chocolate / Strawberry Shake GraphQL resolvers.
///
/// Start points come from the GraphQL schema connector (graphql language plugin).
/// This connector emits Stop points: methods decorated with [GraphQLQuery],
/// [GraphQLMutation], [QueryType], [MutationType], or belonging to a type
/// registered via `descriptor.AddQueryType<T>()`.
pub struct CSharpGraphQlConnector;

impl Connector for CSharpGraphQlConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "csharp_graphql_resolvers",
            protocols: &[Protocol::GraphQl],
            languages: &["csharp"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        let deps = &ctx.external_prefixes;
        deps.contains("HotChocolate")
            || deps.contains("StrawberryShake")
            || deps.contains("GraphQL")
    }

    fn extract(&self, conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Find methods in classes that are marked as GraphQL types.
        // Hot Chocolate: [QueryType] / [MutationType] on class,
        // methods become resolvers by convention (name = field name).
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.file_id, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'csharp'
                   AND s.kind = 'method'
                   AND EXISTS (
                       SELECT 1 FROM symbols p
                       WHERE p.file_id = s.file_id
                         AND p.kind = 'class'
                         AND p.name LIKE '%Query%' OR p.name LIKE '%Mutation%'
                         OR p.name LIKE '%Subscription%'
                   )",
            )
            .context("Failed to prepare C# GraphQL resolver query")?;

        let rows: Vec<(i64, String, i64, u32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .context("Failed to query C# GraphQL resolvers")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect C# GraphQL resolver rows")?;

        let points = rows
            .into_iter()
            .map(|(sym_id, name, file_id, line)| ConnectionPoint {
                file_id,
                symbol_id: Some(sym_id),
                line,
                protocol: Protocol::GraphQl,
                direction: FlowDirection::Stop,
                key: name,
                method: String::new(),
                framework: "hotchocolate".to_string(),
                metadata: None,
            })
            .collect();

        Ok(points)
    }
}

// ===========================================================================
// CsharpRestConnector — HTTP client call starts + route stops for C#
// ===========================================================================

pub struct CsharpRestConnector;

impl Connector for CsharpRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "csharp_rest",
            protocols: &[Protocol::Rest],
            languages: &["csharp"],
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
        let mut points = Vec::new();
        extract_csharp_rest_stops(conn, &mut points)?;
        extract_csharp_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

fn extract_csharp_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'csharp'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare C# REST stops query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("Failed to query C# routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read C# route row")?;
        out.push(ConnectionPoint {
            file_id,
            symbol_id,
            line: line.unwrap_or(0),
            protocol: Protocol::Rest,
            direction: FlowDirection::Stop,
            key: route,
            method: method.to_uppercase(),
            framework: String::new(),
            metadata: None,
        });
    }
    Ok(())
}

fn extract_csharp_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // HttpClient.GetAsync("url"), PostAsync, PutAsync, DeleteAsync
    let re_direct = Regex::new(
        r#"HttpClient\s*\.\s*(?P<method>Get|Post|Put|Delete|Patch)Async\s*\(\s*(?:"(?P<url1>[^"]+)"|@?"(?P<url2>[^"]+)")"#,
    ).expect("csharp httpclient regex");

    // _provider.GetAsync<T>("url") or _provider.GetAsync<T>($"url")
    let re_wrapper = Regex::new(
        r#"(?:_\w+|this\.\w+)\s*\.\s*(?P<method>Get|Post|Put|Delete|Patch)Async\s*(?:<[^>]*>)?\s*\(\s*(?:(?:"(?P<url>[^"]+)")|(?:\$"(?P<interp>[^"]+)"))"#,
    ).expect("csharp wrapper regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'csharp'")
        .context("Failed to prepare C# files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query C# files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect C# file rows")?;

    for (file_id, rel_path) in files {
        if csharp_rest_is_test_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Collect API URL base constants from the file for interpolation.
        let re_api_const = Regex::new(
            r#"(?:const|static\s+readonly)\s+string\s+\w*(?:Api|Url|Base|Endpoint)\w*\s*=\s*"([^"]+)""#,
        ).expect("api const regex");
        let api_bases: Vec<String> = re_api_const
            .captures_iter(&source)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            // HttpClient.XAsync("url")
            if let Some(cap) = re_direct.captures(line_text) {
                let method = cap.name("method").map(|m| m.as_str()).unwrap_or("GET");
                let url = cap.name("url1").or_else(|| cap.name("url2")).map(|m| m.as_str());
                if let Some(url) = url {
                    if csharp_rest_looks_like_api_url(url) {
                        let url_pattern = csharp_normalise_interp_url(url, &api_bases);
                        out.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::Rest,
                            direction: FlowDirection::Start,
                            key: url_pattern,
                            method: method.to_uppercase(),
                            framework: "dotnet_http".to_string(),
                            metadata: None,
                        });
                    }
                }
            }

            // _wrapper.XAsync<T>("url") or _wrapper.XAsync<T>($"url")
            if let Some(cap) = re_wrapper.captures(line_text) {
                let method = cap.name("method").map(|m| m.as_str()).unwrap_or("GET");
                let url = cap.name("url").or_else(|| cap.name("interp")).map(|m| m.as_str());
                if let Some(url) = url {
                    if csharp_rest_looks_like_api_url(url) {
                        let url_pattern = csharp_normalise_interp_url(url, &api_bases);
                        out.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::Rest,
                            direction: FlowDirection::Start,
                            key: url_pattern,
                            method: method.to_uppercase(),
                            framework: "dotnet_http".to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn csharp_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("test") || lower.contains("spec")
}

fn csharp_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() { return false; }
        return csharp_rest_looks_like_api_url(path);
    }
    s.starts_with('/') || s.contains("/api/") || s.contains("/v1/") || s.contains("/v2/") || s.contains("/{")
}

fn csharp_normalise_interp_url(url: &str, api_bases: &[String]) -> String {
    let mut result = url.to_string();
    for value in api_bases {
        if !value.is_empty() {
            result = result.replace("{ApiUrlBase}", value);
            result = result.replace("{ApiUrl}", value);
        }
    }
    let re_interp = Regex::new(r"\{[^}]+\}").expect("interp regex");
    let result = re_interp.replace_all(&result, "{*}").to_string();
    let result = result.split('?').next().unwrap_or(&result).to_string();
    result.replace("//", "/")
}

// ===========================================================================
// EF Core post-index hook
// ===========================================================================

/// Post-index enrichment for EF Core: apply convention pluralisation and
/// create `db_entity` edges.
///
/// Called from `CSharpPlugin::post_index()` after all symbols have been written.
pub fn run_ef_core(db: &crate::db::Database) -> anyhow::Result<()> {
    crate::connectors::ef_core::connect(db)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
#[path = "connectors_di_tests.rs"]
mod di_tests;

#[cfg(test)]
#[path = "connectors_events_tests.rs"]
mod events_tests;
