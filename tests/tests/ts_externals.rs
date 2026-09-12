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
    let db = index_with_node_modules(node_modules.path(), &project);

    // Internal stats reflect only the consumer project.
    let internal_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin != 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        internal_files >= 1,
        "expected at least 1 internal file, got {internal_files}"
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
        bearwisdom::query::search::search_symbols(&db, "Button", 10, &Default::default()).unwrap();
    assert!(
        search_hits
            .iter()
            .all(|s| !s.qualified_name.contains("fake-ui")),
        "search_symbols leaked an external symbol: {:?}",
        search_hits
            .iter()
            .map(|s| &s.qualified_name)
            .collect::<Vec<_>>()
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

/// A package whose entry forwards its whole surface through relative barrels:
/// `index.d.ts` → `./dist` → `./decorators` → the declaring leaf, plus a named
/// re-export one hop down. Nothing is declared at the entry.
fn seed_barrel_node_modules() -> TempDir {
    let nm = TempDir::new().unwrap();
    let pkg = nm.path().join("barrel-pkg");
    fs::create_dir_all(pkg.join("dist").join("decorators")).unwrap();
    fs::write(
        pkg.join("package.json"),
        r#"{"name":"barrel-pkg","version":"1.0.0","main":"index.js","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(pkg.join("index.d.ts"), "export * from './dist';\n").unwrap();
    fs::write(
        pkg.join("dist").join("index.d.ts"),
        "export * from './decorators';\nexport { Baz } from './baz';\n",
    )
    .unwrap();
    fs::write(
        pkg.join("dist").join("baz.d.ts"),
        "export declare function Baz(): void;\n",
    )
    .unwrap();
    fs::write(
        pkg.join("dist").join("decorators").join("index.d.ts"),
        "export * from './foo.decorator';\n",
    )
    .unwrap();
    fs::write(
        pkg.join("dist").join("decorators").join("foo.decorator.d.ts"),
        "export declare function Foo(): MethodDecorator;\nexport declare class Bar {\n    go(): void;\n}\n",
    )
    .unwrap();
    nm
}

fn seed_barrel_consumer() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"consumer","version":"0.0.1","dependencies":{"barrel-pkg":"^1.0.0"}}"#,
    );
    project.add_file(
        "src/ctl.ts",
        r#"import { Foo, Bar, Baz } from "barrel-pkg";

export class Ctl {
  @Foo()
  run(): void {
    const b = new Bar();
    b.go();
    Baz();
  }
}
"#,
    );
    project
}

/// The node_modules override is process-global: tests that set it run one
/// at a time so a concurrent test never indexes against the wrong tree.
static NODE_MODULES_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn index_with_node_modules(
    node_modules: &std::path::Path,
    project: &TestProject,
) -> bearwisdom::Database {
    let _serial = NODE_MODULES_ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prior = std::env::var_os("BEARWISDOM_TS_NODE_MODULES");
    unsafe {
        std::env::set_var("BEARWISDOM_TS_NODE_MODULES", node_modules);
    }
    let mut db = TestProject::in_memory_db();
    let indexed = full_index(&mut db, project.path(), None, None, None);
    unsafe {
        match prior {
            Some(v) => std::env::set_var("BEARWISDOM_TS_NODE_MODULES", v),
            None => std::env::remove_var("BEARWISDOM_TS_NODE_MODULES"),
        }
    }
    indexed.unwrap();
    db
}

