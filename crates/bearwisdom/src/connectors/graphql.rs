// =============================================================================
// connectors/graphql.rs  —  GraphQL schema + resolver connector
//
// Three detection passes:
//
//   1. Schema files (.graphql / .gql) — parse `type Query { ... }`,
//      `type Mutation { ... }`, `type Subscription { ... }` blocks and extract
//      field names as operation names.
//
//   2. SDL embedded in code — search TS/JS/Python files for `gql`...` template
//      literals or plain strings that contain `type Query` or `type Mutation`.
//
//   3. Resolver matching — for each detected operation name search the symbols
//      table for functions / methods with a matching name.  Create flow_edges
//      with edge_type = 'graphql_resolver'.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A GraphQL operation (query / mutation / subscription field) discovered in
/// a schema file or an embedded SDL string.
#[derive(Debug, Clone)]
pub struct GraphQLOperation {
    /// `files.id` of the file that contains the schema definition.
    pub file_id: i64,
    /// The operation (field) name, e.g. `"getUser"`.
    pub name: String,
    /// One of `"query"`, `"mutation"`, or `"subscription"`.
    pub operation_type: String,
    /// 1-based line number where the field was found.
    pub line: u32,
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

fn build_type_block_regex() -> Regex {
    // Matches the opening of a top-level GraphQL type block:
    //   type Query {
    //   type Mutation  {
    //   type Subscription{
    Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{")
        .expect("graphql type block regex is valid")
}

fn build_field_regex() -> Regex {
    // Field definition inside a type block:
    //   fieldName: ReturnType
    //   fieldName(arg: Type): ReturnType
    // We stop at the first colon (or opening paren).
    Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:").expect("graphql field regex is valid")
}

fn build_sdl_search_regex() -> Regex {
    // Detects files that embed SDL — looks for `type Query` or `type Mutation`
    // inside a gql`` template literal or similar string context.
    Regex::new(r"type\s+(?:Query|Mutation|Subscription)\s*\{")
        .expect("graphql sdl search regex is valid")
}

// ---------------------------------------------------------------------------
// Schema file parsing
// ---------------------------------------------------------------------------

/// Extract GraphQL operations from `.graphql` / `.gql` files in the index.
pub fn detect_graphql_operations(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<GraphQLOperation>> {
    let re_type_block = build_type_block_regex();
    let re_field = build_field_regex();
    let re_sdl_search = build_sdl_search_regex();

    let mut stmt = conn
        .prepare(
            "SELECT id, path, language FROM files
             WHERE language IN ('graphql', 'typescript', 'tsx', 'javascript', 'jsx', 'python')",
        )
        .context("Failed to prepare GraphQL/SDL file query")?;

    let files: Vec<(i64, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("Failed to query files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect file rows")?;

    let mut operations: Vec<GraphQLOperation> = Vec::new();

    for (file_id, rel_path, language) in files {
        // For code files, only proceed if they contain SDL-like content.
        let is_schema_file = language == "graphql";
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable file");
                continue;
            }
        };

        // For non-schema files, do a quick pre-check before the expensive parse.
        if !is_schema_file && !re_sdl_search.is_match(&source) {
            continue;
        }

        extract_operations_from_source(
            &source,
            file_id,
            &re_type_block,
            &re_field,
            &mut operations,
        );
    }

    debug!(count = operations.len(), "GraphQL operations detected");
    Ok(operations)
}

/// Parse a single source string for GraphQL type blocks and their fields.
fn extract_operations_from_source(
    source: &str,
    file_id: i64,
    re_type_block: &Regex,
    re_field: &Regex,
    out: &mut Vec<GraphQLOperation>,
) {
    // State machine: track which operation type block we're currently inside.
    let mut current_op_type: Option<String> = None;
    let mut brace_depth: u32 = 0;

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Check for type block opening.
        if let Some(cap) = re_type_block.captures(line_text) {
            current_op_type = Some(cap[1].to_lowercase());
            brace_depth = 1;
            continue;
        }

        if current_op_type.is_none() {
            continue;
        }

        // Track brace depth so we know when the block ends.
        for ch in line_text.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                    }
                }
                _ => {}
            }
        }

        if brace_depth == 0 {
            current_op_type = None;
            continue;
        }

        // Extract field names at depth == 1 (direct members of the type block).
        if brace_depth == 1 {
            if let Some(cap) = re_field.captures(line_text) {
                let field_name = cap[1].to_string();
                // Skip GraphQL built-in meta fields.
                if field_name.starts_with("__") {
                    continue;
                }
                out.push(GraphQLOperation {
                    file_id,
                    name: field_name,
                    operation_type: current_op_type.clone().unwrap_or_default(),
                    line: line_no,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Resolver matching
// ---------------------------------------------------------------------------

/// For each GraphQL operation, search the symbols table for a resolver
/// function/method with a matching name and create a `flow_edges` row.
///
/// Returns the number of edges created.
pub fn match_operations_to_resolvers(
    conn: &Connection,
    operations: &[GraphQLOperation],
) -> Result<u32> {
    if operations.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for op in operations {
        // Look for any function or method that matches the operation name.
        // Could be in any language (JS resolver, Python resolver, etc.).
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.line FROM symbols s
                 WHERE s.name = ?1 AND s.kind IN ('function', 'method')
                 LIMIT 10",
            )
            .context("Failed to prepare resolver symbol query")?;

        let resolvers: Vec<(i64, i64, u32)> = stmt
            .query_map(rusqlite::params![op.name], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, u32>(2)?))
            })
            .context("Failed to query resolver symbols")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect resolver rows")?;

        for (sym_id, resolver_file_id, resolver_line) in resolvers {
            let result = conn.execute(
                "INSERT OR IGNORE INTO flow_edges (
                    source_file_id, source_line, source_symbol, source_language,
                    target_file_id, target_line, target_symbol, target_language,
                    edge_type, protocol, confidence
                 ) VALUES (
                    ?1, ?2, ?3, 'graphql',
                    ?4, ?5, ?6, NULL,
                    'graphql_resolver', 'graphql', 0.80
                 )",
                rusqlite::params![
                    op.file_id,
                    op.line,
                    op.name,
                    resolver_file_id,
                    resolver_line,
                    sym_id,
                ],
            );

            match result {
                Ok(n) if n > 0 => created += 1,
                Ok(_) => {}
                Err(e) => {
                    debug!(err = %e, operation = %op.name, "Failed to insert graphql_resolver edge");
                }
            }
        }
    }

    info!(created, "GraphQL: resolver edges created");
    Ok(created)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run all GraphQL detection passes and write results to the database.
