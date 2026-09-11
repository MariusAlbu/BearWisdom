//! Relative imports across directories bind every position through the
//! importing file's own module rules: type annotations, decorators, calls and
//! constructions, whether the specifier climbs (`../user/x`), stays in the
//! directory (`./tag`), or names a directory entry (`../shared`).

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"relative-imports","version":"0.0.1"}"#,
    );
    project.add_file(
        "src/user/user.entity.ts",
        "export class UserEntity {\n  id: number;\n}\nexport interface UserData {\n  name: string;\n}\n",
    );
    project.add_file(
        "src/shared/index.ts",
        "export function slugify(input: string): string {\n  return input;\n}\n",
    );
    project.add_file(
        "src/article/tag.ts",
        "export class Tag {\n  name: string;\n}\n",
    );
    project.add_file(
        "src/article/article.entity.ts",
        "import { UserEntity, UserData } from '../user/user.entity';\nimport { slugify } from '../shared';\nimport { Tag } from './tag';\n\nexport class ArticleEntity {\n  author: UserEntity;\n  data: UserData;\n  tag: Tag;\n  make(): UserEntity {\n    slugify('x');\n    return new UserEntity();\n  }\n}\n",
    );
    project
}

fn edges(db: &bearwisdom::Database) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, e.kind, t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE f.path != (SELECT f2.path FROM symbols s2 JOIN files f2 ON f2.id = s2.file_id WHERE s2.id = e.source_id)
             ORDER BY s.name, e.kind, t.name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(source: &str, kind: &str, target: &str, file: &str) -> (String, String, String, String) {
    (source.into(), kind.into(), target.into(), file.into())
}

#[test]
fn relative_imports_bind_type_positions_and_calls_across_directories() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let user = "src/user/user.entity.ts";
    let cross_file = edges(&db);
    for expected in [
        edge("ArticleEntity", "type_ref", "Tag", "src/article/tag.ts"),
        edge("ArticleEntity", "type_ref", "UserData", user),
        edge("ArticleEntity", "type_ref", "UserEntity", user),
        edge("make", "calls", "slugify", "src/shared/index.ts"),
        edge("make", "instantiates", "UserEntity", user),
        edge("make", "type_ref", "UserEntity", user),
    ] {
        assert!(
            cross_file.contains(&expected),
            "missing {expected:?} in {cross_file:?}"
        );
    }

    let unlinked: Vec<(String, String, Option<String>)> = {
        let mut stmt = db
            .prepare(
                "SELECT target_name, kind, module FROM unresolved_refs
                 WHERE cause_kind = 'unbound_import_unlinked' ORDER BY target_name",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(unlinked.is_empty(), "every relative import links: {unlinked:?}");
}
