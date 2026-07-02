use super::populate_return_type_ids;
use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

fn method_sym(qname: &str, signature: &str, scope_path: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: "new".to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(signature.to_string()),
        doc_comment: None,
        scope_path: scope_path.map(str::to_string),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

// A bare (unqualified) signature return type on a top-level method — no
// enclosing scope, so the name can't be scope-ambiguous — is still eagerly
// interned here.
#[test]
fn top_level_method_gets_eager_return_type() {
    let arena = TypeArena::new();
    let mut result = ExtractionResult {
        symbols: vec![method_sym("Thing.new", "pub fn new() -> Thing", None)],
        ..Default::default()
    };
    populate_return_type_ids(&mut result, &arena, "rust");
    assert!(result.symbols[0].return_type.is_some());
}

// A bare return type on a method nested under a nonempty scope_path is left
// unset here — the name may be scope-ambiguous (a same-named sibling type
// elsewhere), and this pass runs per-file, before the cross-file symbol
// table exists to disambiguate it. The later scope-aware derivation
// (`resolve_type_name_in_scope`, run once the full table is built) fills it
// in instead.
#[test]
fn nested_method_defers_bare_return_type_to_scope_aware_pass() {
    let arena = TypeArena::new();
    let mut result = ExtractionResult {
        symbols: vec![method_sym(
            "selfmod.Thing.new",
            "pub fn new() -> Thing",
            Some("selfmod.Thing"),
        )],
        ..Default::default()
    };
    populate_return_type_ids(&mut result, &arena, "rust");
    assert!(result.symbols[0].return_type.is_none());
}

// A qualified (already-dotted) return type carries no scope ambiguity — it
// names its target explicitly — so it's eagerly interned regardless of the
// method's own scope_path.
#[test]
fn nested_method_with_qualified_return_type_is_eager() {
    let arena = TypeArena::new();
    let mut result = ExtractionResult {
        symbols: vec![method_sym(
            "selfmod.Thing.new",
            "pub fn new() -> selfmod::Thing",
            Some("selfmod.Thing"),
        )],
        ..Default::default()
    };
    populate_return_type_ids(&mut result, &arena, "rust");
    assert!(result.symbols[0].return_type.is_some());
}
