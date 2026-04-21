//! Integration tests for Starlark / Bazel BUILD file indexing and resolution.
//!
//! Verifies:
//!   1. Starlark symbols and refs are extracted from .bzl and BUILD files.
//!   2. ctx.* chains produce REAL resolved edges (strategy "starlark_ctx_chain")
//!      against the synthetic ctx API symbols — not opaque external classification.
//!   3. repository_ctx.* chains are classified as external (predicate fallback for
//!      refs the chain walker misses — e.g. uncommon members not in CTX_MEMBERS).
//!   4. env.expect.* (3-level analysistest chains) are classified as external.
//!   5. Native rules (cc_library, genrule) resolve to the synthetic builtin file.
//!   6. Synthetic ctx API symbols land in the database.
//!   7. Chain-walker produces edges: ctx.actions.run_shell resolves to its synthetic
//!      symbol; ctx.label.name resolves to its synthetic symbol.

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
fn ctx_chain_refs_produce_real_resolved_edges() {
    // Round 2: ctx.* chains that appear in CTX_MEMBERS are resolved to real
    // internal edges via strategy "starlark_ctx_chain", NOT opaque external refs.
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // No ctx.* refs left unresolved.
    let ctx_unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name LIKE 'ctx.%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        ctx_unresolved, 0,
        "ctx.* refs must never be unresolved — found {ctx_unresolved}"
    );

    // Known ctx.* members in CTX_MEMBERS resolve via the chain walker → edges table.
    let ctx_chain_edges: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE strategy = 'starlark_ctx_chain'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        ctx_chain_edges > 0,
        "expected ctx.* chain-walker edges in edges table (strategy starlark_ctx_chain), got 0"
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

/// Round 2 test: ctx.actions.run_shell resolves to the synthetic symbol via
/// the chain walker, producing a real edge rather than external classification.
#[test]
fn ctx_actions_run_shell_resolves_to_synthetic_symbol() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // The synthetic symbol ctx.actions.run_shell must be the target of at least
    // one resolved edge emitted by the chain walker.
    let run_shell_edges: i64 = db
        .query_row(
            r#"
            SELECT COUNT(*) FROM edges e
            JOIN symbols t ON t.id = e.target_id
            WHERE t.qualified_name = 'ctx.actions.run_shell'
              AND e.strategy = 'starlark_ctx_chain'
            "#,
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        run_shell_edges > 0,
        "expected at least one edge targeting ctx.actions.run_shell via starlark_ctx_chain, got 0"
    );
}

/// Round 2 test: ctx.actions.declare_file (3-level chain) resolves to its
/// synthetic symbol via the chain walker.
#[test]
fn ctx_actions_declare_file_resolves_to_synthetic_symbol() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // `ctx.actions.declare_file(ctx.label.name + ".out")` in my_rule.bzl —
    // the call site is a 3-level chain, resolved by the chain walker.
    let declare_file_edges: i64 = db
        .query_row(
            r#"
            SELECT COUNT(*) FROM edges e
            JOIN symbols t ON t.id = e.target_id
            WHERE t.qualified_name = 'ctx.actions.declare_file'
              AND e.strategy = 'starlark_ctx_chain'
            "#,
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        declare_file_edges > 0,
        "expected at least one edge targeting ctx.actions.declare_file via starlark_ctx_chain, got 0"
    );
}

/// Round 3 test: synthetic env API symbols (env.expect, env_expect.that_str,
/// env_str_subject.equals, env_collection_subject.contains) land in the database.
#[test]
fn synth_env_api_symbols_in_database() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // env.expect — top-level member on env.
    let env_expect: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'env.expect'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(env_expect, 1, "env.expect must be in symbol table");

    // env_expect.that_str — type-level member.
    let that_str: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'env_expect.that_str'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(that_str, 1, "env_expect.that_str must be in symbol table");

    // env_str_subject.equals — assertion method on string subject.
    let str_equals: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'env_str_subject.equals'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        str_equals, 1,
        "env_str_subject.equals must be in symbol table"
    );

    // env_collection_subject.contains — assertion method on collection subject.
    let coll_contains: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'env_collection_subject.contains'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        coll_contains, 1,
        "env_collection_subject.contains must be in symbol table"
    );
}

/// Round 3 test: env.expect.that_str (3-level chain) resolves to the synthetic
/// flat-alias symbol via the chain walker, producing a real starlark_ctx_chain edge.
#[test]
fn env_expect_that_str_resolves_to_synthetic_symbol() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // The flat alias `env.expect.that_str` (matching the extractor's dotted ref)
    // must be the target of at least one edge emitted by the chain walker.
    let edges: i64 = db
        .query_row(
            r#"
            SELECT COUNT(*) FROM edges e
            JOIN symbols t ON t.id = e.target_id
            WHERE t.qualified_name = 'env.expect.that_str'
              AND e.strategy = 'starlark_ctx_chain'
            "#,
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        edges > 0,
        "expected at least one edge targeting env.expect.that_str via starlark_ctx_chain, got 0"
    );
}

/// Round 3 test: env.expect.that_collection (3-level chain from analysistest_impl.bzl)
/// resolves to the synthetic flat-alias symbol via the chain walker.
#[test]
fn env_expect_that_collection_resolves_to_synthetic_symbol() {
    let project = TestProject::starlark_bazel_project();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    let edges: i64 = db
        .query_row(
            r#"
            SELECT COUNT(*) FROM edges e
            JOIN symbols t ON t.id = e.target_id
            WHERE t.qualified_name = 'env.expect.that_collection'
              AND e.strategy = 'starlark_ctx_chain'
            "#,
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(
        edges > 0,
        "expected at least one edge targeting env.expect.that_collection via starlark_ctx_chain, got 0"
    );
}
