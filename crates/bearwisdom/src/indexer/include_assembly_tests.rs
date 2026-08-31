// =============================================================================
// indexer/include_assembly_tests.rs — textual-include splice pre-pass
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;

use super::assemble_includes;
use crate::db::Database;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::indexer::write::SymbolIdMap;
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
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

fn include_ref(stem: &str) -> ExtractedRef {
    ExtractedRef {
        is_include: true,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: stem.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(stem.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_parsed_file(
    path: &str,
    language: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        declared_modules: Vec::new(),
        symbols,
        refs,
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

/// A unit file declaring namespace `ns` plus one include ref per stem.
fn unit_file(path: &str, ns: &str, stems: &[&str]) -> ParsedFile {
    make_parsed_file(
        path,
        "pascal",
        vec![make_symbol(ns, ns, SymbolKind::Namespace, None)],
        stems.iter().map(|s| include_ref(s)).collect(),
    )
}

/// A fragment file with a bare top-level function `Helper`, a bare top-level
/// class `TThing`, and a nested method `TThing.Do`.
fn fragment_file(path: &str) -> ParsedFile {
    make_parsed_file(
        path,
        "pascal",
        vec![
            make_symbol("Helper", "Helper", SymbolKind::Function, None),
            make_symbol("TThing", "TThing", SymbolKind::Class, None),
            make_symbol("Do", "TThing.Do", SymbolKind::Method, Some(1)),
        ],
        Vec::new(),
    )
}

// ---------------------------------------------------------------------------
// Splice + re-parent
// ---------------------------------------------------------------------------

#[test]
fn include_reparents_fragment_symbols_and_rekeys_id_map() {
    let db = Database::open_in_memory().expect("in-memory db");
    db.conn()
        .execute_batch(
            "INSERT INTO files (id, path, hash, language, last_indexed)
             VALUES (1, 'src/helpers.inc', '', 'pascal', 0);
             INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, col, symbol_key)
             VALUES (2, 1, 'Helper', 'Helper', 'function', 0, 0, '1:Helper#function#0'),
                    (3, 1, 'TThing', 'TThing', 'class', 0, 0, '1:TThing#class#0'),
                    (4, 1, 'Do', 'TThing.Do', 'method', 0, 0, '1:TThing.Do#method#0');",
        )
        .expect("seed rows");

    let mut parsed = vec![
        unit_file("src/myunit.pas", "MyUnit", &["helpers"]),
        fragment_file("src/helpers.inc"),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();
    id_map.insert(("src/myunit.pas".into(), "MyUnit".into()), 1);
    id_map.insert(("src/helpers.inc".into(), "Helper".into()), 2);
    id_map.insert(("src/helpers.inc".into(), "TThing".into()), 3);
    id_map.insert(("src/helpers.inc".into(), "TThing.Do".into()), 4);

    let n = assemble_includes(&db, &mut parsed, &mut id_map).expect("pass runs");
    assert_eq!(n, 3, "all three fragment symbols re-parent");

    let frag = &parsed[1];
    let qnames: Vec<&str> = frag.symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    assert_eq!(qnames, ["MyUnit.Helper", "MyUnit.TThing", "MyUnit.TThing.Do"]);
    assert_eq!(frag.symbols[0].scope_path.as_deref(), Some("MyUnit"));
    assert_eq!(frag.symbols[2].scope_path.as_deref(), None);

    // Id map rekeyed under the new qnames; the old keys are gone.
    assert_eq!(
        id_map.get(&("src/helpers.inc".into(), "MyUnit.Helper".into())),
        Some(&2)
    );
    assert!(!id_map.contains_key(&("src/helpers.inc".into(), "Helper".into())));

    // The persisted rows were rewritten in place: qname, scope path, and the
    // qname-bearing prefix of symbol_key.
    let (qname, scope, key): (String, Option<String>, String) = db
        .conn()
        .query_row(
            "SELECT qualified_name, scope_path, symbol_key FROM symbols WHERE id = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row 2");
    assert_eq!(qname, "MyUnit.Helper");
    assert_eq!(scope.as_deref(), Some("MyUnit"));
    assert_eq!(key, "1:MyUnit.Helper#function#0");
}

#[test]
fn members_by_parent_lists_included_symbols_under_unit_namespace() {
    let db = Database::open_in_memory().expect("in-memory db");
    let mut parsed = vec![
        unit_file("src/myunit.pas", "MyUnit", &["helpers"]),
        fragment_file("src/helpers.inc"),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();
    id_map.insert(("src/myunit.pas".into(), "MyUnit".into()), 1);
    id_map.insert(("src/helpers.inc".into(), "Helper".into()), 2);
    id_map.insert(("src/helpers.inc".into(), "TThing".into()), 3);
    id_map.insert(("src/helpers.inc".into(), "TThing.Do".into()), 4);

    assemble_includes(&db, &mut parsed, &mut id_map).expect("pass runs");

    // Post-pass, the re-keyed id map is what the pipeline hands the
    // Compilation build — the included symbols land under the unit namespace.
    let tree = Compilation::build(&parsed, &id_map, Arc::new(TypeArena::new()));
    let members = tree.members_of("MyUnit");
    let names: Vec<&str> = members.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"Helper") && names.contains(&"TThing"),
        "members_of(MyUnit) should list the included symbols; got {names:?}"
    );
}

// ---------------------------------------------------------------------------
// Virtual-path join
// ---------------------------------------------------------------------------

#[test]
fn ext_virtual_path_join_resolves_within_the_same_virtual_root() {
    let db = Database::open_in_memory().expect("in-memory db");
    let mut parsed = vec![
        unit_file(
            "ext:fpc:fpc-rtl-objpas/classes/classes.pp",
            "Classes",
            &["classesh"],
        ),
        fragment_file("ext:fpc:fpc-rtl-objpas/classes/classesh.inc"),
        // Same stem in a DIFFERENT virtual root — must not be claimed.
        fragment_file("ext:fpc:fpc-rtl-win/classes/classesh.inc"),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();

    let n = assemble_includes(&db, &mut parsed, &mut id_map).expect("pass runs");
    assert_eq!(n, 3, "only the same-root fragment splices");

    assert_eq!(parsed[1].symbols[0].qualified_name, "Classes.Helper");
    assert_eq!(
        parsed[2].symbols[0].qualified_name, "Helper",
        "the other virtual root's fragment stays bare"
    );
}

#[test]
fn sibling_directory_probe_resolves_when_same_directory_misses() {
    let db = Database::open_in_memory().expect("in-memory db");
    let mut parsed = vec![
        unit_file("src/base/myunit.pas", "MyUnit", &["helpers"]),
        fragment_file("src/include/helpers.inc"),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();

    assemble_includes(&db, &mut parsed, &mut id_map).expect("pass runs");
    assert_eq!(parsed[1].symbols[0].qualified_name, "MyUnit.Helper");
}

// ---------------------------------------------------------------------------
// Cascade shape — a qualified lookup through the unit reaches the member
// ---------------------------------------------------------------------------

#[test]
fn qualified_lookup_through_unit_reaches_included_member() {
    let db = Database::open_in_memory().expect("in-memory db");
    let mut parsed = vec![
        unit_file("src/myunit.pas", "MyUnit", &["helpers"]),
        fragment_file("src/helpers.inc"),
        // A consumer unit whose `uses MyUnit` resolution walks MyUnit's
        // members: present to mirror the cascade shape, not spliced itself.
        unit_file("src/consumer.pas", "Consumer", &[]),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();
    id_map.insert(("src/myunit.pas".into(), "MyUnit".into()), 1);
    id_map.insert(("src/helpers.inc".into(), "Helper".into()), 2);
    id_map.insert(("src/helpers.inc".into(), "TThing".into()), 3);
    id_map.insert(("src/helpers.inc".into(), "TThing.Do".into()), 4);
    id_map.insert(("src/consumer.pas".into(), "Consumer".into()), 5);

    assemble_includes(&db, &mut parsed, &mut id_map).expect("pass runs");

    let tree = Compilation::build(&parsed, &id_map, Arc::new(TypeArena::new()));

    // `uses MyUnit; ... Helper(...)` resolves by qualifying the bare name
    // with the used unit's namespace: the qualified name must now exist and
    // sit among the unit's members.
    let sym = tree
        .by_qualified_name("MyUnit.Helper")
        .expect("MyUnit.Helper resolves after the splice");
    assert_eq!(sym.name, "Helper");
    let members = tree.members_of("MyUnit");
    assert!(
        members.iter().any(|s| s.qualified_name == "MyUnit.Helper"),
        "the included member is reachable through the unit namespace"
    );
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

#[test]
fn a_file_declaring_its_own_namespace_is_never_claimed() {
    let db = Database::open_in_memory().expect("in-memory db");
    let mut parsed = vec![
        unit_file("src/myunit.pas", "MyUnit", &["other"]),
        // Same stem, but a real unit of its own — not an include fragment.
        unit_file("src/other.pas", "Other", &[]),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();

    let n = assemble_includes(&db, &mut parsed, &mut id_map).expect("pass runs");
    assert_eq!(n, 0);
    assert_eq!(parsed[1].symbols[0].qualified_name, "Other");
}

#[test]
fn pass_is_idempotent_across_repeat_invocations() {
    let db = Database::open_in_memory().expect("in-memory db");
    let mut parsed = vec![
        unit_file("src/myunit.pas", "MyUnit", &["helpers"]),
        fragment_file("src/helpers.inc"),
    ];
    let mut id_map: SymbolIdMap = HashMap::new();

    let first = assemble_includes(&db, &mut parsed, &mut id_map).expect("first run");
    assert_eq!(first, 3);
    let second = assemble_includes(&db, &mut parsed, &mut id_map).expect("second run");
    assert_eq!(second, 0, "a spliced fragment is never claimed again");
    assert_eq!(parsed[1].symbols[0].qualified_name, "MyUnit.Helper");
}
