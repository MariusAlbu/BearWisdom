// Ada resolution runs through the generic DefaultResolver (ADA_PROFILE data) and
// — for receiver chains — the generic chain walker. The body→spec companion
// helper is unit-tested below; the receiver-chain path is exercised end-to-end
// through the real extractor + SymbolIndex + Engine.

use super::extract::extract;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex,
};
use crate::type_checker::core::SymbolIdMap;
use crate::type_checker::Engine;
use crate::types::*;
use std::collections::HashMap;

use super::hooks::spec_for_body;

#[test]
fn spec_for_body_returns_ads_for_adb() {
    assert_eq!(
        spec_for_body("src/bmp280.adb"),
        Some("src/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_handles_unix_path() {
    assert_eq!(
        spec_for_body("drivers/sensors/bmp280.adb"),
        Some("drivers/sensors/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_handles_windows_separators() {
    assert_eq!(
        spec_for_body("drivers\\sensors\\bmp280.adb"),
        Some("drivers/sensors/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_returns_none_for_ads() {
    assert_eq!(spec_for_body("src/bmp280.ads"), None);
}

#[test]
fn spec_for_body_returns_none_for_other_extension() {
    assert_eq!(spec_for_body("src/main.rs"), None);
    assert_eq!(spec_for_body("src/foo.py"), None);
}

#[test]
fn spec_for_body_bare_filename() {
    assert_eq!(spec_for_body("bmp280.adb"), Some("bmp280.ads".to_string()));
}

// ---------------------------------------------------------------------------
// Receiver field-chain resolution, end-to-end through the generic engine.
// ---------------------------------------------------------------------------

const RECEIVER_CHAIN_SRC: &str = concat!(
    "package body Timers is\n",
    "   type Port_Type is record\n",
    "      CCER : Integer;\n",
    "   end record;\n",
    "   type Timer is record\n",
    "      Port : Port_Type;\n",
    "   end record;\n",
    "   procedure Setup (This : Timer; Ch : Integer) is\n",
    "   begin\n",
    "      This.Port.CCER (Ch);\n",
    "   end Setup;\n",
    "end Timers;\n"
);

fn ada_parsed_file(path: &str, src: &str) -> ParsedFile {
    let ex = extract(src);
    ParsedFile {
        path: path.to_string(),
        language: "ada".to_string(),
        content_hash: String::new(),
        size: src.len() as u64,
        line_count: src.lines().count() as u32,
        mtime: None,
        package_id: None,
        content: Some(src.to_string()),
        has_errors: ex.has_errors,
        symbols: ex.symbols,
        refs: ex.refs,
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

/// Build a real `SymbolIndex` + `Engine` over the files and resolve the first
/// chain-bearing call in `importer`. Returns the resolution alongside the
/// `(path, qname) -> id` map so the test can name the expected target.
fn resolve_first_chain_call(
    files: &[ParsedFile],
    importer: usize,
) -> (Option<Resolution>, HashMap<(String, String), i64>) {
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next = 1i64;
    for pf in files {
        for s in &pf.symbols {
            id_map.insert((pf.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    let index = SymbolIndex::build(files, &id_map);
    // The engine keys its type/member maps by (path, idx); reuse the same ids.
    let mut eng_ids = SymbolIdMap::default();
    for pf in files {
        for (i, s) in pf.symbols.iter().enumerate() {
            if let Some(&id) = id_map.get(&(pf.path.clone(), s.qualified_name.clone())) {
                eng_ids.insert((pf.path.clone(), i), id);
            }
        }
    }
    let engine = Engine::build_from_registry(files, &eng_ids, &index, index.type_arena_arc());
    let pf = &files[importer];
    let r = pf
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.chain.is_some())
        .expect("a chain-bearing call ref");
    let source = &pf.symbols[r.source_symbol_index];
    let file_ctx = FileContext {
        file_path: pf.path.clone(),
        language: "ada".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: source,
        scope_chain: build_scope_chain(source.scope_path.as_deref()),
        file_package_id: None,
    };
    (engine.resolve(&ref_ctx, &file_ctx, &index), id_map)
}

/// `This.Port.CCER (Ch)` — root `This` types to `Timer` (param TypeRef), the
/// `Port` hop types to `Port_Type` (field TypeRef), and the leaf binds the
/// `CCER` component. Proves the three coordinated changes (field/param TypeRef
/// emission, structured receiver-chain emission, the `Field` mid-chain kind)
/// resolve a dotted field chain entirely through the generic walker.
#[test]
fn receiver_field_chain_resolves_through_generic_walker() {
    let files = vec![ada_parsed_file("hw/timers.adb", RECEIVER_CHAIN_SRC)];
    let (res, id_map) = resolve_first_chain_call(&files, 0);
    let res = res.expect("This.Port.CCER should resolve through the chain walker");
    let ccer = id_map[&(
        "hw/timers.adb".to_string(),
        "Timers.Port_Type.CCER".to_string(),
    )];
    assert_eq!(
        res.target_symbol_id, ccer,
        "expected the chain to bind the CCER component"
    );
}
