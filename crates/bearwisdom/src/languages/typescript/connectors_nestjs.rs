// =============================================================================
// languages/typescript/connectors_nestjs.rs — NestJS @Controller/@Get extraction
//
// `discover_nestjs_routes` in the connectors facade calls `extract_nestjs_routes`
// to walk indexed TypeScript files, recognise `@Controller(prefix)` plus method
// decorators (`@Get`/`@Post`/`@Put`/`@Delete`/`@Patch`), and emit `NestRoute`
// rows that the facade inserts into the `routes` table.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use super::connectors::OptionalExt;

// ===========================================================================
// NestJS extraction helpers
// ===========================================================================

/// A NestJS HTTP route extracted from decorator annotations.
#[derive(Debug, Clone)]
pub struct NestRoute {
    /// `files.id` of the controller file.
    pub file_id: i64,
    /// `symbols.id` of the handler method, if indexed.
    pub symbol_id: Option<i64>,
    /// HTTP method, uppercased: "GET", "POST", "PUT", "DELETE", "PATCH".
    pub http_method: String,
    /// The combined route template (class prefix + method route).
    pub route_template: String,
    /// The TypeScript method name.
    pub handler_name: String,
    /// 1-based line number of the method decorator.
    pub line: u32,
}

pub(super) struct NestRegexes {
    /// @Controller('prefix') or @Controller("prefix") or @Controller()
    controller: Regex,
    /// @Get('route') / @Post('route') / @Put('route') / @Delete('route') / @Patch('route')
    method_decorator: Regex,
    /// TypeScript/JavaScript method declaration: optional async, name(
    method_name: Regex,
}

impl NestRegexes {
    pub(super) fn build() -> Self {
        Self {
            controller: Regex::new(r#"@Controller\s*\(\s*(?:['"]([^'"]*)['"]\s*)?\)"#)
                .expect("controller regex is valid"),
            method_decorator: Regex::new(
                r#"@(Get|Post|Put|Delete|Patch)\s*\(\s*(?:['"]([^'"]*)['"]\s*)?\)"#,
            )
            .expect("method decorator regex is valid"),
            method_name: Regex::new(r"(?:async\s+)?(\w+)\s*\(")
                .expect("method name regex is valid"),
        }
    }
}

pub(super) fn extract_nestjs_routes(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<NestRoute>> {
    let re = NestRegexes::build();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'typescript'")
        .context("Failed to prepare TypeScript files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to query TypeScript files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TypeScript file rows")?;

    let mut routes: Vec<NestRoute> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TypeScript file");
                continue;
            }
        };
        extract_routes_from_source(conn, &source, file_id, &rel_path, &re, &mut routes);
    }

    Ok(routes)
}

/// Scan a single source file for NestJS route decorators and append found routes to `out`.
pub(super) fn extract_routes_from_source(
    conn: &Connection,
    source: &str,
    file_id: i64,
    rel_path: &str,
    re: &NestRegexes,
    out: &mut Vec<NestRoute>,
) {
    let lines: Vec<&str> = source.lines().collect();

    // First pass: find the class-level @Controller prefix.
    let class_prefix = find_controller_prefix(&lines, re);

    // Second pass: collect method-level decorators.
    let mut pending: Option<(u32, String, String)> = None; // (ann_line, method, route)

    for (idx, line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        if let Some(cap) = re.method_decorator.captures(line) {
            let http_method = cap[1].to_uppercase();
            let suffix = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let route_template = nest_join_paths(&class_prefix, suffix);
            pending = Some((line_no, http_method, route_template));
            continue;
        }

        if let Some((ann_line, http_method, route_template)) = pending.take() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('@') {
                pending = Some((ann_line, http_method, route_template));
                continue;
            }

            if let Some(fn_cap) = re.method_name.captures(line) {
                let handler_name = fn_cap[1].to_string();

                if is_ts_keyword(&handler_name) {
                    pending = Some((ann_line, http_method, route_template));
                    continue;
                }

                let symbol_id = lookup_symbol(conn, &handler_name, rel_path);

                out.push(NestRoute {
                    file_id,
                    symbol_id,
                    http_method,
                    route_template,
                    handler_name,
                    line: ann_line,
                });
            }
        }
    }
}

