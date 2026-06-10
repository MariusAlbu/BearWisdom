// =============================================================================
// type_checker/core/reexport_tests.rs — generic re-export walker unit tests.
// =============================================================================

use super::follow_reexports;
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::types::EdgeKind;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Synthetic SymbolLookup — models reexports_from / resolve_module_from /
// in_module_from without a DB.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Mock {
    empty: Vec<SymbolInfo>,
    empty_pairs: Vec<(String, String)>,
    /// file_path → [(exported_name, source_module_spec)]
    reexports: FxHashMap<String, Vec<(String, String)>>,
    /// module_spec → resolved file_path
    module_files: FxHashMap<String, String>,
    /// file_path → symbols defined in that file
    in_file: FxHashMap<String, Vec<SymbolInfo>>,
    /// symbol name → symbols with that name
    by_name: FxHashMap<String, Vec<SymbolInfo>>,
}

impl Mock {
    fn reexport(mut self, file: &str, name: &str, source: &str) -> Self {
        self.reexports
            .entry(file.to_string())
            .or_default()
            .push((name.to_string(), source.to_string()));
        self
    }
    fn module_file(mut self, spec: &str, file: &str) -> Self {
        self.module_files.insert(spec.to_string(), file.to_string());
        self
    }
    fn define(mut self, file: &str, sym: SymbolInfo) -> Self {
        self.by_name
            .entry(sym.name.clone())
            .or_default()
            .push(sym.clone());
        self.in_file.entry(file.to_string()).or_default().push(sym);
        self
    }
}

impl SymbolLookup for Mock {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, path: &str) -> &[SymbolInfo] {
        self.in_file
            .get(path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }
    fn in_module_from(&self, _source_file: &str, spec: &str) -> &[SymbolInfo] {
        // Mirror the real index: resolve the spec to a file, else fall back to
        // an exact-path `in_file` lookup (relative specs whose file path equals
        // the spec string, the shape the synthetic barrel tests rely on).
        match self.module_files.get(spec) {
            Some(file) => self.in_file(file),
            None => self.in_file(spec),
        }
    }
    fn resolve_module_from(&self, _source_file: &str, spec: &str) -> Option<&str> {
        self.module_files.get(spec).map(|s| s.as_str())
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        self.reexports
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty_pairs)
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn sym(id: i64, name: &str, kind: &str, file: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from(file),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

fn accept(_: EdgeKind, _: &str) -> bool {
    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn named_reexport_resolves_to_defining_module() {
    // foo: `pub use crate::bar::Thing`  ·  bar: `struct Thing`
    let lookup = Mock::default()
        .reexport("foo.rs", "Thing", "barmod")
        .module_file("barmod", "bar.rs")
        .define("bar.rs", sym(1, "Thing", "class", "bar.rs"));

    let res = follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0)
        .expect("named re-export resolves to the defining module");
    assert_eq!(res.target_symbol_id, 1);
    assert_eq!(res.strategy, "reexport_chain");
    assert_eq!(res.confidence, 1.0);
}

#[test]
fn relative_source_followed_without_module_resolution() {
    // A relative re-export source (`./bar`) is internal by construction and is
    // followed via in_module_from's exact-path fallback even when
    // resolve_module_from is unpopulated — the synthetic-index shape barrel
    // resolution depends on. Regression guard for the bare-vs-relative gate.
    let lookup = Mock::default()
        .reexport("foo.rs", "Thing", "./bar")
        .define("./bar", sym(9, "Thing", "class", "./bar"));

    let res = follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0)
        .expect("relative re-export source is followed without resolve_module_from");
    assert_eq!(res.target_symbol_id, 9);
    assert_eq!(res.strategy, "reexport_chain");
}

#[test]
fn wildcard_reexport_resolves() {
    // foo: `pub use crate::bar::*`  ·  bar: `struct Thing`
    let lookup = Mock::default()
        .reexport("foo.rs", "*", "barmod")
        .module_file("barmod", "bar.rs")
        .define("bar.rs", sym(7, "Thing", "class", "bar.rs"));

    let res = follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0)
        .expect("wildcard re-export resolves");
    assert_eq!(res.target_symbol_id, 7);
    assert_eq!(res.strategy, "reexport_star");
    assert_eq!(res.confidence, 1.0);
}

