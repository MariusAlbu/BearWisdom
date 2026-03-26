    use super::*;
    use crate::db::Database;

    // -----------------------------------------------------------------------
    // Unit tests for parsing helpers
    // -----------------------------------------------------------------------

    #[test]
    fn method_mapping_regex_get_mapping() {
        let re = build_method_mapping_regex();
        let line = r#"    @GetMapping("/items")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert_eq!(&cap[2], "/items");
    }

    #[test]
    fn method_mapping_regex_post_mapping_value_form() {
        let re = build_method_mapping_regex();
        let line = r#"@PostMapping(value = "/orders")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "Post");
        assert_eq!(&cap[2], "/orders");
    }

    #[test]
    fn method_mapping_regex_delete_mapping() {
        let re = build_method_mapping_regex();
        let line = r#"@DeleteMapping("/items/{id}")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "Delete");
        assert_eq!(&cap[2], "/items/{id}");
    }

    #[test]
    fn request_mapping_regex_basic() {
        let re = build_request_mapping_regex();
        let line = r#"@RequestMapping("/api/catalog")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "/api/catalog");
        assert!(cap.get(2).is_none());
    }

    #[test]
    fn request_mapping_regex_with_method() {
        let re = build_request_mapping_regex();
        let line = r#"@RequestMapping(value = "/orders", method = RequestMethod.POST)"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "/orders");
        assert_eq!(&cap[2], "POST");
    }

    #[test]
    fn stereotype_regex_matches_controller() {
        let re = build_stereotype_regex();
        assert!(re.is_match("@RestController"));
        let cap = re.captures("@RestController").unwrap();
        assert_eq!(&cap[1], "RestController");
    }

    #[test]
    fn stereotype_regex_matches_service() {
        let re = build_stereotype_regex();
        let cap = re.captures("@Service").unwrap();
        assert_eq!(&cap[1], "Service");
    }

    #[test]
    fn normalise_stereotype_maps_rest_controller() {
        assert_eq!(normalise_stereotype("RestController"), "controller");
        assert_eq!(normalise_stereotype("Controller"), "controller");
    }

    #[test]
    fn normalise_stereotype_maps_service() {
        assert_eq!(normalise_stereotype("Service"), "service");
    }

    #[test]
    fn normalise_stereotype_maps_repository() {
        assert_eq!(normalise_stereotype("Repository"), "repository");
    }

    #[test]
    fn join_paths_combines_prefix_and_suffix() {
        assert_eq!(join_paths("/api", "/items"), "/api/items");
        assert_eq!(join_paths("/api/", "items"), "/api/items");
        assert_eq!(join_paths("", "/items"), "/items");
    }

    #[test]
    fn normalise_path_prefix_adds_leading_slash() {
        assert_eq!(normalise_path_prefix("api/catalog"), "/api/catalog");
        assert_eq!(normalise_path_prefix("/api/catalog/"), "/api/catalog");
    }

    // -----------------------------------------------------------------------
    // Source extraction tests
    // -----------------------------------------------------------------------

    #[test]
    fn extracts_get_mapping_route() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('CatalogController.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();

        let source = r#"
@RestController
@RequestMapping("/api/catalog")
public class CatalogController {

    @GetMapping("/items")
    public List<Item> getItems() {
        return service.findAll();
    }
}
"#;

        let re_method = build_method_mapping_regex();
        let re_request = build_request_mapping_regex();
        let re_method_name = build_method_name_regex();
        let mut routes = Vec::new();

        extract_routes_from_source(
            conn,
            source,
            1,
            "CatalogController.java",
            &re_method,
            &re_request,
            &re_method_name,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].path, "/api/catalog/items");
        assert_eq!(routes[0].handler_name, "getItems");
    }

    #[test]
    fn extracts_post_mapping_no_class_prefix() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('OrderController.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();

        let source = r#"
@RestController
public class OrderController {

    @PostMapping("/orders")
    public Order createOrder(@RequestBody OrderDto dto) {
        return orderService.create(dto);
    }
}
"#;

        let re_method = build_method_mapping_regex();
        let re_request = build_request_mapping_regex();
        let re_method_name = build_method_name_regex();
        let mut routes = Vec::new();

        extract_routes_from_source(
            conn,
            source,
            1,
            "OrderController.java",
            &re_method,
            &re_request,
            &re_method_name,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "POST");
        assert_eq!(routes[0].path, "/orders");
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    fn seed_spring_db(db: &Database) -> (i64, i64) {
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/main/java/com/example/CatalogController.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'CatalogController', 'com.example.CatalogController', 'class', 5, 0)",
            [file_id],
        )
        .unwrap();
        let class_sym_id: i64 = conn.last_insert_rowid();

        (file_id, class_sym_id)
    }

    #[test]
    fn write_routes_inserts_to_routes_table() {
        let db = Database::open_in_memory().unwrap();
        let (file_id, _) = seed_spring_db(&db);

        let routes = vec![SpringRoute {
            file_id,
            symbol_id: None,
            http_method: "GET".to_string(),
            path: "/api/catalog/items".to_string(),
            handler_name: "getItems".to_string(),
            line: 10,
        }];

        write_routes(&db.conn, &routes).unwrap();

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (method, template): (String, String) = db
            .conn
            .query_row(
                "SELECT http_method, route_template FROM routes",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(method, "GET");
        assert_eq!(template, "/api/catalog/items");
    }

    #[test]
    fn create_stereotype_concepts_creates_controller_concept() {
        let db = Database::open_in_memory().unwrap();
        let (_, class_sym_id) = seed_spring_db(&db);

        let services = vec![SpringService {
            symbol_id: class_sym_id,
            name: "CatalogController".to_string(),
            stereotype: "controller".to_string(),
        }];

        create_stereotype_concepts(&db.conn, &services).unwrap();

        let concept_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE name = 'spring-controllers'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(concept_count, 1);

        let member_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM concept_members", [], |r| r.get(0))
            .unwrap();
        assert_eq!(member_count, 1);
    }

    #[test]
    fn create_stereotype_concepts_groups_by_type() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        // Two files, one controller + one service.
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('Controller.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();
        let f1: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('Service.java', 'h2', 'java', 0)",
            [],
        )
        .unwrap();
        let f2: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'MyController', 'com.MyController', 'class', 1, 0)",
            [f1],
        )
        .unwrap();
        let ctrl_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'MyService', 'com.MyService', 'class', 1, 0)",
            [f2],
        )
        .unwrap();
        let svc_id: i64 = conn.last_insert_rowid();

        let services = vec![
            SpringService { symbol_id: ctrl_id, name: "MyController".to_string(), stereotype: "controller".to_string() },
            SpringService { symbol_id: svc_id, name: "MyService".to_string(), stereotype: "service".to_string() },
        ];

        create_stereotype_concepts(conn, &services).unwrap();

        let concept_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM concepts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(concept_count, 2, "Should have spring-controllers and spring-services");
    }

    #[test]
    fn register_spring_patterns_on_empty_inputs_is_noop() {
        let db = Database::open_in_memory().unwrap();
        // Should not panic or error.
        register_spring_patterns(&db.conn, &[], &[]).unwrap();
    }
