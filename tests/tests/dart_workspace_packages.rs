//! A Dart `package:` import of a sibling workspace package binds to that
//! package's own source: the declared name answers to its package-URI
//! spelling, and the package's declared source root maps the URI's sub-path
//! onto the indexed file. Both import forms are covered — the plain
//! whole-library import and the prefixed one generated code emits — and a
//! same-named template stub must not satisfy either.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_pub_workspace() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "pubspec.yaml",
        "name: demo_workspace\n\nenvironment:\n  sdk: '^3.8.0'\n",
    );
    project.add_file(
        "packages/core_client/pubspec.yaml",
        "name: core_client\nversion: 1.0.0\n\nenvironment:\n  sdk: '^3.8.0'\n",
    );
    project.add_file(
        "packages/core_client/lib/core_client.dart",
        "export 'src/manager.dart';\nexport 'package:core_serialization/core_serialization.dart';\n",
    );
    project.add_file(
        "packages/core_client/lib/src/manager.dart",
        "abstract class SerializationManager {\n  String encode(Object value);\n}\n",
    );
    project.add_file(
        "packages/core_serialization/pubspec.yaml",
        "name: core_serialization\nversion: 1.0.0\n\nenvironment:\n  sdk: '^3.8.0'\n",
    );
    // The client library re-exports the serialization package wholesale, the
    // way a generated client forwards its shared model library.
    project.add_file(
        "packages/core_serialization/lib/core_serialization.dart",
        "abstract class SerializableModel {\n  String serialize();\n}\n",
    );
    project.add_file(
        "packages/app_server/pubspec.yaml",
        "name: app_server\nversion: 1.0.0\n\nenvironment:\n  sdk: '^3.8.0'\n\ndependencies:\n  core_client: 1.0.0\n",
    );
    project.add_file(
        "packages/app_server/pubspec_overrides.yaml",
        "dependency_overrides:\n  core_client:\n    path: ../core_client\n",
    );
    project.add_file(
        "packages/app_server/lib/endpoint.dart",
        "import 'package:core_client/core_client.dart';\n\nclass Endpoint {\n  final SerializationManager manager;\n  final SerializableModel model;\n  Endpoint(this.manager, this.model);\n\n  String run() => manager.encode(model.serialize());\n}\n",
    );
    // The prefixed form generated Dart clients emit: the import binds no name
    // directly, so every use carries the raw URI on the ref itself.
    project.add_file(
        "packages/app_server/lib/client.dart",
        "import 'package:core_client/core_client.dart' as _i1;\n\nclass Client implements _i1.SerializableModel {\n  final _i1.SerializationManager codec;\n  Client(this.codec);\n\n  @override\n  String serialize() => '';\n}\n",
    );
    // A packaging template that declares the same name but ships no sources.
    // It must never win the declared name away from the real package.
    project.add_file(
        "templates/pubspecs/packages/core_client/pubspec.yaml",
        "name: core_client\nversion: 1.0.0\n",
    );
    project
}

/// Every edge as `(source file, kind, target name, target file)`. The target
/// FILE is the assertion that matters: it is what separates the real package
/// from a same-named stub.
fn cross_file_edges(db: &bearwisdom::Database) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT sf.path, e.kind, t.name, tf.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files tf ON tf.id = t.file_id
             ORDER BY sf.path, e.kind, t.name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(
    source_file: &str,
    kind: &str,
    target: &str,
    target_file: &str,
) -> (String, String, String, String) {
    (
        source_file.into(),
        kind.into(),
        target.into(),
        target_file.into(),
    )
}

/// Every live unresolved ref in the consuming package other than the import
/// directives themselves: a Dart library declares no file-level symbol, so an
/// `import 'uri'` has nothing to bind to even when its file is linked.
fn live_unresolved_in_app_server(
    db: &bearwisdom::Database,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let mut stmt = db
        .prepare(
            "SELECT u.target_name, u.kind, u.module, u.cause_kind FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files f ON f.id = s.file_id
             WHERE u.drained = 0 AND u.kind <> 'imports'
               AND f.path LIKE 'packages/app_server/%'
             ORDER BY u.target_name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_package_uri_binds_to_the_sibling_workspace_package_source() {
    let project = seed_pub_workspace();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let found = cross_file_edges(&db);
    const DECLARING_FILE: &str = "packages/core_client/lib/src/manager.dart";
    const PLAIN_IMPORT: &str = "packages/app_server/lib/endpoint.dart";
    const PREFIXED_IMPORT: &str = "packages/app_server/lib/client.dart";
    for expected in [
        // The plain import's field type, the prefixed import's field type, and
        // the member call through the bound field.
        edge(
            PLAIN_IMPORT,
            "type_ref",
            "SerializationManager",
            DECLARING_FILE,
        ),
        edge(
            PREFIXED_IMPORT,
            "type_ref",
            "SerializationManager",
            DECLARING_FILE,
        ),
        edge(PLAIN_IMPORT, "calls", "encode", DECLARING_FILE),
        // A name the client library only re-exports from a third workspace
        // package binds to that package's declaration.
        edge(
            PREFIXED_IMPORT,
            "implements",
            "SerializableModel",
            "packages/core_serialization/lib/core_serialization.dart",
        ),
        edge(
            PLAIN_IMPORT,
            "type_ref",
            "SerializableModel",
            "packages/core_serialization/lib/core_serialization.dart",
        ),
        edge(
            PLAIN_IMPORT,
            "calls",
            "serialize",
            "packages/core_serialization/lib/core_serialization.dart",
        ),
    ] {
        assert!(
            found.contains(&expected),
            "missing {expected:?} in {found:?}"
        );
    }

    let unresolved = live_unresolved_in_app_server(&db);
    assert!(
        unresolved.is_empty(),
        "every ref in the consuming package resolves: {unresolved:?}"
    );
}
