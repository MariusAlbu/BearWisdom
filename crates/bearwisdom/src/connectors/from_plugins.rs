// =============================================================================
// connectors/from_plugins.rs — bridge plugin-emitted connection points into
// the registry's matcher pipeline
//
// Language plugins emit `crate::types::ConnectionPoint` during extraction (see
// `LanguagePlugin::extract_connection_points`). Those values are *abstract* —
// they carry symbol qualified names, not DB `file_id` / `symbol_id` rows.
// The matcher works on `connectors::types::ConnectionPoint` which is
// DB-shaped.
//
// This module does the one-way join from abstract → DB-shaped. It's invoked
// once during the registry run, after user + external files are written to
// the DB but before matching fires, so every plugin-emitted point gets a
// concrete `file_id` and (best-effort) `symbol_id`.
// =============================================================================

use std::collections::HashMap;

use crate::types::{
    ConnectionKind, ConnectionPoint as AbstractPoint, ConnectionRole, ParsedFile,
};

use super::types::{ConnectionPoint as DbPoint, FlowDirection, Protocol};

/// Convert every `ParsedFile::connection_points` entry across `parsed` into
/// the DB-shaped `connectors::types::ConnectionPoint` the matcher consumes.
///
/// `file_id_map` is the path → file_id map returned by `write_parsed_files`.
/// `symbol_id_map` is the (path, qname) → symbol_id map returned alongside.
///
/// Points whose file isn't in `file_id_map` are dropped silently — this
/// happens for virtual external files that were parsed but written with a
/// different `origin` than the caller expects. Symbol IDs are best-effort:
/// missing lookups yield `None` rather than failing the conversion.
pub fn collect_plugin_connection_points(
    parsed: &[ParsedFile],
    file_id_map: &HashMap<String, i64>,
    symbol_id_map: &HashMap<(String, String), i64>,
) -> Vec<DbPoint> {
    let mut out: Vec<DbPoint> = Vec::new();
    for pf in parsed {
        if pf.connection_points.is_empty() { continue }
        let Some(&file_id) = file_id_map.get(&pf.path) else { continue };
        for cp in &pf.connection_points {
            let protocol = kind_to_protocol(cp.kind);
            let direction = role_to_direction(cp.role);
            let symbol_id = if cp.symbol_qname.is_empty() {
                None
            } else {
                symbol_id_map
                    .get(&(pf.path.clone(), cp.symbol_qname.clone()))
                    .copied()
            };
            // HTTP method lives in `meta.method` by convention for Rest /
            // GraphQL starts; empty for other kinds.
            let method = cp.meta.get("method").cloned().unwrap_or_default();
            // Framework tag lives in `meta.framework` (`"gin"`, `"spring"`,
            // `"fastapi"`, etc.). Empty when not applicable.
            let framework = cp.meta.get("framework").cloned().unwrap_or_default();
            // Remaining meta keys serialize as a JSON object for the
            // `metadata` column.
            let leftover: HashMap<&String, &String> = cp
                .meta
                .iter()
                .filter(|(k, _)| k.as_str() != "method" && k.as_str() != "framework")
                .collect();
            let metadata = if leftover.is_empty() {
                None
            } else {
                serde_json::to_string(&leftover).ok()
            };

            out.push(DbPoint {
                file_id,
                symbol_id,
                line: cp.line,
                protocol,
                direction,
                key: cp.key.clone(),
                method,
                framework,
                metadata,
            });
        }
    }
    out
}

/// Map the plugin-facing `ConnectionKind` to the DB-side `Protocol`. Close
/// correspondence; only `ConnectionKind::Route` diverges (becomes `Rest` on
/// the matcher side since routes fold into HTTP REST flow edges).
fn kind_to_protocol(kind: ConnectionKind) -> Protocol {
    match kind {
        ConnectionKind::Rest | ConnectionKind::Route => Protocol::Rest,
        ConnectionKind::Grpc => Protocol::Grpc,
        ConnectionKind::GraphQL => Protocol::GraphQl,
        ConnectionKind::Di => Protocol::Di,
        ConnectionKind::Ipc => Protocol::Ipc,
        ConnectionKind::Event => Protocol::EventBus,
        ConnectionKind::MessageQueue => Protocol::MessageQueue,
    }
}