pub fn connect(db: &Database, project_root: &Path) -> Result<()> {
    let conn = &db.conn;

    let operations = detect_graphql_operations(conn, project_root)
        .context("GraphQL operation detection failed")?;
    info!(count = operations.len(), "GraphQL operations detected");

    let edges = match_operations_to_resolvers(conn, &operations)
        .context("GraphQL resolver matching failed")?;
    info!(edges, "GraphQL connector complete");

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_graphql_file(content: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".graphql")
            .tempfile()
            .unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    fn insert_file(conn: &Connection, name: &str, lang: &str) -> i64 {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES (?1, 'h', ?2, 0)",
            rusqlite::params![name, lang],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    // -----------------------------------------------------------------------
    // Regex unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn type_block_regex_matches_query() {
        let re = build_type_block_regex();
        assert!(re.is_match("type Query {"));
        let cap = re.captures("type Mutation {").unwrap();
        assert_eq!(&cap[1], "Mutation");
    }

    #[test]
    fn type_block_regex_matches_subscription() {
        let re = build_type_block_regex();
        assert!(re.is_match("type Subscription {"));
    }

    #[test]
    fn field_regex_extracts_simple_field() {
        let re = build_field_regex();
        let cap = re.captures("  getUser: User").unwrap();
        assert_eq!(&cap[1], "getUser");
    }

    #[test]
    fn field_regex_extracts_field_with_args() {
        let re = build_field_regex();
        let cap = re.captures("  createUser(input: CreateUserInput!): User!").unwrap();
        assert_eq!(&cap[1], "createUser");
    }

    // -----------------------------------------------------------------------
    // Source extraction tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_operations_from_schema() {
        let re_type = build_type_block_regex();
        let re_field = build_field_regex();

        let schema = r#"
type Query {
  getUser(id: ID!): User
  listOrders: [Order!]!
}

type Mutation {
  createUser(input: CreateUserInput!): User!
  deleteUser(id: ID!): Boolean!
}
"#;

        let mut ops: Vec<GraphQLOperation> = Vec::new();
        extract_operations_from_source(schema, 1, &re_type, &re_field, &mut ops);

        let queries: Vec<_> = ops.iter().filter(|o| o.operation_type == "query").collect();
        let mutations: Vec<_> = ops.iter().filter(|o| o.operation_type == "mutation").collect();

        assert_eq!(queries.len(), 2, "Expected 2 query fields");
        assert_eq!(mutations.len(), 2, "Expected 2 mutation fields");
        assert!(queries.iter().any(|o| o.name == "getUser"));
        assert!(mutations.iter().any(|o| o.name == "createUser"));
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_operations_from_graphql_file() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let gql_file = make_graphql_file(
            "type Query {\n  getProduct(id: ID!): Product\n  listProducts: [Product!]!\n}\n",
        );
        let root = gql_file.path().parent().unwrap();
        let file_name = gql_file.path().file_name().unwrap().to_str().unwrap();

        insert_file(conn, file_name, "graphql");

        let ops = detect_graphql_operations(conn, root).unwrap();
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().any(|o| o.name == "getProduct"));
        assert!(ops.iter().any(|o| o.name == "listProducts"));
    }

    #[test]
    fn match_operations_creates_resolver_edge() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        // Schema file.
        insert_file(conn, "schema.graphql", "graphql");
        let schema_file_id: i64 = conn.last_insert_rowid();

        // Resolver file with matching symbol.
        let resolver_file_id = insert_file(conn, "resolvers.ts", "typescript");
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'getProduct', 'Resolvers.getProduct', 'function', 10, 0)",
            [resolver_file_id],
        )
        .unwrap();

        let ops = vec![GraphQLOperation {
            file_id: schema_file_id,
            name: "getProduct".to_string(),
            operation_type: "query".to_string(),
            line: 2,
        }];

        let created = match_operations_to_resolvers(conn, &ops).unwrap();
        assert_eq!(created, 1, "Expected one resolver edge");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'graphql_resolver'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn no_operations_skips_resolver_matching() {
        let db = Database::open_in_memory().unwrap();
        let created = match_operations_to_resolvers(&db.conn, &[]).unwrap();
        assert_eq!(created, 0);
    }

    #[test]
    fn sdl_in_typescript_file_detected() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let mut f = tempfile::Builder::new()
            .suffix(".ts")
            .tempfile()
            .unwrap();
        write!(
            f,
            "const typeDefs = gql`\ntype Query {{\n  hello: String\n}}\n`;\n"
        )
        .unwrap();

        let root = f.path().parent().unwrap();
        let file_name = f.path().file_name().unwrap().to_str().unwrap();
        insert_file(conn, file_name, "typescript");

        let ops = detect_graphql_operations(conn, root).unwrap();
        // The embedded `type Query { hello: String }` block should yield one field.
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].name, "hello");
    }
}
