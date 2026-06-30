use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn resolve(lookup: &Lookup, target: &str, imports: Vec<ImportEntry>) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match ImportedNamespaceRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_when_qname_starts_with_imported_module() {
    // `using FamilyBudget.Api.Entities;` → candidate qname starts with the
    // module path, boundary-checked by a `.` after it.
    let lookup = Lookup::new().with(sym(
        40,
        "Transaction",
        "FamilyBudget.Api.Entities.Transaction",
        "class",
        "src/transaction.cs",
    ));
    let imports = vec![import("*", Some("FamilyBudget.Api.Entities"))];
    assert_eq!(resolve(&lookup, "Transaction", imports), Some(40));
}

#[test]
fn boundary_check_prevents_accidental_match() {
    // `FamilyBudget.Api.EntitiesOther` does NOT start with
    // `FamilyBudget.Api.Entities.` (the trailing `.` boundary).
    let lookup = Lookup::new().with(sym(
        41,
        "Transaction",
        "FamilyBudget.Api.EntitiesOther.Transaction",
        "class",
        "src/t.cs",
    ));
    let imports = vec![import("*", Some("FamilyBudget.Api.Entities"))];
    assert_eq!(resolve(&lookup, "Transaction", imports), None);
}

#[test]
fn binds_via_file_path_match() {
    // The module path `posthog.models` matches the candidate's file path
    // `posthog/models/person.py` via the segment-run check.
    let lookup = Lookup::new().with(sym(
        42,
        "Person",
        "posthog.models.person.Person",
        "class",
        "posthog/models/person.py",
    ));
    let imports = vec![import("*", Some("posthog.models"))];
    assert_eq!(resolve(&lookup, "Person", imports), Some(42));
}

#[test]
fn declines_when_no_import_matches() {
    let lookup = Lookup::new().with(sym(
        43,
        "Widget",
        "com.example.Widget",
        "class",
        "src/Widget.java",
    ));
    let imports = vec![import("*", Some("com.other"))];
    assert_eq!(resolve(&lookup, "Widget", imports), None);
}

#[test]
fn rust_use_import_resolves_bare_name_across_hyphenated_crate_dir() {
    // `use turbo_tasks::Vc` imports `Vc` from a crate whose on-disk directory
    // is `turbo-tasks/` (hyphens). The import module string uses underscores
    // (`turbo_tasks`); the file path uses hyphens (`turbo-tasks`). The rule
    // must resolve the bare `Vc` ref to the struct and not pick a competitor
    // from an unrelated path.
    let lookup = Lookup::new()
        .with(sym(
            100,
            "Vc",
            "Vc",
            "struct",
            "turbopack/crates/turbo-tasks/src/vc/mod.rs",
        ))
        .with(sym(
            200,
            "Vc",
            "Vc",
            "function",
            "turbopack/crates/turbopack-ecmascript/tests/input.js",
        ));
    let imports = vec![import("Vc", Some("turbo_tasks"))];
    assert_eq!(resolve(&lookup, "Vc", imports), Some(100));
}

#[test]
fn rust_use_import_does_not_match_longer_crate_with_same_prefix() {
    // `use turbo_tasks::Vc` must not resolve to a symbol in `turbo-tasks-macros/`
    // even after hyphen normalization: `turbo_tasks_macros` is a longer segment
    // than `turbo_tasks` and the boundary check must reject it. If both the
    // macros symbol and the real struct are present, the struct wins.
    let lookup = Lookup::new()
        .with(sym(
            300,
            "Vc",
            "ReceiverStyle.Vc",
            "enum_member",
            "turbopack/crates/turbo-tasks-macros/src/func.rs",
        ))
        .with(sym(
            100,
            "Vc",
            "Vc",
            "struct",
            "turbopack/crates/turbo-tasks/src/vc/mod.rs",
        ));
    let imports = vec![import("Vc", Some("turbo_tasks"))];
    assert_eq!(resolve(&lookup, "Vc", imports), Some(100));
}

#[test]
fn import_scope_picks_the_workspace_package_candidate_over_first_by_name() {
    // `useQuery` declared in two sibling packages; the file imports it from
    // query-core, so the bind must pick pkg-10's def even though pkg-19's is
    // first in the by-name order.
    let lookup = Lookup::new()
        .with_workspace_pkg("@tanstack/query-core", 10)
        .with_in_package(
            19,
            sym(900, "useQuery", "useQuery", "function", "packages/solid-query/src/useQuery.ts"),
        )
        .with_in_package(
            10,
            sym(910, "useQuery", "useQuery", "function", "packages/query-core/src/useQuery.ts"),
        );
    let imports = vec![import("useQuery", Some("@tanstack/query-core"))];
    assert_eq!(resolve(&lookup, "useQuery", imports), Some(910));
}