fn role_to_direction(role: ConnectionRole) -> FlowDirection {
    match role {
        ConnectionRole::Start => FlowDirection::Start,
        ConnectionRole::Stop => FlowDirection::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ConnectionPoint as AbstractPoint, FlowMeta};
    use std::collections::HashMap;

    fn parsed_file_with_points(path: &str, points: Vec<AbstractPoint>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "test".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: Vec::new(),
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: None,
            has_errors: false,
            flow: FlowMeta::default(),
            connection_points: points,
            demand_contributions: Vec::new(),
        }
    }

    #[test]
    fn rest_start_with_method_and_framework_round_trips() {
        let mut meta = HashMap::new();
        meta.insert("method".to_string(), "GET".to_string());
        meta.insert("framework".to_string(), "gin".to_string());

        let ap = AbstractPoint {
            kind: ConnectionKind::Rest,
            role: ConnectionRole::Start,
            key: "/users/:id".to_string(),
            line: 42,
            col: 1,
            symbol_qname: "app.handlers.GetUser".to_string(),
            meta,
        };
        let pf = parsed_file_with_points("src/users.go", vec![ap]);

        let mut files = HashMap::new();
        files.insert("src/users.go".to_string(), 7i64);
        let mut syms = HashMap::new();
        syms.insert(("src/users.go".to_string(), "app.handlers.GetUser".to_string()), 42i64);

        let db_points =
            collect_plugin_connection_points(std::slice::from_ref(&pf), &files, &syms);
        assert_eq!(db_points.len(), 1);
        let p = &db_points[0];
        assert_eq!(p.file_id, 7);
        assert_eq!(p.symbol_id, Some(42));
        assert_eq!(p.line, 42);
        assert_eq!(p.protocol, Protocol::Rest);
        assert_eq!(p.direction, FlowDirection::Start);
        assert_eq!(p.key, "/users/:id");
        assert_eq!(p.method, "GET");
        assert_eq!(p.framework, "gin");
        assert!(p.metadata.is_none(), "no leftover meta");
    }

    #[test]
    fn point_without_file_id_is_dropped() {
        let ap = AbstractPoint {
            kind: ConnectionKind::Di,
            role: ConnectionRole::Start,
            key: "IUserService".to_string(),
            line: 1,
            col: 1,
            symbol_qname: String::new(),
            meta: HashMap::new(),
        };
        let pf = parsed_file_with_points("unknown/file.cs", vec![ap]);
        let empty_files = HashMap::new();
        let empty_syms = HashMap::new();
        let db_points =
            collect_plugin_connection_points(std::slice::from_ref(&pf), &empty_files, &empty_syms);
        assert!(db_points.is_empty());
    }

    #[test]
    fn route_kind_folds_into_rest_protocol() {
        let ap = AbstractPoint {
            kind: ConnectionKind::Route,
            role: ConnectionRole::Start,
            key: "/api/ping".to_string(),
            line: 10,
            col: 1,
            symbol_qname: String::new(),
            meta: HashMap::new(),
        };
        let pf = parsed_file_with_points("routes.rb", vec![ap]);
        let mut files = HashMap::new();
        files.insert("routes.rb".to_string(), 1i64);
        let db_points =
            collect_plugin_connection_points(std::slice::from_ref(&pf), &files, &HashMap::new());
        assert_eq!(db_points.len(), 1);
        assert_eq!(db_points[0].protocol, Protocol::Rest);
    }

    #[test]
    fn leftover_meta_serializes_into_metadata_json() {
        let mut meta = HashMap::new();
        meta.insert("service".to_string(), "UserService".to_string());
        meta.insert("method".to_string(), "POST".to_string());

        let ap = AbstractPoint {
            kind: ConnectionKind::Grpc,
            role: ConnectionRole::Start,
            key: "users.UserService/Create".to_string(),
            line: 5,
            col: 1,
            symbol_qname: String::new(),
            meta,
        };
        let pf = parsed_file_with_points("grpc/users.proto", vec![ap]);
        let mut files = HashMap::new();
        files.insert("grpc/users.proto".to_string(), 3i64);
        let db_points =
            collect_plugin_connection_points(std::slice::from_ref(&pf), &files, &HashMap::new());
        assert_eq!(db_points.len(), 1);
        let p = &db_points[0];
        assert_eq!(p.method, "POST");
        let md = p.metadata.as_ref().expect("leftover meta should serialize");
        assert!(md.contains("\"service\""));
        assert!(md.contains("UserService"));
        assert!(!md.contains("method"), "method shouldn't appear in leftover json");
    }
}
