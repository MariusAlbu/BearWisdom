//! A global name has two faces: the shape it contributes to the type space and
//! the value it binds in the value space. A dependency is free to write them in
//! different declaration files of one package, and a member access on the bare
//! name reaches the VALUE's declared type — the shape carries the instance
//! members, never the static surface.
//!
//! The control case is the opposite shape: a same-named value in an unrelated
//! package is a collision, not the other package's value face, so the type
//! declaration keeps the root.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn index(project: &TestProject) -> bearwisdom::Database {
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

/// `(target name, declaring file)` for every edge leaving the consumer source.
fn consumer_targets(db: &bearwisdom::Database) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE sf.path = 'src/app.ts'
             ORDER BY t.name, f.path",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Undrained member misses still filed against a receiver declaration.
fn member_misses(db: &bearwisdom::Database) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT u.target_name FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path = 'src/app.ts' AND u.drained = 0
               AND u.cause_kind = 'member_missing'
             ORDER BY u.target_name",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// One package whose global `Ticker` is split across declaration files: the
/// instance shape in `shape.d.ts`, the constructor value in `value.d.ts`. The
/// entry re-exports both so the demand closure reaches them; the consumer
/// imports only the marker, so the chain root is a bare unimported name.
fn seed_split_global(project: &TestProject) {
    project.add_file(
        "package.json",
        r#"{"name":"consumer","dependencies":{"split-globals":"*"}}"#,
    );
    project.add_file(
        "node_modules/split-globals/package.json",
        r#"{"name":"split-globals","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/split-globals/index.d.ts",
        "export * from './shape';\nexport * from './value';\nexport declare const marker: string;\n",
    );
    project.add_file(
        "node_modules/split-globals/shape.d.ts",
        "export interface Ticker {\n    elapsed(): number;\n}\n",
    );
    project.add_file(
        "node_modules/split-globals/value.d.ts",
        "export interface TickerConstructor {\n    start(label: string): number;\n}\n\nexport declare const Ticker: TickerConstructor;\n",
    );
    project.add_file(
        "src/app.ts",
        "import { marker } from 'split-globals';\n\nexport const begin = () => Ticker.start(marker);\n",
    );
}

/// Two packages that merely share a name: `shape-pkg` declares an evaluable
/// `Gauge` with a static surface, `value-pkg` an unrelated constant also called
/// `Gauge`. The consumer imports a marker from each and calls `Gauge.create()`.
fn seed_same_name_across_packages(project: &TestProject) {
    project.add_file(
        "package.json",
        r#"{"name":"consumer","dependencies":{"shape-pkg":"*","value-pkg":"*"}}"#,
    );
    project.add_file(
        "node_modules/shape-pkg/package.json",
        r#"{"name":"shape-pkg","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/shape-pkg/index.d.ts",
        "export declare class Gauge {\n    static create(): string;\n}\n\nexport declare const shapeMarker: string;\n",
    );
    project.add_file(
        "node_modules/value-pkg/package.json",
        r#"{"name":"value-pkg","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/value-pkg/index.d.ts",
        "export interface Recorder {\n    create(): number;\n}\n\nexport declare const Gauge: Recorder;\n\nexport declare const valueMarker: string;\n",
    );
    project.add_file(
        "src/app.ts",
        "import { shapeMarker } from 'shape-pkg';\nimport { valueMarker } from 'value-pkg';\n\nexport const made = () => Gauge.create();\nexport const marks = () => shapeMarker + valueMarker;\n",
    );
}

#[test]
fn a_split_globals_static_surface_binds_through_its_value_declaration() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_split_global(&project);
    let db = index(&project);

    let targets = consumer_targets(&db);
    assert!(
        targets.contains(&(
            "start".to_string(),
            "ext:ts:split-globals/value.d.ts".to_string()
        )),
        "`Ticker.start` lives on the constructor value's declared type: {targets:?}"
    );
    assert!(
        !member_misses(&db).contains(&"start".to_string()),
        "the instance shape must not be blamed for a static member"
    );
}

#[test]
fn a_same_named_value_in_another_package_does_not_take_the_root() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_same_name_across_packages(&project);
    let db = index(&project);

    let targets = consumer_targets(&db);
    assert!(
        targets.contains(&(
            "create".to_string(),
            "ext:ts:shape-pkg/index.d.ts".to_string()
        )),
        "the declaration that owns the static surface keeps the root: {targets:?}"
    );
    assert!(
        !targets.contains(&(
            "create".to_string(),
            "ext:ts:value-pkg/index.d.ts".to_string()
        )),
        "an unrelated package's same-named constant is a collision: {targets:?}"
    );
}