/// `(source, kind, target, target file)` for every edge of `kind` that lands
/// on an external declaration, in a deterministic order.
fn external_edges(db: &bearwisdom::Database, kind: &str) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, e.kind, t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE e.kind = ?1 AND t.origin = 'external' AND f.path NOT LIKE '%__ts_lib__%'
             ORDER BY s.name, t.name",
        )
        .unwrap();
    stmt.query_map([kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(source: &str, kind: &str, target: &str, file: &str) -> (String, String, String, String) {
    (source.into(), kind.into(), target.into(), file.into())
}

/// Calls and constructions through a barrel-forwarded import bind to the
/// declaring leaf by identity: every barrel between the package entry and the
/// leaf is materialized, so the module graph walks the forwarded surface.
#[test]
fn barrel_forwarded_imports_bind_calls_to_the_declaring_leaf() {
    let node_modules = seed_barrel_node_modules();
    let project = seed_barrel_consumer();
    let db = index_with_node_modules(node_modules.path(), &project);

    let barrels: Vec<String> = {
        let mut stmt = db
            .prepare("SELECT path FROM files WHERE origin = 'external' AND path LIKE '%barrel-pkg%index.d.ts' ORDER BY path")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        barrels,
        vec![
            "ext:ts:barrel-pkg/dist/decorators/index.d.ts".to_string(),
            "ext:ts:barrel-pkg/dist/index.d.ts".to_string(),
            "ext:ts:barrel-pkg/index.d.ts".to_string(),
        ],
        "every relative re-export hop from the entry must be materialized"
    );

    let leaf = "ext:ts:barrel-pkg/dist/decorators/foo.decorator.d.ts";
    // The decorator call is owned by the decorated class; the body calls by
    // the method.
    assert_eq!(
        external_edges(&db, "calls"),
        vec![
            edge("Ctl", "calls", "Foo", leaf),
            edge("run", "calls", "Baz", "ext:ts:barrel-pkg/dist/baz.d.ts"),
            edge("run", "calls", "go", leaf),
        ],
        "Foo() through three barrels, Baz() through a named re-export, b.go() on the bound construction"
    );
    assert_eq!(
        external_edges(&db, "instantiates"),
        vec![edge("run", "instantiates", "Bar", leaf)],
    );

    let unlinked: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE cause_kind = 'unbound_import_unlinked'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unlinked, 0, "no import through the barrel chain may stay unlinked");
}

/// A package whose published entry (`exports["."].types`) sits deeper than a
/// shallow sibling declaration file that also re-exports (vitest's
/// `config.d.ts` beside `dist/index.d.ts`): the ecosystem's entry wins over any
/// depth pick, so `import { expect } from 'vt'` reaches the real surface.
fn seed_deep_entry_node_modules() -> TempDir {
    let nm = TempDir::new().unwrap();
    let pkg = nm.path().join("vt");
    fs::create_dir_all(pkg.join("dist").join("chunks")).unwrap();
    fs::write(
        pkg.join("package.json"),
        r#"{"name":"vt","version":"1.0.0","main":"./dist/index.js","types":"./dist/index.d.ts","exports":{".":{"types":"./dist/index.d.ts","default":"./dist/index.js"},"./config":{"types":"./config.d.ts"}}}"#,
    )
    .unwrap();
    fs::write(pkg.join("config.d.ts"), "export * from './dist/config.js';
").unwrap();
    fs::write(
        pkg.join("dist").join("config.d.ts"),
        "export declare function defineConfig(c: object): object;
",
    )
    .unwrap();
    fs::write(
        pkg.join("dist").join("index.d.ts"),
        "import './chunks/global.d.AbCd.js';
export { e as expect } from './chunks/expect.d.ts';
export * from './chunks/runner.js';
",
    )
    .unwrap();
    fs::write(
        pkg.join("dist").join("chunks").join("expect.d.ts"),
        "declare function e(value: unknown): { toBe(v: unknown): void };
export { e };
",
    )
    .unwrap();
    fs::write(
        pkg.join("dist").join("chunks").join("runner.d.ts"),
        "export declare function it(name: string, fn: () => void): void;
",
    )
    .unwrap();
    // A side-effect import carries augmentations; a compiler loads it too.
    fs::write(
        pkg.join("dist").join("chunks").join("global.d.AbCd.d.ts"),
        "declare global {
    const vtGlobal: number;
}
export {};
",
    )
    .unwrap();
    nm
}

#[test]
fn a_published_package_entry_outranks_a_shallower_declaration_file() {
    let node_modules = seed_deep_entry_node_modules();
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"consumer","version":"0.0.1","devDependencies":{"vt":"^1.0.0"}}"#,
    );
    project.add_file(
        "src/a.test.ts",
        "import { expect, it } from 'vt';

it('works', () => {
  expect(1).toBe(1);
});
",
    );
    let db = index_with_node_modules(node_modules.path(), &project);

    let entries: String = db
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key = 'module_entries_v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        entries.contains("[\"vt\",\"ext:ts:vt/dist/index.d.ts\"]"),
        "the ecosystem's entry is the module entry: {entries}"
    );
    let loaded: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path = 'ext:ts:vt/dist/chunks/global.d.AbCd.d.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(loaded, 1, "a side-effect import of the entry is materialized like any relative hop");
    let found = external_edges(&db, "calls");
    for expected in [
        edge("a.test", "calls", "it", "ext:ts:vt/dist/chunks/runner.d.ts"),
        // `export { e as expect }`: the binding is the declaration `e`.
        edge("a.test", "calls", "e", "ext:ts:vt/dist/chunks/expect.d.ts"),
    ] {
        assert!(
            found.iter().any(|e| e.1 == expected.1 && e.2 == expected.2 && e.3 == expected.3),
            "missing {expected:?} in {found:?}"
        );
    }
}
