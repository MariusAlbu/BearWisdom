    use super::*;
    use crate::db::Database;

    // -----------------------------------------------------------------------
    // Unit tests for parsing helpers
    // -----------------------------------------------------------------------

    #[test]
    fn controller_regex_with_single_quoted_prefix() {
        let re = Regexes::build();
        let line = "@Controller('users')";
        let cap = re.controller.captures(line).unwrap();
        assert_eq!(cap.get(1).unwrap().as_str(), "users");
    }

    #[test]
    fn controller_regex_with_double_quoted_prefix() {
        let re = Regexes::build();
        let line = r#"@Controller("articles")"#;
        let cap = re.controller.captures(line).unwrap();
        assert_eq!(cap.get(1).unwrap().as_str(), "articles");
    }

    #[test]
    fn controller_regex_with_no_argument() {
        let re = Regexes::build();
        let line = "@Controller()";
        let cap = re.controller.captures(line).unwrap();
        assert!(cap.get(1).is_none(), "no-arg controller should have no prefix group");
    }

    #[test]
    fn method_decorator_regex_get_no_arg() {
        let re = Regexes::build();
        let line = "  @Get()";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert!(cap.get(2).is_none());
    }

    #[test]
    fn method_decorator_regex_get_with_param() {
        let re = Regexes::build();
        let line = "  @Get(':id')";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert_eq!(cap.get(2).unwrap().as_str(), ":id");
    }

    #[test]
    fn method_decorator_regex_post_double_quoted() {
        let re = Regexes::build();
        let line = r#"  @Post("register")"#;
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Post");
        assert_eq!(cap.get(2).unwrap().as_str(), "register");
    }

    #[test]
    fn method_decorator_regex_delete_with_param() {
        let re = Regexes::build();
        let line = "  @Delete(':id')";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Delete");
        assert_eq!(cap.get(2).unwrap().as_str(), ":id");
    }

    #[test]
    fn join_paths_prefix_and_suffix() {
        assert_eq!(join_paths("/users", ":id"), "/users/:id");
        assert_eq!(join_paths("/users/", "/:id"), "/users/:id");
    }

    #[test]
    fn join_paths_no_suffix() {
        assert_eq!(join_paths("/users", ""), "/users");
    }

    #[test]
    fn join_paths_no_prefix() {
        assert_eq!(join_paths("", "register"), "/register");
    }

    #[test]
    fn join_paths_both_empty() {
        assert_eq!(join_paths("", ""), "/");
    }

    #[test]
    fn normalise_prefix_adds_leading_slash() {
        assert_eq!(normalise_prefix("users"), "/users");
    }

    #[test]
    fn normalise_prefix_strips_trailing_slash() {
        assert_eq!(normalise_prefix("/users/"), "/users");
    }

    #[test]
    fn normalise_prefix_empty_stays_empty() {
        assert_eq!(normalise_prefix(""), "");
    }

    // -----------------------------------------------------------------------
    // Source extraction tests
    // -----------------------------------------------------------------------

    /// Basic controller + method: @Controller('users') + @Get() → GET /users
    #[test]
    fn extracts_basic_controller_and_get_method() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/users/users.controller.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
import { Controller, Get } from '@nestjs/common';

@Controller('users')
export class UsersController {
  @Get()
  findAll() {
    return this.usersService.findAll();
  }
}
"#;
        let re = Regexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(conn, source, 1, "src/users/users.controller.ts", &re, &mut routes);

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].route_template, "/users");
        assert_eq!(routes[0].handler_name, "findAll");
    }

    /// Parameterised routes: @Controller('articles') + @Get(':slug') → GET /articles/:slug
    #[test]
    fn extracts_parameterised_route() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/articles/articles.controller.ts', 'h2', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
import { Controller, Get, Post, Delete } from '@nestjs/common';

@Controller('articles')
export class ArticlesController {
  @Get(':slug')
  findOne(@Param('slug') slug: string) {
    return this.articlesService.findBySlug(slug);
  }

  @Post()
  create(@Body() dto: CreateArticleDto) {
    return this.articlesService.create(dto);
  }

  @Delete(':id')
  remove(@Param('id') id: string) {
    return this.articlesService.remove(id);
  }
}
"#;
        let re = Regexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            1,
            "src/articles/articles.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 3);

        let get = routes.iter().find(|r| r.http_method == "GET").unwrap();
        assert_eq!(get.route_template, "/articles/:slug");
        assert_eq!(get.handler_name, "findOne");

        let post = routes.iter().find(|r| r.http_method == "POST").unwrap();
        assert_eq!(post.route_template, "/articles");
        assert_eq!(post.handler_name, "create");

        let delete = routes.iter().find(|r| r.http_method == "DELETE").unwrap();
        assert_eq!(delete.route_template, "/articles/:id");
        assert_eq!(delete.handler_name, "remove");
    }

    /// @Controller() with no argument → root prefix, routes are just their own path.
    #[test]
    fn extracts_controller_with_no_prefix() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/health/health.controller.ts', 'h3', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
import { Controller, Get } from '@nestjs/common';

@Controller()
export class HealthController {
  @Get('healthz')
  check() {
    return { status: 'ok' };
  }
}
"#;
        let re = Regexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            1,
            "src/health/health.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].route_template, "/healthz");
        assert_eq!(routes[0].handler_name, "check");
    }

    /// All five HTTP verbs on a single controller.
    #[test]
    fn extracts_all_http_verbs() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/items/items.controller.ts', 'h4', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
@Controller('items')
export class ItemsController {
  @Get()
  findAll() {}

  @Get(':id')
  findOne() {}

  @Post()
  create() {}

  @Put(':id')
  update() {}

  @Patch(':id')
  patch() {}

  @Delete(':id')
  remove() {}
}
"#;
        let re = Regexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(conn, source, 1, "src/items/items.controller.ts", &re, &mut routes);

        assert_eq!(routes.len(), 6);
        let methods: Vec<&str> = routes.iter().map(|r| r.http_method.as_str()).collect();
        assert!(methods.contains(&"GET"));
        assert!(methods.contains(&"POST"));
        assert!(methods.contains(&"PUT"));
        assert!(methods.contains(&"PATCH"));
        assert!(methods.contains(&"DELETE"));
    }

    /// Routes written to the routes table via connect() end-to-end.
    #[test]
    fn write_routes_inserts_to_routes_table() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/users/users.controller.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        let routes = vec![NestRoute {
            file_id,
            symbol_id: None,
            http_method: "GET".to_string(),
            route_template: "/users".to_string(),
            handler_name: "findAll".to_string(),
            line: 6,
        }];

        write_routes(conn, &routes).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (method, template): (String, String) = conn
            .query_row(
                "SELECT http_method, route_template FROM routes",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(method, "GET");
        assert_eq!(template, "/users");
    }

    /// Symbol ID is resolved when the method exists in the symbols table.
    #[test]
    fn symbol_id_is_resolved_when_indexed() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/users/users.controller.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'findAll', 'UsersController.findAll', 'method', 7, 2)",
            [file_id],
        )
        .unwrap();
        let sym_id: i64 = conn.last_insert_rowid();

        let source = r#"
@Controller('users')
export class UsersController {
  @Get()
  findAll() {
    return [];
  }
}
"#;
        let re = Regexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            file_id,
            "src/users/users.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].symbol_id, Some(sym_id));
    }
