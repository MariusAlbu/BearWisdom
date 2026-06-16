use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    LanguageProfile, DEFAULT_PROFILE,
};

const ENABLED_PROFILE: LanguageProfile = LanguageProfile {
    explicit_member_import: true,
    ..DEFAULT_PROFILE
};

fn resolve(
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
    match ExplicitMemberImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_when_module_last_segment_matches_target() {
    // `import NSData from 'Foundation.NSData'` — only one internal symbol named NSData.
    let lookup = Lookup::new().with(sym(40, "NSData", "Foundation.NSData", "class", "src/a.swift"));
    let imports = vec![ImportEntry {
        imported_name: "NSData".to_string(),
        module_path: Some("Foundation.NSData".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(
        resolve(&lookup, "NSData", imports, &ENABLED_PROFILE),
        Some(40)
    );
}

#[test]
fn declines_when_gate_is_off() {
    let lookup = Lookup::new().with(sym(41, "NSData", "Foundation.NSData", "class", "src/a.swift"));
    let imports = vec![ImportEntry {
        imported_name: "NSData".to_string(),
        module_path: Some("Foundation.NSData".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    // DEFAULT_PROFILE has explicit_member_import: false.
    assert_eq!(
        resolve(&lookup, "NSData", imports, &DEFAULT_PROFILE),
        None
    );
}

#[test]
fn declines_when_multiple_internal_candidates_exist() {
    // Ambiguous — two symbols named NSData; must not bind.
    let lookup = Lookup::new()
        .with(sym(42, "NSData", "A.NSData", "class", "src/a.swift"))
        .with(sym(43, "NSData", "B.NSData", "class", "src/b.swift"));
    let imports = vec![ImportEntry {
        imported_name: "NSData".to_string(),
        module_path: Some("Foundation.NSData".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(
        resolve(&lookup, "NSData", imports, &ENABLED_PROFILE),
        None
    );
}

#[test]
fn declines_when_module_has_no_dot() {
    // `import Foundation` — single segment, no dot in module path; rule stays inert.
    let lookup = Lookup::new().with(sym(44, "Foundation", "Foundation", "module", "src/a.swift"));
    let imports = vec![import("Foundation", Some("Foundation"))];
    assert_eq!(
        resolve(&lookup, "Foundation", imports, &ENABLED_PROFILE),
        None
    );
}
