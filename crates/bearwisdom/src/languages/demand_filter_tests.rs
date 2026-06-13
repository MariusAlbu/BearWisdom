use super::filter_extraction_to_demand;
use crate::types::{
    DbMappingSource, EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol,
    ExtractionResult, SymbolKind, Visibility,
};
use std::collections::HashSet;

fn sym(name: &str, qname: &str, kind: SymbolKind, parent: Option<usize>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn type_ref(source_idx: usize, target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn ref_of_kind(source_idx: usize, target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        kind,
        ..type_ref(source_idx, target)
    }
}

fn demand_of(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Shared fixture: module `m`(0) → Foo(1) → bar(2); Other(3) → baz(4).
// bar refs external `Handle`; baz refs external `Discard`.
// ---------------------------------------------------------------------------

fn nested_fixture() -> ExtractionResult {
    let symbols = vec![
        sym("m", "m", SymbolKind::Module, None),
        sym("Foo", "m.Foo", SymbolKind::Struct, Some(0)),
        sym("bar", "m.Foo.bar", SymbolKind::Field, Some(1)),
        sym("Other", "m.Other", SymbolKind::Struct, Some(0)),
        sym("baz", "m.Other.baz", SymbolKind::Field, Some(3)),
    ];
    let refs = vec![type_ref(2, "Handle"), type_ref(4, "Discard")];
    ExtractionResult {
        symbols,
        refs,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Descendant-closure tests (layer 1 — unchanged from original)
// ---------------------------------------------------------------------------

#[test]
fn keeps_seed_and_descendants_drops_rest() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["Foo"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Foo", "bar"]);
}

#[test]
fn seed_parent_link_drops_when_parent_pruned() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["Foo"]));
    let foo = &filtered.symbols[0];
    assert_eq!(foo.name, "Foo");
    assert_eq!(foo.parent_index, None);
}

#[test]
fn descendant_parent_index_remaps_to_new_seed_index() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["Foo"]));
    // bar was idx 2, parent idx 1; after filtering Foo is new-idx 0, bar is new-idx 1.
    let bar = &filtered.symbols[1];
    assert_eq!(bar.name, "bar");
    assert_eq!(bar.parent_index, Some(0));
}

#[test]
fn kept_ref_source_index_remaps_dropped_ref_removed() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["Foo"]));
    // bar's ref to external `Handle` survives; baz's ref dropped.
    // bar moved old-idx 2 → new-idx 1.
    assert_eq!(filtered.refs.len(), 1);
    let r = &filtered.refs[0];
    assert_eq!(r.target_name, "Handle");
    assert_eq!(r.source_symbol_index, 1);
}

#[test]
fn empty_demand_keeps_everything() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &HashSet::new());
    assert_eq!(filtered.symbols.len(), 5);
    assert_eq!(filtered.refs.len(), 2);
}

#[test]
fn demand_on_member_keeps_only_that_member() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["baz"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["baz"]);
    assert_eq!(filtered.symbols[0].parent_index, None);
    // baz's own ref (`Discard`) survives and remaps to new idx 0.
    assert_eq!(filtered.refs.len(), 1);
    assert_eq!(filtered.refs[0].source_symbol_index, 0);
}

#[test]
fn routes_and_db_sets_drop_when_handler_pruned() {
    let mut result = nested_fixture();
    result.routes.push(ExtractedRoute {
        handler_symbol_index: 4,
        http_method: "GET".to_string(),
        template: "/x".to_string(),
    });
    result.db_sets.push(ExtractedDbSet {
        property_symbol_index: 3,
        entity_type: "Other".to_string(),
        table_name: "others".to_string(),
        source: DbMappingSource::Convention,
    });
    let filtered = filter_extraction_to_demand(result, &demand_of(&["Foo"]));
    assert!(filtered.routes.is_empty());
    assert!(filtered.db_sets.is_empty());
}

#[test]
fn route_index_remaps_when_handler_survives() {
    let mut result = nested_fixture();
    result.routes.push(ExtractedRoute {
        handler_symbol_index: 2,
        http_method: "GET".to_string(),
        template: "/x".to_string(),
    });
    let filtered = filter_extraction_to_demand(result, &demand_of(&["Foo"]));
    assert_eq!(filtered.routes.len(), 1);
    assert_eq!(filtered.routes[0].handler_symbol_index, 1);
}

