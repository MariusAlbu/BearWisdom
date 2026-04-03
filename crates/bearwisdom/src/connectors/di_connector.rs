// =============================================================================
// connectors/di_connector.rs — Dependency Injection connectors (new architecture)
//
// Covers .NET DI (AddScoped/AddTransient/AddSingleton), Angular DI
// (@Injectable + constructor injection), and Spring DI (@Service/@Component
// implementing interfaces).
//
// Start points: interface/abstract types (the dependency being requested).
// Stop points: concrete implementations (the type that satisfies the binding).
// Key: the interface type name — ProtocolMatcher does exact key matching.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// .NET DI
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

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let registrations = super::dotnet_di::detect_di_registrations(conn, project_root)
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
            let (iface_file, iface_sym, iface_line) = iface_def
                .unwrap_or((reg.file_id, None, reg.line));

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
            let (impl_file, impl_sym, impl_line) = impl_def
                .unwrap_or((reg.file_id, None, reg.line));

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
// Angular DI
// ===========================================================================

pub struct AngularDiConnector;

impl Connector for AngularDiConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "angular_di",
            protocols: &[Protocol::Di],
            languages: &["typescript", "tsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("@angular/core")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let re_injectable = regex::Regex::new(r"@Injectable\s*\(").expect("injectable regex");
        let re_class = regex::Regex::new(r"\bclass\s+(\w+)").expect("class regex");
        let re_ctor_param = regex::Regex::new(
            r"\b(private|public|protected)\s+(?:readonly\s+)?(\w+)\s*:\s*(\w+)",
        )
        .expect("ctor param regex");

        let files = query_ts_files(conn)?;
        let mut points = Vec::new();

        // Pass 1: find @Injectable classes → stop points (providers)
        let mut injectable_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (file_id, rel_path) in &files {
            let abs_path = project_root.join(rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let lines: Vec<&str> = source.lines().collect();
            let mut i = 0;
            while i < lines.len() {
                if re_injectable.is_match(lines[i]) {
                    // Find the class declaration within the next few lines
                    for j in (i + 1)..std::cmp::min(i + 6, lines.len()) {
                        if let Some(cap) = re_class.captures(lines[j]) {
                            let class_name = cap[1].to_string();
                            let line_no = (j + 1) as u32;

                            injectable_names.insert(class_name.clone());

                            points.push(ConnectionPoint {
                                file_id: *file_id,
                                symbol_id: None,
                                line: line_no,
                                protocol: Protocol::Di,
                                direction: FlowDirection::Stop,
                                key: class_name,
                                method: String::new(),
                                framework: "angular".to_string(),
                                metadata: None,
                            });

                            i = j;
                            break;
                        }
                    }
                }
                i += 1;
            }
        }

        // Pass 2: find constructor injection sites → start points (consumers)
        for (file_id, rel_path) in &files {
            let abs_path = project_root.join(rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;
                for cap in re_ctor_param.captures_iter(line_text) {
                    let type_name = cap[3].to_string();
                    if injectable_names.contains(&type_name) {
                        points.push(ConnectionPoint {
                            file_id: *file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::Di,
                            direction: FlowDirection::Start,
                            key: type_name,
                            method: String::new(),
                            framework: "angular".to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// Spring DI
// ===========================================================================

pub struct SpringDiConnector;

impl Connector for SpringDiConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "spring_di",
            protocols: &[Protocol::Di],
            languages: &["java"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        _project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        // Spring DI works from the existing symbol graph: stereotype classes
        // that have `implements` edges to interfaces get DI bindings.
        // Query stereotype members, then follow implements edges.
        let mut points = Vec::new();

        let mut stmt = conn
            .prepare(
                "SELECT cm.symbol_id, s.name, s.file_id, s.line
                 FROM concept_members cm
                 JOIN concepts c ON c.id = cm.concept_id
                 JOIN symbols s ON s.id = cm.symbol_id
                 WHERE c.name IN (
                     'spring-services',
                     'spring-repositories',
                     'spring-components'
                 )",
            )
            .context("Failed to query Spring stereotype members")?;

        let impls: Vec<(i64, String, i64, u32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .context("Failed to execute Spring stereotype query")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Spring stereotype rows")?;

        for (impl_sym_id, impl_name, impl_file_id, impl_line) in &impls {
            // Find interfaces this class implements
            let mut iface_stmt = conn
                .prepare(
                    "SELECT tgt.name, tgt.file_id, tgt.line
                     FROM edges e
                     JOIN symbols tgt ON tgt.id = e.target_id
                     WHERE e.source_id = ?1
                       AND e.kind = 'implements'
                       AND tgt.kind = 'interface'",
                )
                .context("Failed to prepare implements query")?;

            let ifaces: Vec<(String, i64, u32)> = iface_stmt
                .query_map([impl_sym_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .context("Failed to query implements edges")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect implements rows")?;

            for (iface_name, iface_file_id, iface_line) in &ifaces {
                // Start: interface (the dependency being requested)
                points.push(ConnectionPoint {
                    file_id: *iface_file_id,
                    symbol_id: None,
                    line: *iface_line,
                    protocol: Protocol::Di,
                    direction: FlowDirection::Start,
                    key: iface_name.clone(),
                    method: String::new(),
                    framework: "spring".to_string(),
                    metadata: None,
                });

                // Stop: implementation (the type that fulfills the binding)
                points.push(ConnectionPoint {
                    file_id: *impl_file_id,
                    symbol_id: Some(*impl_sym_id),
                    line: *impl_line,
                    protocol: Protocol::Di,
                    direction: FlowDirection::Stop,
                    key: iface_name.clone(),
                    method: String::new(),
                    framework: "spring".to_string(),
                    metadata: Some(
                        serde_json::json!({
                            "implementation": impl_name,
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
// Shared helpers
// ===========================================================================

/// Look up a type name in the symbols table and return (file_id, symbol_id, line).
///
/// Prefers interfaces/classes over other symbol kinds.  Returns None if not found.
fn resolve_symbol_def(
    conn: &Connection,
    type_name: &str,
) -> Option<(i64, Option<i64>, u32)> {
    // Try exact match on symbol name, preferring interface/class kinds.
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
                row.get::<_, i64>(1)?,  // file_id
                Some(row.get::<_, i64>(0)?),  // symbol_id
                row.get::<_, u32>(2)?,  // line
            ))
        },
    )
    .ok()
}

fn query_ts_files(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
        )
        .context("Failed to prepare TS files query")?;

    let files = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS file rows")?;

    Ok(files)
}
