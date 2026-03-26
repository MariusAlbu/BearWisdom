    use super::*;
    use crate::db::Database;

    // -----------------------------------------------------------------------
    // Unit tests for proto parsing helpers
    // -----------------------------------------------------------------------

    #[test]
    fn parse_single_service_and_rpc() {
        let source = r#"
syntax = "proto3";

service Catalog {
    rpc GetItem(GetItemRequest) returns (GetItemResponse);
    rpc ListItems(ListItemsRequest) returns (ListItemsResponse);
}
"#;
        let re_service = Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#).unwrap();
        let re_rpc = Regex::new(
            r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
        )
        .unwrap();

        let services = parse_proto_services(source, 1, &re_service, &re_rpc);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].service_name, "Catalog");
        assert_eq!(services[0].rpcs.len(), 2);
        assert_eq!(services[0].rpcs[0].name, "GetItem");
        assert_eq!(services[0].rpcs[0].input_type, "GetItemRequest");
        assert_eq!(services[0].rpcs[0].output_type, "GetItemResponse");
        assert_eq!(services[0].rpcs[1].name, "ListItems");
    }

    #[test]
    fn parse_multiple_services() {
        let source = r#"
service OrderService {
    rpc CreateOrder(CreateOrderRequest) returns (CreateOrderResponse);
}
service PaymentService {
    rpc ProcessPayment(ProcessPaymentRequest) returns (ProcessPaymentResponse);
}
"#;
        let re_service = Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#).unwrap();
        let re_rpc = Regex::new(
            r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
        )
        .unwrap();

        let services = parse_proto_services(source, 1, &re_service, &re_rpc);
        assert_eq!(services.len(), 2);

        let names: Vec<&str> = services.iter().map(|s| s.service_name.as_str()).collect();
        assert!(names.contains(&"OrderService"));
        assert!(names.contains(&"PaymentService"));
    }

    #[test]
    fn parse_empty_service() {
        let source = "service Empty {}";
        let re_service = Regex::new(r#"(?m)^\s*service\s+(\w+)\s*\{"#).unwrap();
        let re_rpc = Regex::new(
            r#"(?m)^\s*rpc\s+(\w+)\s*\(\s*(\w+)\s*\)\s+returns\s+\(\s*(\w+)\s*\)"#,
        )
        .unwrap();

        let services = parse_proto_services(source, 1, &re_service, &re_rpc);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].rpcs.len(), 0);
    }

    #[test]
    fn line_number_at_first_byte_is_one() {
        assert_eq!(line_number_at("hello\nworld", 0), 1);
    }

    #[test]
    fn line_number_at_after_newline() {
        assert_eq!(line_number_at("hello\nworld", 6), 2);
    }

    // -----------------------------------------------------------------------
    // Integration test against in-memory DB
    // -----------------------------------------------------------------------

    fn seed_db_for_grpc(db: &Database) -> (i64, i64) {
        let conn = &db.conn;

        // Proto file (in-memory, so path won't be readable — that's OK for
        // connect(); we test parse_proto_services separately).
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('protos/catalog.proto', 'hp', 'protobuf', 0)",
            [],
        )
        .unwrap();
        let proto_file_id: i64 = conn.last_insert_rowid();

        // C# file with the generated base class.
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/CatalogService.cs', 'hc', 'csharp', 0)",
            [],
        )
        .unwrap();
        let cs_file_id: i64 = conn.last_insert_rowid();

        // C# class: CatalogBase (generated gRPC stub).
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'CatalogBase', 'CatalogService.CatalogBase', 'class', 5, 0)",
            [cs_file_id],
        )
        .unwrap();

        // C# method: GetItem in CatalogBase.
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'GetItem', 'CatalogService.CatalogBase.GetItem', 'method', 20, 0)",
            [cs_file_id],
        )
        .unwrap();

        (proto_file_id, cs_file_id)
    }

    #[test]
    fn match_service_creates_flow_edge() {
        let db = Database::open_in_memory().unwrap();
        let (proto_file_id, _) = seed_db_for_grpc(&db);

        let service = ProtoService {
            file_id: proto_file_id,
            service_name: "Catalog".to_string(),
            rpcs: vec![ProtoRpc {
                name: "GetItem".to_string(),
                input_type: "GetItemRequest".to_string(),
                output_type: "GetItemResponse".to_string(),
                line: 5,
            }],
        };

        let created = match_service_to_csharp(&db.conn, &service).unwrap();
        assert_eq!(created, 1, "Expected one grpc flow_edge");

        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'grpc_call'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn no_matching_class_creates_no_edge() {
        let db = Database::open_in_memory().unwrap();
        let (proto_file_id, _) = seed_db_for_grpc(&db);

        let service = ProtoService {
            file_id: proto_file_id,
            service_name: "NonExistentService".to_string(),
            rpcs: vec![ProtoRpc {
                name: "SomeRpc".to_string(),
                input_type: "Req".to_string(),
                output_type: "Resp".to_string(),
                line: 1,
            }],
        };

        let created = match_service_to_csharp(&db.conn, &service).unwrap();
        assert_eq!(created, 0);
    }

    #[test]
    fn connect_on_empty_db_is_noop() {
        let db = Database::open_in_memory().unwrap();
        // Should complete without error even with no proto files indexed.
        connect(&db).unwrap();
    }
