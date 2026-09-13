//! A `.zig` file is itself a struct, and `@import("../std.zig")` yields that
//! struct. The file-struct symbol is what a relative `@import` binds to — so
//! the import edge lands on the target file's own container, at any `../`
//! depth, and a target with no file behind it stays honestly unresolved.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "lib/std/std.zig",
        "const mem = @import(\"mem.zig\");\n\npub fn debugPrint() void {}\n",
    );
    project.add_file(
        "lib/std/mem.zig",
        "pub fn copy() void {}\n\npub fn eql() bool {\n    return true;\n}\n",
    );
    project.add_file(
        "lib/std/Build/Step.zig",
        "const std = @import(\"../std.zig\");\nconst missing = @import(\"Build.zig\");\n\npub fn make() void {}\n",
    );
    project.add_file(
        "lib/std/Build/Cache/Path.zig",
        "const std = @import(\"../../std.zig\");\n\npub fn join() void {}\n",
    );
    project
}

/// `(source symbol, source file, target symbol, target file)` for every
/// `imports` edge in the index.
fn import_edges(db: &bearwisdom::Database) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, sf.path, t.name, tf.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files tf ON tf.id = t.file_id
             WHERE e.kind = 'imports' AND sf.language = 'zig'
             ORDER BY sf.path, t.name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(
    source: &str,
    source_file: &str,
    target: &str,
    target_file: &str,
) -> (String, String, String, String) {
    (
        source.into(),
        source_file.into(),
        target.into(),
        target_file.into(),
    )
}

#[test]
fn zig_relative_imports_bind_to_the_target_file_struct() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let edges = import_edges(&db);
    for expected in [
        // One `../` hop.
        edge("std", "lib/std/Build/Step.zig", "std", "lib/std/std.zig"),
        // Two `../` hops — the candidate path normalizes before the lookup.
        edge("std", "lib/std/Build/Cache/Path.zig", "std", "lib/std/std.zig"),
        // Same-directory specifier.
        edge("mem", "lib/std/std.zig", "mem", "lib/std/mem.zig"),
    ] {
        assert!(edges.contains(&expected), "missing {expected:?} in {edges:?}");
    }
}

#[test]
fn a_missing_zig_import_target_stays_unresolved() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let unlinked: Vec<String> = {
        let mut stmt = db
            .prepare(
                "SELECT u.target_name FROM unresolved_refs u
                 JOIN symbols s ON s.id = u.source_id
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'zig' AND u.cause_kind = 'unbound_import_unlinked'
                 ORDER BY u.target_name",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        unlinked.contains(&"Build.zig".to_string()),
        "a specifier with no file behind it must not bind: {unlinked:?}"
    );
}
