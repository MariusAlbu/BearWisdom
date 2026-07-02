//! Dart resolution probes — sibling to `resolution_corpus.rs`, same per-fix
//! harness shape: one in-memory `TestProject` indexed once through the real
//! pipeline, self-validating preconditions, asserted-green probes plus
//! printed known-red candidates carrying their traced root cause.
//!
//! `.dart_tool/package_config.json` seeds a stub pub package (mirrors the
//! `@base-ui/react` stub in `resolution_corpus.rs`) and `BEARWISDOM_DART_SDK`
//! is pointed at an EMPTY stub `lib/` — this short-circuits the SDK probe
//! chain (`probe_dart_sdk_lib` in `ecosystem/dart_sdk.rs`) so a `DateTime`
//! reference stays deterministically unresolved regardless of whether the
//! host machine happens to have a real Dart SDK on `PATH`.

use bearwisdom::full_index;
use bearwisdom::Database;
use bearwisdom_tests::TestProject;
use rusqlite::params;
use tempfile::TempDir;

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`,
/// whose resolved target qname matches `target_like` (SQL LIKE).
fn count_resolved_to(db: &Database, file_suffix: &str, callee: &str, target_like: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2
           AND t.qualified_name LIKE ?3",
        params![format!("%{file_suffix}"), callee, target_like],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`,
