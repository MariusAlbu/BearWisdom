use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    ChainQualification, LanguageProfile, DEFAULT_PROFILE,
};

/// Profile with the gate active — `PackageShortName`.
const PSN_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    chain_qualification: ChainQualification::PackageShortName,
    ..DEFAULT_PROFILE
};

fn resolve_with_profile(
    lookup: &Lookup,
    target: &str,
    imports: Vec<ImportEntry>,
    profile: &LanguageProfile,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile,
    };
    match PackageShortNameRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn gate_off_returns_pass() {
    // DEFAULT_PROFILE has ChainQualification::None — rule must not bind.
    let lookup = Lookup::new().with(sym(60, "NewRouter", "gin.NewRouter", "function", "src/a.go"));
    let imports = vec![import("gin", Some("github.com/gin-gonic/gin"))];
    let result = resolve_with_profile(&lookup, "NewRouter", imports, &DEFAULT_PROFILE);
    assert_eq!(result, None);
}

#[test]
fn gate_on_binds_via_imported_name() {
    // `import gin "github.com/gin-gonic/gin"` → probe `gin.NewRouter`.
    let lookup =
        Lookup::new().with(sym(61, "NewRouter", "gin.NewRouter", "function", "src/gin/router.go"));
    let imports = vec![import("gin", Some("github.com/gin-gonic/gin"))];
    let result = resolve_with_profile(&lookup, "NewRouter", imports, &PSN_PROFILE);
    assert_eq!(result, Some(61));
}

#[test]
fn gate_on_binds_via_last_path_segment() {
    // No alias (`imported_name` is empty); fall back to `gin` from the module path.
    let lookup =
        Lookup::new().with(sym(62, "Default", "gin.Default", "function", "src/gin/router.go"));
    let imports = vec![import("", Some("github.com/gin-gonic/gin"))];
    let result = resolve_with_profile(&lookup, "Default", imports, &PSN_PROFILE);
    assert_eq!(result, Some(62));
}

#[test]
fn declines_dotted_target() {
    // A dotted target is not a bare name — rule returns Pass regardless of gate.
    let lookup = Lookup::new().with(sym(
        63,
        "NewRouter",
        "gin.NewRouter",
        "function",
        "src/a.go",
    ));
    let imports = vec![import("gin", Some("github.com/gin-gonic/gin"))];
    let result = resolve_with_profile(&lookup, "gin.NewRouter", imports, &PSN_PROFILE);
    assert_eq!(result, None);
}
