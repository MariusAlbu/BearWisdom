// =============================================================================
// indexer/expand_tests.rs — unit tests for indexer/expand.rs
//
// Kept in a sibling file so the production module stays free of synthetic
// fixture literals (dep names, file paths) that look like hardcoded
// production values to a casual reader.
// =============================================================================

use super::*;

#[test]
fn empty_misses_returns_zero_stats() {
    // Smoke test: we don't even touch the DB if there's nothing to do.
    // This is the hot path on a project with perfect resolution.
    let stats = ExpansionStats::default();
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.mapped, 0);
    assert_eq!(stats.new_files, 0);
}

#[test]
fn module_scoped_demand_locates_within_its_package_only() {
    // EXT-1: two packages export the same name. A module-scoped demand must
    // pull only the file inside its own module — never the coincidental
    // same-name symbol in the other package.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg", "find", "/nm/pkg/index.d.ts");
    idx.insert("other", "find", "/nm/other/index.d.ts");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "find".to_string(),
        module: Some("pkg".to_string()),
        source_path: String::new(),
    };
    let hits = locate_via_symbol_index(&idx, &miss);
    assert_eq!(hits, vec![std::path::PathBuf::from("/nm/pkg/index.d.ts")]);
}

#[test]
fn module_scoped_demand_misses_without_cross_package_fallback() {
    // A name absent under its declared module is a genuine gap — no
    // find_by_name fallback that would grep a same-name symbol elsewhere.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("other", "find", "/nm/other/index.d.ts");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "find".to_string(),
        module: Some("pkg".to_string()),
        source_path: String::new(),
    };
    assert!(locate_via_symbol_index(&idx, &miss).is_empty());
}

#[test]
fn module_less_demand_keeps_find_by_name() {
    // A bare/ambient demand (module: None) keeps the existing whole-index
    // find_by_name probe — the EXT-1 change is additive, not a replacement.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("pkg", "find", "/nm/pkg/index.d.ts");
    idx.insert("other", "find", "/nm/other/index.d.ts");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "find".to_string(),
        module: None,
        source_path: String::new(),
    };
    assert_eq!(locate_via_symbol_index(&idx, &miss).len(), 2);
}

#[test]
fn include_path_demand_locates_header_by_include_path() {
    // A C `#include <windows.h>` records a chain miss whose module AND
    // target_name are the include-path. The path-keyed header index keys
    // headers at `(include_path, include_path)`, so the module-scoped
    // locate hits the registered header — this is the include-driven
    // admission seam.
    let mut idx = SymbolLocationIndex::new();
    idx.insert("windows.h", "windows.h", "/sdk/um/windows.h");

    let miss = ChainMiss {
        current_type: String::new(),
        target_name: "windows.h".to_string(),
        module: Some("windows.h".to_string()),
        source_path: String::new(),
    };
    let hits = locate_via_symbol_index(&idx, &miss);
    assert_eq!(hits, vec![std::path::PathBuf::from("/sdk/um/windows.h")]);
}

// ---------------------------------------------------------------------------
// Transitive `#include` closure (header A includes B which defines a type)
// ---------------------------------------------------------------------------

#[test]
fn header_include_shape_classifies_headers_vs_relative() {
    assert!(header_include_shape("stdio.h"));
    assert!(header_include_shape("openssl/bio.h"));
    assert!(header_include_shape("vector"));
    assert!(!header_include_shape("./local"));
    assert!(!header_include_shape("sub/module"));
}

