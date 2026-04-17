// =============================================================================
// languages/vbnet/connectors.rs — VB.NET-specific flow connectors
//
// VbNetDiConnector:
//   Scans indexed VB.NET files for .NET DI registration calls
//   (AddScoped/AddTransient/AddSingleton) and emits DI connection points.
//   VB.NET uses the same IServiceCollection extension methods as C# and F#.
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
// VbNetDiConnector
// ===========================================================================

pub struct VbNetDiConnector;

impl Connector for VbNetDiConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "vbnet_di",
            protocols: &[Protocol::Di],
            languages: &["vbnet"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.manifests.contains_key(&ManifestKind::NuGet)
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // VB.NET uses the same generic syntax as C#/F# for DI registrations.
        let re_two = Regex::new(r"Add(Scoped|Transient|Singleton)\s*\(\s*Of\s+(\w+)\s*,\s*(\w+)\s*\)")
            .expect("vbnet two-type DI regex");
        // Also handle the C#-style angle bracket form which some VB codebases use.
        let re_two_cs = Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*,\s*(\w+)\s*>")
            .expect("vbnet two-type DI angle regex");
        let re_one = Regex::new(r"Add(Scoped|Transient|Singleton)\s*\(\s*Of\s+(\w+)\s*\)")
            .expect("vbnet one-type DI regex");
        let re_one_cs = Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*>")
            .expect("vbnet one-type DI angle regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'vbnet'")
            .context("Failed to prepare VB.NET files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query VB.NET files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect VB.NET file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!(path = %abs_path.display(), err = %e, "Skipping unreadable VB.NET file");
                    continue;
                }
            };

            extract_di_points(
                &source, file_id,
                &re_two, &re_two_cs,
                &re_one, &re_one_cs,
                conn, &mut points,
            );
        }

        Ok(points)
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_di_points(
    source: &str,
    file_id: i64,
    re_two: &Regex,
    re_two_cs: &Regex,
    re_one: &Regex,
    re_one_cs: &Regex,
    conn: &Connection,
    out: &mut Vec<ConnectionPoint>,
) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Two-type form: .AddScoped(Of IFoo, Foo)  or  .AddScoped<IFoo, Foo>
        let two_match = re_two.captures(line_text)
            .or_else(|| re_two_cs.captures(line_text));

        if let Some(cap) = two_match {
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

        // One-type form: .AddSingleton(Of Foo)  or  .AddSingleton<Foo>
        let one_match = re_one.captures(line_text)
            .or_else(|| re_one_cs.captures(line_text));

        if let Some(cap) = one_match {
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
