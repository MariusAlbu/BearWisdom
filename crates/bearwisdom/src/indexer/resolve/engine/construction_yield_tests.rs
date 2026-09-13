// =============================================================================
// engine/construction_yield_tests — the construction yield and its two slots
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::{constructed_declaration, declaration_yield, record_return_type};
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::{SymbolLookup, TypeInfo};
use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility};

fn sym(name: &str, qname: &str, kind: SymbolKind, parent_index: Option<usize>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// `class Foo { Foo(); }` — index 0 is the class, index 1 its constructor.
fn class_with_constructor() -> Vec<ExtractedSymbol> {
    vec![
        sym("Foo", "Foo", SymbolKind::Class, None),
        sym("Foo", "Foo.Foo", SymbolKind::Constructor, Some(0)),
    ]
}

#[test]
fn constructor_yields_its_declaring_declaration() {
    let symbols = class_with_constructor();
    let decl = constructed_declaration(&symbols[1], &symbols)
        .expect("a constructor under a class names that class");
    assert_eq!(decl.qualified_name, "Foo");
    assert_eq!(decl.kind, SymbolKind::Class);

    let arena = TypeArena::new();
    assert_eq!(
        declaration_yield(&arena, decl),
        arena.class("Foo"),
        "the yield is the declaration's type, not the constructor's"
    );
}

#[test]
fn type_declaration_is_its_own_yield() {
    let symbols = class_with_constructor();
    let decl = constructed_declaration(&symbols[0], &symbols)
        .expect("a type declaration constructs itself");
    assert_eq!(decl.qualified_name, "Foo");
}

#[test]
fn parentless_constructor_declines() {
    let symbols = vec![sym("Foo", "Foo.Foo", SymbolKind::Constructor, None)];
    assert!(
        constructed_declaration(&symbols[0], &symbols).is_none(),
        "no structural parent means no declaration — the qname is not truncated to invent one"
    );
}

#[test]
fn constructor_under_a_non_type_parent_declines() {
    let symbols = vec![
        sym("outer", "outer", SymbolKind::Function, None),
        sym("Foo", "outer.Foo", SymbolKind::Constructor, Some(0)),
    ];
    assert!(
        constructed_declaration(&symbols[1], &symbols).is_none(),
        "a parent that declares no member set cannot be the constructed declaration"
    );
}

#[test]
fn non_constructor_declines() {
    let symbols = vec![
        sym("Foo", "Foo", SymbolKind::Class, None),
        sym("bar", "Foo.bar", SymbolKind::Method, Some(0)),
    ];
    assert!(
        constructed_declaration(&symbols[1], &symbols).is_none(),
        "methods keep their signature-inferred return"
    );
}

#[test]
fn record_return_type_is_first_writer_wins() {
    let arena = TypeArena::new();
    let extractor_set = arena.class("Extractor");
    let derived = arena.class("Foo");

    let mut by_qname: FxHashMap<String, TypeInfo> = FxHashMap::default();
    let mut by_id: FxHashMap<i64, TypeInfo> = FxHashMap::default();
    by_qname.entry("Foo.Foo".to_string()).or_default().return_type_id = Some(extractor_set);
    by_id.entry(7).or_default().return_type_id = Some(extractor_set);

    record_return_type("Foo.Foo", Some(7), derived, &mut by_qname, &mut by_id);

    assert_eq!(by_qname["Foo.Foo"].return_type_id, Some(extractor_set));
    assert_eq!(by_id[&7].return_type_id, Some(extractor_set));
}

#[test]
fn record_return_type_writes_both_slots() {
    let arena = TypeArena::new();
    let foo = arena.class("Foo");

    let mut by_qname: FxHashMap<String, TypeInfo> = FxHashMap::default();
    let mut by_id: FxHashMap<i64, TypeInfo> = FxHashMap::default();
    record_return_type("Foo.Foo", Some(7), foo, &mut by_qname, &mut by_id);
    assert_eq!(by_qname["Foo.Foo"].return_type_id, Some(foo));
    assert_eq!(by_id[&7].return_type_id, Some(foo));

    let mut only_qname: FxHashMap<String, TypeInfo> = FxHashMap::default();
    let mut untouched: FxHashMap<i64, TypeInfo> = FxHashMap::default();
    record_return_type("Bar.Bar", None, foo, &mut only_qname, &mut untouched);
    assert_eq!(only_qname["Bar.Bar"].return_type_id, Some(foo));
    assert!(untouched.is_empty(), "an id-less symbol writes no id slot");
}

// ---------------------------------------------------------------------------
// The yield as the built `Compilation` exposes it — both return slots.
// ---------------------------------------------------------------------------

/// `class Foo { Foo(); bar(): User }` as one parsed file, indexed through the
/// real `Compilation::build`. Returns `(compilation, arena)`.
fn build_class_with_constructor() -> (Compilation, Arc<TypeArena>) {
    let arena = Arc::new(TypeArena::new());

    let mut symbols = class_with_constructor();
    let mut bar = sym("bar", "Foo.bar", SymbolKind::Method, Some(0));
    bar.signature = Some("bar(): User".to_string());
    symbols.push(bar);

    let pf = ParsedFile {
        path: "src/foo.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    };

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("src/foo.ts".to_string(), "Foo".to_string()), 1);
    id_map.insert(("src/foo.ts".to_string(), "Foo.Foo".to_string()), 2);
    id_map.insert(("src/foo.ts".to_string(), "Foo.bar".to_string()), 3);

    let tree = Compilation::build(&[pf], &id_map.into(), Arc::clone(&arena));
    (tree, arena)
}

#[test]
fn constructor_symbol_returns_its_class_type() {
    let (tree, arena) = build_class_with_constructor();
    let ctor = tree
        .by_qualified_name("Foo.Foo")
        .expect("the constructor is indexed")
        .id;

    assert_eq!(
        tree.return_type_id_of(ctor),
        Some(arena.class("Foo")),
        "a constructor with no signature return still yields its declaring class"
    );
    assert_eq!(
        tree.return_type_id("Foo.Foo").map(|id| arena.format_type(id)),
        Some("Foo".to_string()),
        "the qname-keyed slot carries the same yield for name-only readers"
    );
}

#[test]
fn method_return_inference_is_unchanged_by_the_constructor_split() {
    let (tree, arena) = build_class_with_constructor();
    assert_eq!(
        tree.return_type_id("Foo.bar").map(|id| arena.format_type(id)),
        Some("User".to_string()),
        "splitting Constructor out of the inference arm must leave methods inferring"
    );
}