#[test]
fn harvest_collects_header_includes_only_from_c_family() {
    use crate::types::{EdgeKind, ExtractedRef, FlowMeta, ParsedFile};

    fn import_ref(module: &str) -> ExtractedRef {
        ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: 0,
            target_name: module.rsplit('/').next().unwrap_or(module).to_string(),
            kind: EdgeKind::Imports,
            line: 0,
            col: 0,
            module: Some(module.to_string()),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        }
    }
    fn pf(lang: &str, refs: Vec<ExtractedRef>) -> ParsedFile {
        ParsedFile {
            path: format!("ext:idx:/sdk/{lang}.h"),
            language: lang.to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            symbols: Vec::new(),
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

    let c_file = pf(
        "c",
        vec![import_ref("winerror.h"), import_ref("./project_local")],
    );
    let cpp_file = pf("cpp", vec![import_ref("vector")]);
    // A non-C file's imports must never be admitted as header includes.
    let ts_file = pf("typescript", vec![import_ref("react.h")]);

    let harvested = harvest_header_includes(&[c_file, cpp_file, ts_file]);
    assert!(harvested.contains("winerror.h"));
    assert!(harvested.contains("vector"));
    assert!(!harvested.contains("./project_local"));
    assert!(!harvested.contains("react.h"));
    assert_eq!(harvested.len(), 2);
}

#[test]
fn transitive_closure_pulls_included_header_within_cap() {
    use crate::ecosystem::posix_headers::_test_build_c_header_index as build_c_header_index;
    use crate::ecosystem::posix_headers::_test_make_root as make_root;
    use std::fs;
    use tempfile::TempDir;

    // SDK layout: `a.h` includes `b.h`, and `b.h` defines a type. The first
    // wave (symbol-miss) pulls `a.h`; the transitive closure must then follow
    // `a.h`'s own `#include "b.h"` and pull `b.h` so its type lands in the
    // index. Before the closure wiring this header was never reached.
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("b.h"), "int b_entry(void);\n").unwrap();
    fs::write(
        tmp.path().join("a.h"),
        "#include \"b.h\"\nint a_entry(void);\n",
    )
    .unwrap();

    let dep = make_root(tmp.path(), "test");
    let index = build_c_header_index(&[dep]);
    assert!(
        index.locate("b.h", "b.h").is_some(),
        "fixture: b.h must be registered by include-path"
    );

    let registry = crate::languages::default_registry();
    // Parse the seed header `a.h` as a real external C file so its
    // `#include "b.h"` is genuinely extracted as an Imports ref.
    let seed = parse_file_with_demand(
        &WalkedFile {
            relative_path: format!(
                "ext:idx:{}",
                tmp.path().join("a.h").to_string_lossy().replace('\\', "/")
            ),
            absolute_path: tmp.path().join("a.h"),
            language: "c",
        },
        registry,
        None,
    )
    .expect("parse seed header");

    let mut db = Database::open_in_memory().unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut walked = std::collections::HashSet::new();
    let result = follow_header_includes(
        &mut db,
        &[seed],
        &index,
        registry,
        None,
        &mut seen,
        &mut walked,
    )
    .expect("transitive closure");

    assert!(
        result.new_files >= 1,
        "expected b.h to be pulled, got {} files",
        result.new_files
    );
    // The pulled header's declaration lands in the DB as an external symbol.
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'b_entry'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "b.h's b_entry declaration must be admitted");
}

#[test]
fn transitive_closure_declines_unanswerable_include() {
    use crate::ecosystem::posix_headers::_test_build_c_header_index as build_c_header_index;
    use crate::ecosystem::posix_headers::_test_make_root as make_root;
    use std::fs;
    use tempfile::TempDir;

    // The seed header includes a path the index does NOT carry. The closure
    // must locate nothing, pull nothing, and return cleanly — never scan the
    // filesystem for a basename or run away.
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("a.h"),
        "#include <not_in_index.h>\nint a_entry(void);\n",
    )
    .unwrap();
    let dep = make_root(tmp.path(), "test");
    // Build an index that deliberately omits `not_in_index.h` by pointing the
    // root at an empty sibling dir.
    let empty = TempDir::new().unwrap();
    let index = build_c_header_index(&[make_root(empty.path(), "test")]);
    let _ = dep;

    let registry = crate::languages::default_registry();
    let seed = parse_file_with_demand(
        &WalkedFile {
            relative_path: format!(
                "ext:idx:{}",
                tmp.path().join("a.h").to_string_lossy().replace('\\', "/")
            ),
            absolute_path: tmp.path().join("a.h"),
            language: "c",
        },
        registry,
        None,
    )
    .expect("parse seed header");

    let mut db = Database::open_in_memory().unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut walked = std::collections::HashSet::new();
    let result = follow_header_includes(
        &mut db,
        &[seed],
        &index,
        registry,
        None,
        &mut seen,
        &mut walked,
    )
    .expect("transitive closure");

    assert_eq!(
        result.new_files, 0,
        "an include the index can't answer must pull nothing"
    );
}
