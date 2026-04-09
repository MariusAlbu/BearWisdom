// =============================================================================
// languages/angular/connectors.rs — Angular dependency injection connector
//
// Detects Angular DI patterns (@Injectable + constructor injection) and emits
// Di ConnectionPoints that the ConnectorRegistry matches into flow_edges.
//
// All detection helpers from the legacy `connectors/angular_di.rs` module are
// inlined here. That module is no longer used.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// AngularDiConnector
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
        let re_injectable = build_injectable_regex();
        let re_class = build_class_regex();
        let re_ctor_param = build_constructor_param_regex();

        let files = query_ts_files(conn)?;
        let mut points = Vec::new();

        // Pass 1: find @Injectable classes → stop points (providers)
        let mut injectable_names: HashMap<String, (i64, u32)> = HashMap::new();

        for (file_id, rel_path) in &files {
            let abs_path = project_root.join(rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TypeScript file");
                    continue;
                }
            };

            let lines: Vec<&str> = source.lines().collect();
            let mut i = 0;
            while i < lines.len() {
                if re_injectable.is_match(lines[i]) {
                    for j in (i + 1)..std::cmp::min(i + 6, lines.len()) {
                        if let Some(cap) = re_class.captures(lines[j]) {
                            let class_name = cap[1].to_string();
                            let line_no = (j + 1) as u32;

                            injectable_names.insert(class_name.clone(), (*file_id, line_no));

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
                    if injectable_names.contains_key(&type_name) {
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
// Helpers
// ===========================================================================

fn build_injectable_regex() -> Regex {
    Regex::new(r"@Injectable\s*\(").expect("injectable regex is valid")
}

fn build_class_regex() -> Regex {
    Regex::new(r"\bclass\s+(\w+)").expect("class regex is valid")
}

fn build_constructor_param_regex() -> Regex {
    Regex::new(r"\b(private|public|protected)\s+(?:readonly\s+)?(\w+)\s*:\s*(\w+)")
        .expect("constructor param regex is valid")
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn seed_db(db: &Database) -> (i64, i64) {
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/auth.service.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();
        let service_file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/dashboard.component.ts', 'h2', 'typescript', 0)",
            [],
        )
        .unwrap();
        let component_file_id: i64 = conn.last_insert_rowid();

        (service_file_id, component_file_id)
    }

    #[test]
    fn detects_injectable_class() {
        let re_injectable = build_injectable_regex();
        let re_class = build_class_regex();

        let source = r#"
@Injectable()
export class UserService {
  constructor() {}
}
"#;
        let lines: Vec<&str> = source.lines().collect();
        let mut found = false;
        let mut i = 0;
        while i < lines.len() {
            if re_injectable.is_match(lines[i]) {
                for j in (i + 1)..std::cmp::min(i + 6, lines.len()) {
                    if let Some(cap) = re_class.captures(lines[j]) {
                        assert_eq!(&cap[1], "UserService");
                        found = true;
                        break;
                    }
                }
            }
            i += 1;
        }
        assert!(found, "should have detected UserService as injectable");
    }

    #[test]
    fn detects_constructor_injection() {
        let re_param = build_constructor_param_regex();

        let source = r#"
export class DashboardComponent {
  constructor(private userService: UserService) {}
}
"#;

        let mut injectable_names: HashMap<String, (i64, u32)> = HashMap::new();
        injectable_names.insert("UserService".to_string(), (1, 3));

        let mut found = false;
        for line_text in source.lines() {
            for cap in re_param.captures_iter(line_text) {
                let type_name = cap[3].to_string();
                if injectable_names.contains_key(&type_name) {
                    assert_eq!(type_name, "UserService");
                    found = true;
                }
            }
        }
        assert!(found, "should have detected UserService injection site");
    }

    #[test]
    fn connector_detect_requires_angular_core() {
        let mut ctx = ProjectContext::default();
        let c = AngularDiConnector;

        assert!(!c.detect(&ctx), "should not detect without @angular/core");

        ctx.ts_packages.insert("@angular/core".to_string());
        assert!(c.detect(&ctx), "should detect with @angular/core");
    }

    #[test]
    fn connector_produces_stop_for_injectable_and_start_for_consumer() {
        let db = Database::open_in_memory().unwrap();
        let (service_file_id, component_file_id) = seed_db(&db);

        let conn = db.conn();

        // Write the service source into a temp dir so the connector can read it.
        // Use a temp path approach: write directly to a temp dir.
        let tmp = std::env::temp_dir().join("bw_angular_di_test");
        std::fs::create_dir_all(tmp.join("src")).unwrap();

        std::fs::write(
            tmp.join("src/auth.service.ts"),
            b"@Injectable({ providedIn: 'root' })\nexport class AuthService {}\n",
        )
        .unwrap();

        std::fs::write(
            tmp.join("src/dashboard.component.ts"),
            b"export class DashboardComponent {\n  constructor(private authService: AuthService) {}\n}\n",
        )
        .unwrap();

        // Fix up file paths in DB to match temp dir relative paths.
        // (They were inserted as 'src/auth.service.ts' etc. which is correct.)
        let _ = (service_file_id, component_file_id); // used implicitly via DB

        let connector = AngularDiConnector;
        let mut ctx = ProjectContext::default();
        ctx.ts_packages.insert("@angular/core".to_string());

        assert!(connector.detect(&ctx));

        let points = connector.extract(conn, &tmp).unwrap();

        let stops: Vec<_> = points.iter().filter(|p| p.direction == FlowDirection::Stop).collect();
        let starts: Vec<_> = points.iter().filter(|p| p.direction == FlowDirection::Start).collect();

        assert_eq!(stops.len(), 1, "one @Injectable stop");
        assert_eq!(stops[0].key, "AuthService");
        assert_eq!(starts.len(), 1, "one constructor injection start");
        assert_eq!(starts[0].key, "AuthService");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
