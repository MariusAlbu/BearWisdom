// Ada resolution runs through the generic DefaultResolver (ADA_PROFILE data) and
// — for receiver chains — the generic chain walker. The body→spec companion
// helper is unit-tested below; the receiver-chain path is exercised end-to-end
// through the real extractor + SymbolIndex + Engine.

use super::extract::extract;
use crate::indexer::resolve::legacy::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex,
};
use crate::type_checker::core::SymbolIdMap;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::type_checker::Engine;
use crate::types::*;
use std::collections::HashMap;

use super::hooks::{spec_for_body, AdaHooks};

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

// ---------------------------------------------------------------------------
// use'd-package bare member resolution, end-to-end through the generic engine.
// `use Pkg;` brings Pkg's members into bare scope; a bare `Member` call must
// bind to `Pkg.Member`. The ada hook turns the `use` clause into a wildcard
// import, so the candidate qualifies under the wildcard rung.
// ---------------------------------------------------------------------------

/// Build a real `SymbolIndex` + `Engine` over `files` and resolve the bare
/// (chain-less) `Calls` ref named `target` in `caller`, with the file's
/// `use`/`with` clauses turned into FileContext imports by the ada hook.
fn resolve_bare_call(files: &[ParsedFile], caller: usize, target: &str) -> Option<Resolution> {
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next = 1i64;
    for pf in files {
        for s in &pf.symbols {
            id_map.insert((pf.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    let index = SymbolIndex::build(files, &id_map);
    let mut eng_ids = SymbolIdMap::default();
    for pf in files {
        for (i, s) in pf.symbols.iter().enumerate() {
            if let Some(&id) = id_map.get(&(pf.path.clone(), s.qualified_name.clone())) {
                eng_ids.insert((pf.path.clone(), i), id);
            }
        }
    }
    let engine = Engine::build_from_registry(files, &eng_ids, &index, index.type_arena_arc());
    let pf = &files[caller];
    let r = pf
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.chain.is_none() && r.target_name == target)
        .expect("the bare call ref");
    let source = &pf.symbols[r.source_symbol_index];
    let file_ctx = AdaHooks.build_file_context(pf, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: source,
        scope_chain: build_scope_chain(source.scope_path.as_deref()),
        file_package_id: None,
    };
    engine.resolve(&ref_ctx, &file_ctx, &index)
}

const USE_MEMBER_PKG_SRC: &str = concat!(
    "package Pkg is\n",
    "   procedure Member;\n",
    "end Pkg;\n"
);

const USE_MEMBER_CLIENT_SRC: &str = concat!(
    "with Pkg;\n",
    "use Pkg;\n",
    "procedure Client is\n",
    "begin\n",
    "   Member;\n",
    "end Client;\n"
);

#[test]
fn used_package_bare_member_resolves() {
    let files = vec![
        ada_parsed_file("pkg.ads", USE_MEMBER_PKG_SRC),
        ada_parsed_file("client.adb", USE_MEMBER_CLIENT_SRC),
    ];
    let res = resolve_bare_call(&files, 1, "Member")
        .expect("bare `Member` after `use Pkg;` must bind to Pkg.Member");
    // Re-derive the expected id the same way the harness assigns them.
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next = 1i64;
    for pf in &files {
        for s in &pf.symbols {
            id_map.insert((pf.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    let member = id_map[&("pkg.ads".to_string(), "Pkg.Member".to_string())];
    assert_eq!(res.target_symbol_id, member);
}

#[test]
fn unused_package_bare_member_with_no_definition_declines() {
    // `Nonexistent` is not declared in any project file; the bare call must
    // stay unresolved so external classification can brand it.
    let client_src = concat!(
        "with Pkg;\n",
        "use Pkg;\n",
        "procedure Client is\n",
        "begin\n",
        "   Nonexistent;\n",
        "end Client;\n"
    );
    let files = vec![
        ada_parsed_file("pkg.ads", USE_MEMBER_PKG_SRC),
        ada_parsed_file("client.adb", client_src),
    ];
    assert!(
        resolve_bare_call(&files, 1, "Nonexistent").is_none(),
        "a bare name with no project definition must not bind"
    );
}
