use super::hooks::{detect_lua_db_emission, detect_lua_lapis_route, detect_lua_resty_http};
use crate::indexer::resolve::engine::{infer_external_from_chain, SymbolIndex};
use crate::types::*;
use std::collections::HashMap;

#[test]
fn test_lua_lapis_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_lua_lapis_route("", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_lua_lapis_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_lapis_route("", "post", &args).is_some());
}

#[test]
fn test_lua_lapis_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_lapis_route("", "middleware", &args).is_none());
}

#[test]
fn test_lua_resty_http_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_lua_resty_http("resty.http", "request_uri", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_lua_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_resty_http("string", "request", &args).is_none());
}

#[test]
fn test_lua_pgmoon_query_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_lua_db_emission("pgmoon", "query").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_lua_db_rejects_non_db_module() {
    assert!(detect_lua_db_emission("io", "query").is_none());
}

// ---------------------------------------------------------------------------
// Colon-call receiver chain: `s:gsub(...)` carries a 2-segment receiver chain
// `[s, gsub]`, so a string-typed receiver roots the leaf to the external
// `string` library (Lua's `string.gsub`). Before the fix the extractor emitted
// a bare `gsub` with chain None, dropping the receiver entirely.
// ---------------------------------------------------------------------------

#[test]
fn test_lua_colon_call_binds_gsub_to_external_string() {
    let src = "function clean(s) return s:gsub('%s+', ' ') end";
    let result = super::extract::extract(src);

    let gsub = result
        .refs
        .iter()
        .find(|r| r.target_name == "gsub" && r.kind == EdgeKind::Calls)
        .expect("expected a Calls ref for the gsub method call");

    // The receiver must survive as a 2-segment chain, not be dropped to None.
    let chain = gsub
        .chain
        .as_ref()
        .expect("s:gsub() must carry a receiver chain, not chain None");
    let names: Vec<&str> = chain.segments.iter().map(|seg| seg.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["s", "gsub"],
        "expected 2-segment receiver chain [s, gsub]"
    );

    // With the receiver typed `string` (post flow-inference), the chain roots to
    // the external `string` library — Lua's `string.gsub` builtin — instead of
    // landing as an unresolved bare call.
    let mut typed = chain.clone();
    typed.segments[0].declared_type = Some("string".to_string());
    let index = SymbolIndex::build(&[], &HashMap::new());
    let ns = infer_external_from_chain(&typed, &[], &index)
        .expect("string receiver should classify the chain as external");
    assert!(
        ns.contains("string"),
        "expected the chain to root to the external `string` library; got {ns:?}"
    );
}

// ---------------------------------------------------------------------------
// Stdlib ambient binding + value-alias binding (Track F / L)
//
// The Lua stdlib is indexed at `ext:lua-stdlib:` (base globals keyed by bare
// qname, e.g. `print`; other modules keyed `<module>.<name>`, e.g.
// `math.floor`). Two engine levers admit these:
//   * value alias — `local floor = math.floor; floor(x)` binds the bare call
//     to `math.floor` via `LuaHooks::resolve_bare_pre`.
//   * ambient global — a bare base global (`print()`) binds to the stdlib
//     symbol via the `ambient_globals` rung, last in the ladder.
// ---------------------------------------------------------------------------

use super::hooks::{parse_value_alias, LuaHooks};
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, RefContext, SymbolIndex as Index,
};
use crate::type_checker::core::SymbolIdMap;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::type_checker::Engine;

fn lua_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "lua".to_string(),
        content_hash: "x".to_string(),
        size: 100,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn fn_sym(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 3,
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

/// A stdlib symbol as `lua_stdlib.rs` synthesizes it: a Function under the
/// `ext:lua-stdlib:` synthetic file, keyed by the (possibly dotted) qname.
fn stdlib_sym(name: &str, qname: &str) -> ExtractedSymbol {
    fn_sym(name, qname)
}

/// A `local <name> = <table>.<member>` value-alias variable: the dotted RHS
/// rides the signature exactly as the extractor emits it.
fn alias_var(name: &str, target_qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        kind: SymbolKind::Variable,
        signature: Some(format!("{} = {}", name, target_qname)),
        ..fn_sym(name, name)
    }
}

const STDLIB_PATH: &str = "ext:lua-stdlib:lua_stdlib_generated.lua";

