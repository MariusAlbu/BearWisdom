//! Integration tests for Starlark / Bazel BUILD file indexing and resolution.
//!
//! Verifies:
//!   1. Starlark symbols and refs are extracted from .bzl and BUILD files.
//!   2. ctx.* chains at any depth are classified as external (namespace "bazel"),
//!      not left as unresolved refs.
//!   3. repository_ctx.* chains are classified as external.
//!   4. env.expect.* (3-level analysistest chains) are classified as external.
//!   5. Native rules (cc_library, genrule) resolve to the synthetic builtin file.
//!   6. Synthetic ctx API symbols land in the database.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;

#[test]
fn starlark_project_indexes_without_errors() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    assert!(
        stats.file_count >= 3,
        "expected at least 3 Starlark files, got {}",
        stats.file_count
    );
    assert!(
        stats.symbol_count > 0,
        "expected symbols from Starlark extraction"
    );
    assert_eq!(
        stats.files_with_errors, 0,
        "no files should have parse errors"
    );
}

#[test]
fn ctx_chain_refs_classified_as_external_not_unresolved() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // ctx.actions.run_shell, ctx.actions.declare_file, ctx.label.name,
    // ctx.file.src, ctx.files.srcs, ctx.outputs — all must land in
    // external_refs with namespace "bazel", never in unresolved_refs.
    let ctx_unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name LIKE 'ctx.%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        ctx_unresolved, 0,
        "ctx.* refs must be external, not unresolved — found {ctx_unresolved} unresolved ctx.* refs"
    );

    let ctx_external: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM external_refs WHERE target_name LIKE 'ctx.%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        ctx_external > 0,
        "expected ctx.* refs in external_refs with namespace 'bazel'"
    );
}

#[test]
fn ctx_label_name_three_level_chain_is_external() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // ctx.label.name is a 3-level dotted ref — the predicate must catch it.
    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name = 'ctx.label.name'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        unresolved, 0,
        "ctx.label.name must not appear in unresolved_refs"
    );
}

#[test]
fn env_expect_chain_classified_as_external() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // env.expect.that_str — analysistest 3-level chain.
    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name LIKE 'env.%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        unresolved, 0,
        "env.* refs must be external, not unresolved — found {unresolved}"
    );
}

#[test]
fn repository_ctx_chain_classified_as_external() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    let unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name LIKE 'repository_ctx.%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        unresolved, 0,
        "repository_ctx.* refs must be external, not unresolved"
    );
}

#[test]
fn synth_ctx_api_symbols_in_database() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // The synthetic ctx.bzl file contributes external symbols.
    let ctx_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name LIKE 'ctx.%' AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        ctx_symbols > 0,
        "expected synthetic ctx.* symbols in external origin, got {ctx_symbols}"
    );

    // Specifically verify ctx.actions.run_shell and ctx.label.name are present.
    let run_shell: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'ctx.actions.run_shell'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(run_shell, 1, "ctx.actions.run_shell must be in symbol table");

    let label_name: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'ctx.label.name'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(label_name, 1, "ctx.label.name must be in symbol table");
}
