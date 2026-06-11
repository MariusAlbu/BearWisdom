// =============================================================================
// prolog/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// Prolog predicates are a flat top-level name/arity namespace. A rule-body goal
// calls a sibling predicate bare by functor; the extractor emits it as a Calls
// ref with target_name = bare functor (symbol.name = bare functor,
// qualified_name = functor/arity). With no scope/import structure to bind
// through, `namespaceless_global_type_lookup` binds such a bare call first-match
// to the project's own predicate definition via the dead-last by-name rung.
// A same-named library predicate with no project definition declines and stays
// external; a module-qualified `lists:member` call carries the `:` in its
// target_name, so the bare-name rung never sees it.
// =============================================================================

use super::PROLOG_PROFILE;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::types::*;
use std::collections::HashMap;

#[test]
fn prolog_profile_identity_and_shadow_mode() {
    assert_eq!(PROLOG_PROFILE.id, "prolog");
    assert_eq!(PROLOG_PROFILE.qname_separator, ":");
}

#[test]
fn prolog_namespaceless_global_is_on() {
    // Prolog's flat predicate namespace binds bare functor calls to the
    // project's own predicate definitions via the dead-last by-name rung.
    assert_eq!(
        PROLOG_PROFILE.namespaceless_global_type_lookup,
        crate::type_checker::profile::language_profile::NamespaceScope::Global
    );
}

// ---------------------------------------------------------------------------
// Full-ladder bind tests through resolve_all_with_profile(&PROLOG_PROFILE).
// ---------------------------------------------------------------------------

fn accept_any(_edge: EdgeKind, _sym_kind: &str) -> bool {
    true
}

fn make_pred(functor: &str, arity: u32) -> ExtractedSymbol {
    // Mirror the extractor: name = bare functor, qualified_name = functor/arity.
    ExtractedSymbol {
        name: functor.to_string(),
        qualified_name: format!("{functor}/{arity}"),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_call(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 2,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "prolog".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 10,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn clone_pf(f: &ParsedFile) -> ParsedFile {
    make_file(&f.path, f.symbols.clone(), f.refs.clone())
}

fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files.iter().map(|f| clone_pf(f)).collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

fn sym_id(id_map: &HashMap<(String, String), i64>, file: &str, qname: &str) -> i64 {
    *id_map
        .get(&(file.to_string(), qname.to_string()))
        .unwrap_or_else(|| panic!("symbol not found: {file}::{qname}"))
}

fn resolve_call(file_path: &str, target: &str, all: &[&ParsedFile]) -> Option<Resolution> {
    let (index, _) = build_index(all);
    let caller = make_file(
        file_path,
        vec![make_pred("caller", 0)],
        vec![make_call(target)],
    );
    let file_ctx = FileContext {
        file_path: file_path.to_string(),
        language: "prolog".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let r = &caller.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &index,
        kind_compatible: accept_any,
    }
    .resolve_all_with_profile(&PROLOG_PROFILE)
}

#[test]
fn prolog_bare_predicate_binds_internal_over_external() {
    // `solve` is the project's own predicate (two clauses across .pl files,
    // distinct arities) plus a same-named ext: library stub. A bare cross-file
    // call binds an INTERNAL definition first-match; the external stub loses.
    let a = make_file("src/solve.pl", vec![make_pred("solve", 2)], vec![]);
    let b = make_file("src/solve_aux.pl", vec![make_pred("solve", 3)], vec![]);
    let ext = make_file(
        "ext:prolog-runtime/library/solve.pl",
        vec![make_pred("solve", 2)],
        vec![],
    );
    let (id_a, id_b, ext_id) = {
        let (_, id_map) = build_index(&[&a, &b, &ext]);
        (
            sym_id(&id_map, "src/solve.pl", "solve/2"),
            sym_id(&id_map, "src/solve_aux.pl", "solve/3"),
            sym_id(&id_map, "ext:prolog-runtime/library/solve.pl", "solve/2"),
        )
    };
    let res = resolve_call("src/main.pl", "solve", &[&a, &b, &ext])
        .expect("bare Prolog predicate call binds an internal definition");
    assert_eq!(res.strategy, "default_namespaceless_global");
    assert_ne!(
        res.target_symbol_id, ext_id,
        "must not bind the ext library stub"
    );
    assert!(
        res.target_symbol_id == id_a || res.target_symbol_id == id_b,
        "binds an internal solve (got {})",
        res.target_symbol_id
    );
}

#[test]
fn prolog_external_only_predicate_stays_unresolved() {
    // `format` is owned ONLY by an external library file — no project clause.
    // The internal-only rung declines, leaving it for external classification.
    let ext = make_file(
        "ext:prolog-runtime/library/format.pl",
        vec![make_pred("format", 2)],
        vec![],
    );
    let res = resolve_call("src/main.pl", "format", &[&ext]);
    assert!(
        res.is_none(),
        "external-only predicate must not bind an internal symbol; got: {res:?}"
    );
}

#[test]
fn prolog_module_qualified_call_does_not_bind_bare_predicate() {
    // A `lists:member(X,L)` goal extracts target_name = "lists:member" (the `:`
    // survives `split('(')`). The bare-name rung keys on `.name` = bare functor,
    // so it never coincidentally matches a project predicate named `member`.
    let internal = make_file("src/util.pl", vec![make_pred("member", 2)], vec![]);
    let res = resolve_call("src/main.pl", "lists:member", &[&internal]);
    assert!(
        res.is_none(),
        "module-qualified call must not bind the bare internal predicate; got: {res:?}"
    );
}