fn build_id_map(files: &[ParsedFile]) -> HashMap<(String, String), i64> {
    let mut id_map = HashMap::new();
    let mut next = 1i64;
    for p in files {
        for s in &p.symbols {
            id_map.insert((p.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    id_map
}

fn engine_for(files: &[ParsedFile], id_map: &HashMap<(String, String), i64>) -> (Index, Engine<'static>) {
    let index = Index::build(files, id_map);
    let mut eng = SymbolIdMap::default();
    for p in files {
        for (i, s) in p.symbols.iter().enumerate() {
            if let Some(&id) = id_map.get(&(p.path.clone(), s.qualified_name.clone())) {
                eng.insert((p.path.clone(), i), id);
            }
        }
    }
    let engine = Engine::build_from_registry(files, &eng, &index, index.type_arena_arc());
    (index, engine)
}

fn calls(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
    }
}

/// Resolve `call_ref` issued from `files[file_idx].symbols[src_sym_idx]`.
fn resolve_call(
    files: &[ParsedFile],
    file_idx: usize,
    src_sym_idx: usize,
    call_ref: &ExtractedRef,
) -> Option<i64> {
    let id_map = build_id_map(files);
    let (index, engine) = engine_for(files, &id_map);
    let pf = &files[file_idx];
    let fc = LuaHooks
        .build_file_context(pf, None)
        .expect("lua build_file_context");
    let src = &pf.symbols[src_sym_idx];
    let rc = RefContext {
        extracted_ref: call_ref,
        source_symbol: src,
        scope_chain: build_scope_chain(src.scope_path.as_deref()),
        file_package_id: None,
    };
    engine.resolve(&rc, &fc, &index).map(|r| r.target_symbol_id)
}

// (a) `local floor = math.floor; floor(x)` binds to `math.floor`.
#[test]
fn value_alias_binds_bare_call_to_qualified_stdlib() {
    let stdlib = lua_file(STDLIB_PATH, vec![stdlib_sym("floor", "math.floor")], vec![]);
    let app = lua_file(
        "app.lua",
        vec![alias_var("floor", "math.floor"), fn_sym("use", "use")],
        vec![],
    );
    let files = vec![stdlib, app];
    let id_map = build_id_map(&files);
    let floor_id = id_map[&(STDLIB_PATH.to_string(), "math.floor".to_string())];

    let got = resolve_call(&files, 1, 1, &calls("floor"));
    assert_eq!(
        got,
        Some(floor_id),
        "bare `floor()` with `local floor = math.floor` must bind to math.floor"
    );
}

// (b) direct `math.floor(x)` qualified call still binds (regression).
#[test]
fn qualified_stdlib_call_still_binds() {
    let stdlib = lua_file(STDLIB_PATH, vec![stdlib_sym("floor", "math.floor")], vec![]);
    let app = lua_file("app.lua", vec![fn_sym("use", "use")], vec![]);
    let files = vec![stdlib, app];
    let id_map = build_id_map(&files);
    let floor_id = id_map[&(STDLIB_PATH.to_string(), "math.floor".to_string())];

    let got = resolve_call(&files, 1, 0, &calls("math.floor"));
    assert_eq!(
        got,
        Some(floor_id),
        "qualified `math.floor()` must resolve by exact qname"
    );
}

// Base-module global (`print`) binds via the ambient rung (bare qname in the
// stdlib pool), the companion to the alias path.
#[test]
fn base_global_binds_via_ambient_rung() {
    let stdlib = lua_file(STDLIB_PATH, vec![stdlib_sym("print", "print")], vec![]);
    let app = lua_file("app.lua", vec![fn_sym("use", "use")], vec![]);
    let files = vec![stdlib, app];
    let id_map = build_id_map(&files);
    let print_id = id_map[&(STDLIB_PATH.to_string(), "print".to_string())];

    let got = resolve_call(&files, 1, 0, &calls("print"));
    assert_eq!(
        got,
        Some(print_id),
        "bare `print()` must bind to the ambient base global"
    );
}

// (c) `widget:close()` (receiver-typed colon call) binds the internal method,
// NOT the stdlib `io.close`. The colon call carries a 2-segment receiver chain
// (`[widget, close]`) with the receiver typed `Widget`, so it routes through
// the chain walker and never reaches the bare/ambient path — mirroring the
// `s:gsub()` external-string test, but with an internal receiver type.
#[test]
fn receiver_typed_close_does_not_bind_stdlib() {
    let stdlib = lua_file(STDLIB_PATH, vec![stdlib_sym("close", "io.close")], vec![]);

    let recv = ChainSegment {
        name: "widget".to_string(),
        node_kind: "identifier".to_string(),
        kind: SegmentKind::Identifier,
        declared_type: Some("Widget".to_string()),
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        type_arg_ids: Vec::new(),
        is_call: false,
        call_args: Vec::new(),
    };
    let leaf = ChainSegment {
        name: "close".to_string(),
        node_kind: "method".to_string(),
        kind: SegmentKind::Property,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        type_arg_ids: Vec::new(),
        is_call: true,
        call_args: Vec::new(),
    };
    let mut close_call = calls("close");
    close_call.chain = Some(MemberChain {
        segments: vec![recv, leaf],
    });

    let app = lua_file(
        "widget.lua",
        vec![
            ExtractedSymbol {
                kind: SymbolKind::Class,
                ..fn_sym("Widget", "Widget")
            },
            ExtractedSymbol {
                kind: SymbolKind::Method,
                scope_path: Some("Widget".to_string()),
                parent_index: Some(0),
                ..fn_sym("close", "Widget.close")
            },
            ExtractedSymbol {
                kind: SymbolKind::Function,
                ..fn_sym("run", "run")
            },
        ],
        vec![close_call.clone()],
    );
    let files = vec![stdlib, app];
    let id_map = build_id_map(&files);
    let internal_close = id_map[&("widget.lua".to_string(), "Widget.close".to_string())];
    let stdlib_close = id_map[&(STDLIB_PATH.to_string(), "io.close".to_string())];

    // Source the call from `run` (index 2).
    let got = resolve_call(&files, 1, 2, &close_call);
    assert_ne!(
        got,
        Some(stdlib_close),
        "receiver-typed widget:close() must NOT bind the stdlib io.close"
    );
    assert_eq!(
        got,
        Some(internal_close),
        "receiver-typed widget:close() binds the internal Widget.close method"
    );
}

// (d) busted DSL names (`it`, `describe`) and a dynamic-self method (`setDirty`)
// with no internal candidate and no stdlib entry stay unresolved — the ambient
// rung must not invent a binding for a name absent from the stdlib pool.
#[test]
fn unknown_bare_names_do_not_spuriously_bind() {
    // Stdlib present but does NOT contain these names.
    let stdlib = lua_file(
        STDLIB_PATH,
        vec![stdlib_sym("floor", "math.floor"), stdlib_sym("print", "print")],
        vec![],
    );
    let app = lua_file("spec.lua", vec![fn_sym("use", "use")], vec![]);
    let files = vec![stdlib, app];

    for name in ["it", "describe", "setDirty"] {
        let got = resolve_call(&files, 1, 0, &calls(name));
        assert_eq!(
            got, None,
            "bare `{name}()` has no internal/stdlib candidate and must stay unresolved"
        );
    }
}

// (e) a bare `close()` (no receiver, no alias) with ambiguous internal
// candidates across files AND a stdlib `io.close` stays unresolved — declining
// over guessing, since `io.close` is keyed by its dotted qname (not bare
// `close`) and the internal candidates are ambiguous.
#[test]
fn ambiguous_bare_close_stays_unresolved() {
    let stdlib = lua_file(STDLIB_PATH, vec![stdlib_sym("close", "io.close")], vec![]);
    // Two different files each define a bare `close` function → ambiguous.
    let a = lua_file("a.lua", vec![fn_sym("close", "close")], vec![]);
    let b = lua_file("b.lua", vec![fn_sym("close", "close")], vec![]);
    // The caller lives in a third file with no local `close`.
    let caller = lua_file("c.lua", vec![fn_sym("run", "run")], vec![]);
    let files = vec![stdlib, a, b, caller];

    let got = resolve_call(&files, 3, 0, &calls("close"));
    assert_eq!(
        got, None,
        "bare close() with ambiguous internals + dotted-qname stdlib must stay unresolved"
    );
}

// ---------------------------------------------------------------------------
// parse_value_alias — signature decoding
// ---------------------------------------------------------------------------

#[test]
fn parse_value_alias_accepts_dotted_member() {
    assert_eq!(
        parse_value_alias("floor = math.floor"),
        Some(("floor", "math.floor"))
    );
    assert_eq!(
        parse_value_alias("insert = table.insert"),
        Some(("insert", "table.insert"))
    );
}

#[test]
fn parse_value_alias_rejects_non_alias_signatures() {
    // table literal
    assert_eq!(parse_value_alias("M = {}"), None);
    // function definition signature
    assert_eq!(parse_value_alias("function(a, b)"), None);
    // bare (non-dotted) RHS — not a qualified member
    assert_eq!(parse_value_alias("x = y"), None);
    // call-result RHS
    assert_eq!(parse_value_alias("db = require.load()"), None);
}

// ---------------------------------------------------------------------------
// Extractor emits the value-alias signature for `local NAME = TABLE.MEMBER`.
// ---------------------------------------------------------------------------

#[test]
fn extractor_emits_value_alias_signature() {
    let src = "local floor = math.floor\nlocal function use() return floor(1.5) end";
    let result = super::extract::extract(src);
    let floor = result
        .symbols
        .iter()
        .find(|s| s.name == "floor" && s.kind == SymbolKind::Variable)
        .expect("local floor variable");
    let sig = floor.signature.as_deref().expect("value-alias signature");
    assert_eq!(
        parse_value_alias(sig),
        Some(("floor", "math.floor")),
        "extractor signature must decode to the (local, qname) alias pair; got {sig:?}"
    );
}