fn find_controller_prefix(lines: &[&str], re: &NestRegexes) -> String {
    for line in lines {
        if let Some(cap) = re.controller.captures(line) {
            return cap
                .get(1)
                .map(|m| normalise_prefix(m.as_str()))
                .unwrap_or_default();
        }
    }
    String::new()
}

fn nest_join_paths(prefix: &str, suffix: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let s = suffix.trim_start_matches('/');
    if p.is_empty() && s.is_empty() {
        "/".to_string()
    } else if p.is_empty() {
        format!("/{s}")
    } else if s.is_empty() {
        p.to_string()
    } else {
        format!("{p}/{s}")
    }
}

fn normalise_prefix(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn is_ts_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "while" | "for" | "switch" | "catch" | "function" | "return" | "new"
    )
}

fn lookup_symbol(conn: &Connection, name: &str, rel_path: &str) -> Option<i64> {
    conn.query_row(
        "SELECT s.id FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 AND f.path = ?2
           AND s.kind IN ('method', 'function')
         LIMIT 1",
        rusqlite::params![name, rel_path],
        |r| r.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn controller_regex_with_single_quoted_prefix() {
        let re = NestRegexes::build();
        let line = "@Controller('users')";
        let cap = re.controller.captures(line).unwrap();
        assert_eq!(cap.get(1).unwrap().as_str(), "users");
    }

    #[test]
    fn controller_regex_with_double_quoted_prefix() {
        let re = NestRegexes::build();
        let line = r#"@Controller("articles")"#;
        let cap = re.controller.captures(line).unwrap();
        assert_eq!(cap.get(1).unwrap().as_str(), "articles");
    }

    #[test]
    fn controller_regex_with_no_argument() {
        let re = NestRegexes::build();
        let line = "@Controller()";
        let cap = re.controller.captures(line).unwrap();
        assert!(
            cap.get(1).is_none(),
            "no-arg controller should have no prefix group"
        );
    }

    #[test]
    fn method_decorator_regex_get_no_arg() {
        let re = NestRegexes::build();
        let line = "  @Get()";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert!(cap.get(2).is_none());
    }

    #[test]
    fn method_decorator_regex_get_with_param() {
        let re = NestRegexes::build();
        let line = "  @Get(':id')";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert_eq!(cap.get(2).unwrap().as_str(), ":id");
    }

    #[test]
    fn method_decorator_regex_post_double_quoted() {
        let re = NestRegexes::build();
        let line = r#"  @Post("register")"#;
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Post");
        assert_eq!(cap.get(2).unwrap().as_str(), "register");
    }

    #[test]
    fn method_decorator_regex_delete_with_param() {
        let re = NestRegexes::build();
        let line = "  @Delete(':id')";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Delete");
        assert_eq!(cap.get(2).unwrap().as_str(), ":id");
    }

    #[test]
    fn join_paths_prefix_and_suffix() {
        assert_eq!(nest_join_paths("/users", ":id"), "/users/:id");
        assert_eq!(nest_join_paths("/users/", "/:id"), "/users/:id");
    }

    #[test]
    fn join_paths_no_suffix() {
        assert_eq!(nest_join_paths("/users", ""), "/users");
    }

    #[test]
    fn join_paths_no_prefix() {
        assert_eq!(nest_join_paths("", "register"), "/register");
    }

    #[test]
    fn join_paths_both_empty() {
        assert_eq!(nest_join_paths("", ""), "/");
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

    #[test]
    fn extracts_basic_controller_and_get_method() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

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
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            1,
            "src/users/users.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].route_template, "/users");
        assert_eq!(routes[0].handler_name, "findAll");
    }

    #[test]
    fn extracts_parameterised_route() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

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
        let re = NestRegexes::build();
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

    #[test]
    fn extracts_controller_with_no_prefix() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

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
        let re = NestRegexes::build();
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

    #[test]
    fn extracts_all_http_verbs() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

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
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            1,
            "src/items/items.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 6);
        let methods: Vec<&str> = routes.iter().map(|r| r.http_method.as_str()).collect();
        assert!(methods.contains(&"GET"));
        assert!(methods.contains(&"POST"));
        assert!(methods.contains(&"PUT"));
        assert!(methods.contains(&"PATCH"));
        assert!(methods.contains(&"DELETE"));
    }

    #[test]
    fn symbol_id_is_resolved_when_indexed() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

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
        let re = NestRegexes::build();
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
}
