// =============================================================================
// languages/fsharp/connectors.rs — F#-specific flow connectors
//
// FSharpDiConnector:
//   Scans indexed F# files for .NET DI registration calls
//   (AddScoped/AddTransient/AddSingleton) and emits DI connection points.
//   F# uses the same .NET DI API as C# — the IServiceCollection extension
//   methods are identical, just called from F# syntax.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// FSharpDiConnector
// ===========================================================================

pub struct FSharpDiConnector;

impl Connector for FSharpDiConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "fsharp_di",
            protocols: &[Protocol::Di],
            languages: &["fsharp"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        // Only run if this looks like a .NET project.
        ctx.manifests.contains_key(&ManifestKind::NuGet)
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_two = Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*,\s*(\w+)\s*>")
            .expect("fsharp two-type DI regex");
        let re_one = Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*>")
            .expect("fsharp one-type DI regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'fsharp'")
            .context("Failed to prepare F# files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query F# files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect F# file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!(path = %abs_path.display(), err = %e, "Skipping unreadable F# file");
                    continue;
                }
            };

            extract_di_points(&source, file_id, &re_two, &re_one, conn, &mut points);
        }

        Ok(points)
    }
}

fn extract_di_points(
    source: &str,
    file_id: i64,
    re_two: &Regex,
    re_one: &Regex,
    conn: &Connection,
    out: &mut Vec<ConnectionPoint>,
) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Two-type form: AddScoped<IFoo, Foo>
        if let Some(cap) = re_two.captures(line_text) {
            let lifetime = cap[1].to_lowercase();
            let iface = cap[2].to_string();
            let impl_type = cap[3].to_string();

            if iface == impl_type {
                continue;
            }

            let metadata = serde_json::json!({
                "lifetime": lifetime,
                "implementation": impl_type,
            })
            .to_string();

            let iface_def = resolve_symbol_def(conn, &iface);
            let impl_def = resolve_symbol_def(conn, &impl_type);

            let (iface_file, iface_sym, iface_line) =
                iface_def.unwrap_or((file_id, None, line_no));
            out.push(ConnectionPoint {
                file_id: iface_file,
                symbol_id: iface_sym,
                line: iface_line,
                protocol: Protocol::Di,
                direction: FlowDirection::Start,
                key: iface.clone(),
                method: String::new(),
                framework: "dotnet".to_string(),
                metadata: Some(metadata.clone()),
            });

            let (impl_file, impl_sym, impl_line) =
                impl_def.unwrap_or((file_id, None, line_no));
            out.push(ConnectionPoint {
                file_id: impl_file,
                symbol_id: impl_sym,
                line: impl_line,
                protocol: Protocol::Di,
                direction: FlowDirection::Stop,
                key: iface,
                method: String::new(),
                framework: "dotnet".to_string(),
                metadata: Some(metadata),
            });

            continue;
        }

        // One-type form: AddSingleton<Foo>
        if let Some(cap) = re_one.captures(line_text) {
            let lifetime = cap[1].to_lowercase();
            let impl_type = cap[2].to_string();

            let metadata = serde_json::json!({ "lifetime": lifetime }).to_string();
            let impl_def = resolve_symbol_def(conn, &impl_type);
            let (impl_file, impl_sym, impl_line) =
                impl_def.unwrap_or((file_id, None, line_no));

            out.push(ConnectionPoint {
                file_id: impl_file,
                symbol_id: impl_sym,
                line: impl_line,
                protocol: Protocol::Di,
                direction: FlowDirection::Stop,
                key: impl_type,
                method: String::new(),
                framework: "dotnet".to_string(),
                metadata: Some(metadata),
            });
        }
    }
}

fn resolve_symbol_def(conn: &Connection, name: &str) -> Option<(i64, Option<i64>, u32)> {
    conn.query_row(
        "SELECT s.file_id, s.id, COALESCE(s.line, 0)
         FROM symbols s
         WHERE s.name = ?1
           AND s.kind IN ('class', 'interface', 'struct')
         LIMIT 1",
        [name],
        |row| Ok((row.get::<_, i64>(0)?, Some(row.get::<_, i64>(1)?), row.get::<_, u32>(2)?)),
    )
    .ok()
}
