use super::{prefix_ts_external_symbols, ts_post_process_external};
use crate::type_checker::core::types::{Type, TypeArena};
use crate::types::{ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind};

fn sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.into(),
        qualified_name: name.into(),
        kind,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn pf_with(symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: "ext:ts:fake-ui/index.d.ts".into(),
        language: "typescript".into(),
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
    }
}

#[test]
fn variable_declared_type_requalified_with_package_prefix() {
    let arena = TypeArena::new();
    let mut v = sym("gadget", SymbolKind::Variable);
    v.declared_type = Some(arena.class("ApiKind"));
    let mut pf = pf_with(vec![v]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    let id = pf.symbols[0]
        .declared_type
        .expect("variable declared_type survives prefixing");
    assert!(
        matches!(arena.get(id), Type::Class(n) if n == "fake-ui.ApiKind"),
        "declared type head must carry the package prefix"
    );
}

#[test]
fn variable_declared_type_apply_head_requalified_args_untouched() {
    let arena = TypeArena::new();
    let base = arena.class("Registry");
    let arg = arena.class("Cfg");
    let applied = arena.intern(Type::Apply { base, args: vec![arg] });
    let mut v = sym("q", SymbolKind::Variable);
    v.declared_type = Some(applied);
    let mut pf = pf_with(vec![v]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    let id = pf.symbols[0].declared_type.expect("apply survives");
    let Type::Apply { base, args } = arena.get(id) else {
        panic!("expected Apply, got {:?}", arena.get(id));
    };
    assert!(matches!(arena.get(base), Type::Class(n) if n == "fake-ui.Registry"));
    assert_eq!(args, vec![arg], "type args pass through unqualified");
}

#[test]
fn already_prefixed_declared_type_passes_through() {
    let arena = TypeArena::new();
    let pre = arena.class("fake-ui.ApiKind");
    let mut v = sym("gadget", SymbolKind::Variable);
    v.declared_type = Some(pre);
    let mut pf = pf_with(vec![v]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    assert_eq!(pf.symbols[0].declared_type, Some(pre));
}

#[test]
fn non_variable_type_fields_still_cleared() {
    let arena = TypeArena::new();
    let mut prop = sym("count", SymbolKind::Property);
    prop.declared_type = Some(arena.class("Counter"));
    let mut method = sym("build", SymbolKind::Method);
    method.return_type = Some(arena.class("Widget"));
    method.param_types = vec![arena.class("Opts")];
    let mut pf = pf_with(vec![prop, method]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    assert_eq!(pf.symbols[0].declared_type, None);
    assert_eq!(pf.symbols[1].return_type, None);
    assert!(pf.symbols[1].param_types.is_empty());
}

#[test]
fn variable_non_nominal_declared_type_dropped() {
    // A shape with no requalifiable head (a bare tuple) still drops — Phase B
    // re-derives it from refs.
    let arena = TypeArena::new();
    let tup = arena.intern(Type::Tuple(vec![arena.class("A"), arena.class("B")]));
    let mut v = sym("pair", SymbolKind::Variable);
    v.declared_type = Some(tup);
    let mut pf = pf_with(vec![v]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    assert_eq!(pf.symbols[0].declared_type, None);
}

#[test]
fn post_process_requalifies_through_package_detection() {
    let arena = TypeArena::new();
    let mut v = sym("gadget", SymbolKind::Variable);
    v.declared_type = Some(arena.class("ApiKind"));
    let mut pf = pf_with(vec![v]);
    ts_post_process_external(&mut pf, &arena);
    assert_eq!(pf.symbols[0].qualified_name, "fake-ui.gadget");
    let id = pf.symbols[0].declared_type.expect("declared_type survives");
    assert!(matches!(arena.get(id), Type::Class(n) if n == "fake-ui.ApiKind"));
}

#[test]
fn function_typed_variable_keeps_its_shape_with_requalified_return() {
    // `declare const make: (opts: Opts) => Client<Cfg>` — the function shape
    // must survive prefixing (a call root peels through it to the return);
    // the return's head is a name in the package's own surface.
    let arena = TypeArena::new();
    let param = arena.class("Opts");
    let ret_base = arena.class("Client");
    let ret_arg = arena.class("Cfg");
    let ret = arena.intern(Type::Apply { base: ret_base, args: vec![ret_arg] });
    let f = arena.intern(Type::Function { params: vec![param], return_: ret });
    let mut v = sym("make", SymbolKind::Variable);
    v.declared_type = Some(f);
    let mut pf = pf_with(vec![v]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    let id = pf.symbols[0].declared_type.expect("function type survives prefixing");
    let Type::Function { params, return_ } = arena.get(id) else {
        panic!("expected Function, got {:?}", arena.get(id));
    };
    assert_eq!(params, vec![param], "param types pass through unqualified");
    let Type::Apply { base, .. } = arena.get(return_) else {
        panic!("expected Apply return, got {:?}", arena.get(return_));
    };
    assert!(matches!(arena.get(base), Type::Class(n) if n == "fake-ui.Client"));
}

#[test]
fn function_typed_variable_with_primitive_return_passes_through() {
    let arena = TypeArena::new();
    let ret = arena.primitive(crate::type_checker::core::types::PrimKind::Bool);
    let f = arena.intern(Type::Function { params: Vec::new(), return_: ret });
    let mut v = sym("flag", SymbolKind::Variable);
    v.declared_type = Some(f);
    let mut pf = pf_with(vec![v]);
    prefix_ts_external_symbols(&mut pf, "fake-ui", &arena);
    assert_eq!(pf.symbols[0].declared_type, Some(f), "unrequalifiable return keeps the annotation as written");
}
