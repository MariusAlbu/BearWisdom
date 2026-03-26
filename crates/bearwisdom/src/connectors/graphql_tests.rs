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
