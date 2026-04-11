//! Integration test for S5 TypeScript externals MVP.
//!
//! Mirrors `python_externals.rs`: seeds a fake `node_modules` with a small
//! package, points `BEARWISDOM_TS_NODE_MODULES` at it, indexes a consumer
//! project whose `package.json` depends on that package, and asserts the
//! full externals pipeline end-to-end.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// Build a synthetic `node_modules` with one package: `fake-ui` exposing
/// `Button`, `ButtonProps`, and `useFake`.
fn seed_fake_node_modules() -> TempDir {
    let nm = TempDir::new().unwrap();
    let pkg = nm.path().join("fake-ui");
    fs::create_dir_all(&pkg).unwrap();

    fs::write(
        pkg.join("package.json"),
        r#"{"name":"fake-ui","version":"1.0.0","main":"index.js","types":"index.d.ts"}"#,
    )
    .unwrap();

    fs::write(
        pkg.join("index.d.ts"),
        r#"export interface ButtonProps {
    label: string;
    onClick(): void;
}

export declare class Button {
    props: ButtonProps;
    constructor(props: ButtonProps);
    render(): string;
}

export declare function useFake<T>(initial: T): [T, (next: T) => void];
"#,
    )
    .unwrap();

    nm
}

/// Build a tiny TS project that depends on `fake-ui`.
fn seed_consumer_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "package.json",
        r#"{
  "name": "consumer",
  "version": "0.0.1",
  "dependencies": {
    "fake-ui": "^1.0.0"
  }
}
"#,
    );

    project.add_file(
        "src/app.ts",
        r#"import { Button, ButtonProps, useFake } from "fake-ui";

export function bootstrap(label: string): Button {
    const props: ButtonProps = { label, onClick: () => {} };
    return new Button(props);
}

export function counter() {
    const [value, setValue] = useFake<number>(0);
    setValue(value + 1);
    return value;
}
"#,
    );

    project
}

#[test]
fn external_ts_package_is_indexed_and_resolved() {
    let node_modules = seed_fake_node_modules();
    let project = seed_consumer_project();

    let prior = std::env::var_os("BEARWISDOM_TS_NODE_MODULES");
    unsafe {
        std::env::set_var("BEARWISDOM_TS_NODE_MODULES", node_modules.path());
    }

    let mut db = TestProject::in_memory_db();
    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    unsafe {
        match prior {
            Some(v) => std::env::set_var("BEARWISDOM_TS_NODE_MODULES", v),
            None => std::env::remove_var("BEARWISDOM_TS_NODE_MODULES"),
        }
    }

    // Internal stats reflect only the consumer project.
    assert!(
        stats.file_count >= 1,
        "expected at least 1 internal file, got {}",
        stats.file_count
    );

    // External files landed.
    let external_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_files >= 1,
        "expected at least 1 external file, got {external_files}"
    );

    // External symbols indexed — Button class, ButtonProps interface,
    // useFake function at a minimum.
    let external_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_symbols >= 3,
        "expected Button + ButtonProps + useFake (at least 3), got {external_symbols}"
    );

    // User queries skip externals.
    let search_hits =
        bearwisdom::query::search::search_symbols(&db, "Button", 10, &Default::default())
            .unwrap();
    assert!(
        search_hits.iter().all(|s| !s.qualified_name.contains("fake-ui")),
        "search_symbols leaked an external symbol: {:?}",
        search_hits.iter().map(|s| &s.qualified_name).collect::<Vec<_>>()
    );

    // Tier 1 TS resolver closes the loop: at least one internal→external
    // edge from app.ts importing fake-ui. S5 relies on the package-prefix
    // rewrite (`fake-ui.Button`) plus the bare-import qname lookup step
    // added to the TS resolver.
    let edges_to_external: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             WHERE s.origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        edges_to_external >= 1,
        "expected at least one internal→external edge (app.ts → fake-ui.Button), got {edges_to_external}"
    );
}