#[test]
fn two_hop_reexport_resolves() {
    // foo → mid → bar; only bar defines Thing.
    let lookup = Mock::default()
        .reexport("foo.rs", "Thing", "midmod")
        .reexport("mid.rs", "Thing", "barmod")
        .module_file("midmod", "mid.rs")
        .module_file("barmod", "bar.rs")
        .define("bar.rs", sym(2, "Thing", "class", "bar.rs"));

    let res = follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0)
        .expect("two-hop re-export resolves");
    assert_eq!(res.target_symbol_id, 2);
}

#[test]
fn transitive_wildcard_reexport_resolves_indexed_external_source() {
    // Nim shape from nimbus:
    //   api.nim exports/imports common
    //   common.nim exports results
    //   vendor results.nim defines err
    let lookup = Mock::default()
        .reexport("api.nim", "*", "common")
        .reexport("common.nim", "*", "results")
        .module_file("common", "common.nim")
        .module_file("results", "ext:submodule:vendor/nim-results/results.nim")
        .define(
            "ext:submodule:vendor/nim-results/results.nim",
            sym(
                12,
                "err",
                "function",
                "ext:submodule:vendor/nim-results/results.nim",
            ),
        );

    let res = follow_reexports("api.nim", "err", EdgeKind::Calls, accept, &lookup, 0)
        .expect("transitive wildcard re-export resolves to indexed external source");
    assert_eq!(res.target_symbol_id, 12);
    assert_eq!(res.strategy, "reexport_star");
}

#[test]
fn wildcard_reexport_uses_unique_matching_module_file_when_first_suffix_lacks_target() {
    // Nim projects can index several files named results.nim. The module
    // resolver may pick the first suffix hit, but the explicit re-export demand
    // is for `results.err`; if exactly one matching results.nim provider defines
    // err, the generic re-export walk should bind that provider.
    let lookup = Mock::default()
        .reexport("common.nim", "*", "results")
        .module_file("results", "ext:nim:eth/eth/rlp/results.nim")
        .define(
            "ext:submodule:vendor/nim-results/results.nim",
            sym(
                12,
                "err",
                "function",
                "ext:submodule:vendor/nim-results/results.nim",
            ),
        );

    let res = follow_reexports("common.nim", "err", EdgeKind::Calls, accept, &lookup, 0)
        .expect("unique matching results.nim provider should resolve err");
    assert_eq!(res.target_symbol_id, 12);
    assert_eq!(res.strategy, "reexport_star");
}

#[test]
fn empty_reexport_map_does_not_resolve() {
    // The soundness invariant at the walker level: a module that does NOT
    // genuinely re-export the name (its private imports were filtered out of
    // the map) forwards nothing — even though some caller imported the name
    // "from" it. This is what blocks a private `use` from binding through a
    // re-export hop (Invariant #2).
    let lookup = Mock::default()
        .module_file("barmod", "bar.rs")
        .define("bar.rs", sym(3, "Thing", "class", "bar.rs"));

    assert!(
        follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0).is_none(),
        "no re-export entry for foo → no hop"
    );
}

#[test]
fn indexed_external_source_is_followed() {
    // foo re-exports Thing from a module that resolves to an already-indexed
    // external file. This is safe to follow because the edge came from an
    // explicit re-export entry; unresolved bare packages still stay out of this
    // walk.
    let lookup = Mock::default()
        .reexport("foo.rs", "Thing", "pkg")
        .module_file("pkg", "ext:node_modules/pkg/index.d.ts")
        .define(
            "ext:node_modules/pkg/index.d.ts",
            sym(4, "Thing", "class", "ext:node_modules/pkg/index.d.ts"),
        );

    let res = follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0)
        .expect("indexed external re-export source resolves");
    assert_eq!(res.target_symbol_id, 4);
    assert_eq!(res.strategy, "reexport_chain");
}

#[test]
fn unresolvable_source_is_skipped() {
    // The re-export source doesn't resolve to any known file → skipped.
    let lookup = Mock::default().reexport("foo.rs", "Thing", "barmod");

    assert!(
        follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0).is_none(),
        "a source module that resolves to no file must be skipped"
    );
}

#[test]
fn cyclic_reexport_terminates() {
    // foo re-exports Thing from foo (self-cycle); no definition anywhere.
    // The depth guard must terminate the walk rather than recurse forever.
    let lookup = Mock::default()
        .reexport("foo.rs", "Thing", "foomod")
        .module_file("foomod", "foo.rs");

    assert!(
        follow_reexports("foo.rs", "Thing", EdgeKind::TypeRef, accept, &lookup, 0).is_none(),
        "a re-export cycle must terminate via the depth guard"
    );
}