#[test]
fn multiple_seeds_keep_disjoint_subtrees() {
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["Foo", "Other"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Foo", "bar", "Other", "baz"]);
    assert_eq!(filtered.symbols[1].parent_index, Some(0));
    assert_eq!(filtered.symbols[3].parent_index, Some(2));
}

// ---------------------------------------------------------------------------
// Intra-file type-ref closure tests (layer 2)
//
// Fixture layout: A(0) -TypeRef-> B(1); B(1) -TypeRef-> D(3); C(2) isolated.
// All top-level (no parent). No descendant children.
// ---------------------------------------------------------------------------

fn intra_file_fixture() -> ExtractionResult {
    // A(0): demanded, refs sibling B
    // B(1): not demanded, refs sibling D
    // C(2): unrelated, no refs
    // D(3): not demanded, no refs
    let symbols = vec![
        sym("A", "A", SymbolKind::Struct, None),
        sym("B", "B", SymbolKind::Struct, None),
        sym("C", "C", SymbolKind::Struct, None),
        sym("D", "D", SymbolKind::Struct, None),
    ];
    let refs = vec![
        type_ref(0, "B"), // A references sibling B
        type_ref(1, "D"), // B references sibling D
    ];
    ExtractionResult {
        symbols,
        refs,
        ..Default::default()
    }
}

#[test]
fn intra_file_ref_pulls_sibling_into_keep_set() {
    // A is demanded. A has a TypeRef to B (same file). B must survive even
    // though it is not in the demand set — the file is already_walked after
    // this parse so B can never be re-pulled on a later iteration.
    let filtered = filter_extraction_to_demand(intra_file_fixture(), &demand_of(&["A"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"A"), "A must be kept (seed)");
    assert!(names.contains(&"B"), "B must be kept (intra-file ref from A)");
    assert!(!names.contains(&"C"), "C must be dropped (unreferenced)");
}

#[test]
fn intra_file_closure_is_transitive() {
    // A → B (TypeRef), B → D (TypeRef). Demand {A} must keep {A, B, D}.
    // C is never reached and must drop.
    let filtered = filter_extraction_to_demand(intra_file_fixture(), &demand_of(&["A"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"D"), "D must be kept (transitive: A→B→D)");
    assert!(!names.contains(&"C"), "C must be dropped");
    // A(0→0), B(1→1), D(3→2) — three symbols total (C dropped).
    assert_eq!(filtered.symbols.len(), 3);
}

#[test]
fn intra_file_ref_source_index_remaps_after_closure() {
    // After pulling B into the keep set via intra-file closure, B's own ref
    // to D should survive with source_symbol_index remapped to B's new index.
    // Keep order: A(old 0 → new 0), B(old 1 → new 1), D(old 3 → new 2).
    let filtered = filter_extraction_to_demand(intra_file_fixture(), &demand_of(&["A"]));
    // Two refs survive: A→B (src 0) and B→D (src 1). C was dropped; its refs
    // (none) don't exist. D has no refs.
    assert_eq!(filtered.refs.len(), 2);
    let a_to_b = filtered
        .refs
        .iter()
        .find(|r| r.target_name == "B")
        .expect("A→B ref must survive");
    assert_eq!(a_to_b.source_symbol_index, 0); // A stayed at new-idx 0
    let b_to_d = filtered
        .refs
        .iter()
        .find(|r| r.target_name == "D")
        .expect("B→D ref must survive (B was pulled by intra-file closure)");
    assert_eq!(b_to_d.source_symbol_index, 1); // B stayed at new-idx 1
}

#[test]
fn non_type_dep_refs_do_not_drive_intra_file_closure() {
    // A Calls ref to a sibling should NOT pull that sibling into the keep set —
    // only TypeRef / Inherits / Implements / Instantiates drive the closure.
    let symbols = vec![
        sym("A", "A", SymbolKind::Struct, None),
        sym("B", "B", SymbolKind::Function, None),
    ];
    let refs = vec![ref_of_kind(0, "B", EdgeKind::Calls)];
    let result = ExtractionResult {
        symbols,
        refs,
        ..Default::default()
    };
    let filtered = filter_extraction_to_demand(result, &demand_of(&["A"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["A"]);
    assert!(!names.contains(&"B"), "B must not be pulled by a Calls ref");
}

#[test]
fn inherits_ref_drives_intra_file_closure() {
    // A inherits from B in the same file. Demand {A} must keep B so the
    // inheritance chain survives.
    let symbols = vec![
        sym("A", "A", SymbolKind::Class, None),
        sym("B", "B", SymbolKind::Class, None),
        sym("C", "C", SymbolKind::Class, None),
    ];
    let refs = vec![ref_of_kind(0, "B", EdgeKind::Inherits)];
    let result = ExtractionResult {
        symbols,
        refs,
        ..Default::default()
    };
    let filtered = filter_extraction_to_demand(result, &demand_of(&["A"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"B"), "B must be kept (Inherits from A)");
    assert!(!names.contains(&"C"), "C must be dropped");
}

#[test]
fn external_ref_target_does_not_pollute_intra_file_closure() {
    // bar's TypeRef to external `Handle` must not accidentally keep a same-named
    // local symbol if one happens to exist.  Using the nested_fixture where
    // neither `Handle` nor `Discard` is a symbol in the file — closure stays
    // bounded to real siblings.
    let filtered = filter_extraction_to_demand(nested_fixture(), &demand_of(&["Foo"]));
    let names: Vec<&str> = filtered.symbols.iter().map(|s| s.name.as_str()).collect();
    // Only Foo + bar (descendant) — no extra symbols from the closure.
    assert_eq!(names, vec!["Foo", "bar"]);
}