/// regardless of the target's qname.
fn count_resolved(db: &Database, file_suffix: &str, callee: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2",
        params![format!("%{file_suffix}"), callee],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count unresolved `calls` refs for `callee` in the file ending `file_suffix`.
fn count_unresolved(db: &Database, file_suffix: &str, callee: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM unresolved_refs u
         JOIN symbols s ON u.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         WHERE f.path LIKE ?1 AND u.kind = 'calls' AND u.target_name = ?2",
        params![format!("%{file_suffix}"), callee],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn resolution_corpus_dart() {
    // Container directory: the project lives at `<container>/app`, and the
    // stub pub package lives at `<container>/_dart_pub_cache/...` — a
    // sibling of `app`, matching the relative `rootUri` resolution rule
    // `.dart_tool/package_config.json` URIs are resolved against (relative
    // to the `.dart_tool/` directory itself, per the Dart package-config spec).
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    let root = project.path().join("app");

    project.add_file(
        "app/pubspec.yaml",
        "name: resolution_corpus_dart\ndependencies:\n  widgetlib: ^1.0.0\n",
    );
    project.add_file(
        "app/.dart_tool/package_config.json",
        r#"{
  "configVersion": 2,
  "packages": [
    {
      "name": "widgetlib",
      "rootUri": "../../_dart_pub_cache/widgetlib-1.0.0",
      "packageUri": "lib/",
      "version": "1.0.0"
    }
  ]
}
"#,
    );
    // A same-directory-tree EXTERNAL package: `widgetlib` exports `Widget`
    // with a `render()` member — the pub-external counterpart to
    // `resolution_corpus.rs`'s `@base-ui/react` stub.
    project.add_file(
        "_dart_pub_cache/widgetlib-1.0.0/lib/widgetlib.dart",
        "class Widget {\n  void render() {}\n}\n",
    );

    // Three constructs, one file: a local-var receiver (`c.dispose()`), a
    // parameter receiver on the seeded pub-external type (`w.render()`), and
    // an unseeded SDK reference (`DateTime.now()`). Each probe's outcome and
    // traced root cause is documented at its assertion below.
    project.add_file(
        "app/lib/main.dart",
        r#"import 'package:widgetlib/widgetlib.dart';

class Controller {
  void dispose() {}
}

void useController() {
  final c = Controller();
  c.dispose();
}

void useWidget(Widget w) {
  w.render();
}

DateTime getClock() {
  return DateTime.now();
}
"#,
    );

    // An empty `lib/` under the override short-circuits `probe_dart_sdk_lib`
    // (`ecosystem/dart_sdk.rs`) at its first probe step, so the DateTime probe
    // stays deterministic even on a machine with a real Dart SDK on PATH.
    let dart_sdk_stub = TempDir::new().unwrap();
    std::fs::create_dir_all(dart_sdk_stub.path().join("lib")).unwrap();

    let prior_sdk = std::env::var_os("BEARWISDOM_DART_SDK");
    unsafe {
        std::env::set_var("BEARWISDOM_DART_SDK", dart_sdk_stub.path());
    }

    let mut db = TestProject::in_memory_db();
    let result = full_index(&mut db, &root, None, None, None);

    unsafe {
        match prior_sdk {
            Some(v) => std::env::set_var("BEARWISDOM_DART_SDK", v),
            None => std::env::remove_var("BEARWISDOM_DART_SDK"),
        }
    }
    result.expect("index failed");

    // Precondition: the stub pub package MUST be indexed, else the
    // parameter-receiver pattern passes vacuously.
    let stub_widget: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='Widget' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("\n--- preconditions ---");
    println!("  stub pub package Widget indexed (package_config.json): {stub_widget}");
    assert!(
        stub_widget >= 1,
        "precondition: external `widgetlib.Widget` must be indexed, else the pub-external pattern \
         passes vacuously"
    );

    // Candidate probes — KNOWN-RED, diagnostic only (not asserted). Root
    // causes (traced):
    //   param-widget — `useWidget(Widget w)`'s parameter is never captured
    //     with a type anywhere: `dart/symbols.rs`'s `extract_top_level_function`
    //     and `extract_method` both hardcode `param_types: Vec::new()` and
    //     never walk the node's `formal_parameter_list`, so no Variable
    //     symbol and no `declared_type` exist for `w` at all. Independent of
    //     the pub-external seam — the precondition above confirms
    //     `widgetlib.Widget` DOES index correctly; the gap is upstream of
    //     externals entirely (every Dart function/method parameter, project
    //     or external, is untyped this way).
    //   sdk-unseeded — `BEARWISDOM_DART_SDK` is deliberately pointed at an
    //     empty `lib/` (see setup above) so this probe reflects an
    //     unconfigured project: no `dart:core` types are indexed, so
    //     `DateTime` has nothing to bind to. A real Dart SDK IS locatable
    //     (`BEARWISDOM_DART_SDK`/`DART_SDK`/`FLUTTER_ROOT`/PATH/well-known
    //     paths, `ecosystem/dart_sdk.rs::probe_dart_sdk_lib`) — this probe
    //     specifically isolates the "nothing configured" case, not a locator
    //     gap.
    println!("\n--- candidate probes (known red) ---");
    println!(
        "  param-widget    w.render()   resolved-to-Widget={} unresolved={}",
        count_resolved_to(&db, "main.dart", "render", "%Widget%"),
        count_unresolved(&db, "main.dart", "render")
    );
    println!(
        "  sdk-unseeded    DateTime.now() resolved={} unresolved={}",
        count_resolved(&db, "main.dart", "now"),
        count_unresolved(&db, "main.dart", "now")
    );

    // `final c = Controller(); c.dispose();` resolves today — but NOT via
    // genuine local-variable typing: Dart never creates a Variable symbol
    // for `c` either (`dart/extract.rs`'s `initialized_variable_definition`
    // arm is guarded on `parent_index.is_none()`, so it only fires at module
    // scope, never inside a function/method body), and no chain segment
    // ever carries a `declared_type` (`dart/calls.rs` sets `declared_type:
    // None` on every segment it builds). The resolution comes from a
    // same-project bare-name fallback, not from typing `c` — asserted here
    // as a real, reproducible-today behavior the harness should catch a
    // regression on, not as proof local-variable typing works.
    let controller_dispose = count_resolved_to(&db, "main.dart", "dispose", "%Controller%");

    let checks = [(
        "local var (untyped)  c.dispose() -> Controller.dispose",
        controller_dispose >= 1,
        format!("resolved-to-Controller edges = {controller_dispose}"),
    )];

    println!("\n=== resolution corpus (dart) ===");
    let mut failures = Vec::new();
    for (label, pass, detail) in &checks {
        println!("  {} {label}  [{detail}]", if *pass { "✅" } else { "❌" });
        if !pass {
            failures.push(*label);
        }
    }
    println!(
        "  {} / {} patterns resolved as expected\n",
        checks.len() - failures.len(),
        checks.len()
    );

    assert!(
        failures.is_empty(),
        "resolution corpus (dart) regressions: {failures:?}"
    );
}
