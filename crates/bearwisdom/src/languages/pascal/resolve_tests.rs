// =============================================================================
// pascal/resolve_tests.rs — unit tests for pascal/resolve.rs
// =============================================================================

use super::{pascal_stem_matches, resolve_pascal_wildcard};
use crate::indexer::resolve::engine::{FileContext, ImportEntry, SymbolInfo, SymbolLookup};
use crate::types::EdgeKind;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// pascal_stem_matches
// ---------------------------------------------------------------------------

#[test]
fn stem_matches_exact_pas() {
    assert!(pascal_stem_matches("src/sysutils.pas", "sysutils"));
}

#[test]
fn stem_matches_exact_case_insensitive() {
    assert!(pascal_stem_matches("src/CastleUtils.pas", "castleutils"));
}

#[test]
fn stem_matches_inc_prefix() {
    assert!(pascal_stem_matches(
        "src/base/castleutils_math.inc",
        "castleutils"
    ));
}

#[test]
fn stem_matches_inc_prefix_deep() {
    assert!(pascal_stem_matches(
        "audio/castlesoundengine_allocator.inc",
        "castlesoundengine"
    ));
}

#[test]
fn stem_does_not_match_unrelated() {
    assert!(!pascal_stem_matches("src/classes.pas", "sysutils"));
}

#[test]
fn stem_does_not_match_partial_prefix() {
    // "castleutils_math.inc" stem starts with "castle", not "cast_"
    // so querying with module "cast" should NOT match
    assert!(!pascal_stem_matches("src/castleutils_math.inc", "cast"));
}

// ---------------------------------------------------------------------------
// resolve_pascal_wildcard — minimal SymbolLookup stub
// ---------------------------------------------------------------------------

struct ByNameLookup {
    symbols: Vec<SymbolInfo>,
}

impl SymbolLookup for ByNameLookup {
    fn by_name(&self, _name: &str) -> &[SymbolInfo] {
        &self.symbols
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &[]
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &[]
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &[]
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
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn make_sym(id: i64, name: &str, kind: &str, file_path: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from(file_path),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: module.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

fn file_ctx_with_imports(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/main.pas".to_string(),
        language: "pascal".to_string(),
        imports,
        file_namespace: None,
    }
}

#[test]
fn wildcard_resolves_symbol_in_inc_prefix_file() {
    // Unit "CastleUtils" imported; symbol "CastleNow" lives in "castleutils_now.inc"
    let lookup = ByNameLookup {
        symbols: vec![make_sym(42, "CastleNow", "function", "src/base/castleutils_now.inc")],
    };
    let ctx = file_ctx_with_imports(vec![wildcard_import("CastleUtils")]);
    let result = resolve_pascal_wildcard(EdgeKind::Calls, "CastleNow", "castlenow", &ctx, &lookup);
    assert!(
        result.is_some(),
        "expected resolution for CastleNow in castleutils_now.inc via CastleUtils import"
    );
    assert_eq!(result.unwrap().target_symbol_id, 42);
}

#[test]
fn wildcard_resolves_symbol_in_exact_stem_file() {
    // Unit "SysUtils" imported; symbol "FreeAndNil" lives in "sysutils.pas"
    let lookup = ByNameLookup {
        symbols: vec![make_sym(7, "FreeAndNil", "function", "rtl/sysutils.pas")],
    };
    let ctx = file_ctx_with_imports(vec![wildcard_import("SysUtils")]);
    let result =
        resolve_pascal_wildcard(EdgeKind::Calls, "FreeAndNil", "freeandnil", &ctx, &lookup);
    assert!(
        result.is_some(),
        "expected resolution for FreeAndNil in sysutils.pas via SysUtils import"
    );
}

#[test]
fn wildcard_does_not_resolve_unrelated_module() {
    // "CastleNow" is in castleutils_now.inc but only "Classes" is imported
    let lookup = ByNameLookup {
        symbols: vec![make_sym(42, "CastleNow", "function", "src/base/castleutils_now.inc")],
    };
    let ctx = file_ctx_with_imports(vec![wildcard_import("Classes")]);
    let result = resolve_pascal_wildcard(EdgeKind::Calls, "CastleNow", "castlenow", &ctx, &lookup);
    assert!(result.is_none(), "should not resolve across unrelated import");
}

#[test]
fn wildcard_resolves_class_via_calls_edge() {
    // Calls edge must also match class symbols (type references use Calls kind)
    let lookup = ByNameLookup {
        symbols: vec![make_sym(
            5,
            "TSoundAllocator",
            "class",
            "audio/castlesoundengine_allocator.inc",
        )],
    };
    let ctx = file_ctx_with_imports(vec![wildcard_import("CastleSoundEngine")]);
    let result = resolve_pascal_wildcard(
        EdgeKind::Calls,
        "TSoundAllocator",
        "tsoundallocator",
        &ctx,
        &lookup,
    );
    assert!(
        result.is_some(),
        "expected class to resolve via Calls edge; predicates should accept class for Calls"
    );
}
