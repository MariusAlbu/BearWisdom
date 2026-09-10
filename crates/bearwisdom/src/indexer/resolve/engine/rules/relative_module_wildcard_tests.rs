use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    LanguageProfile, WildcardMatch, DEFAULT_PROFILE,
};

fn physical_candidates(importing_file: &str, module: &str, target: &str) -> Vec<String> {
    match (importing_file, module, target) {
        ("project/consumer.lang", "parent", "parse") => vec!["project/parent.lang".to_string()],
        _ => Vec::new(),
    }
}

static PHYSICAL_WILDCARD_PROFILE: LanguageProfile = LanguageProfile {
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        wildcard_match: WildcardMatch::QnameUnderWithPhysicalFiles {
            candidate_files: physical_candidates,
        },
        ..DEFAULT_PROFILE.imports
    },
    ..DEFAULT_PROFILE
};

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: "*".to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
        binding_kind: None,
    }
}

fn resolve_in(
    lookup: &Lookup,
    target: &str,
    file_path: &str,
    imports: Vec<ImportEntry>,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let mut fc = file_ctx(imports, None);
    fc.file_path = file_path.to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &PHYSICAL_WILDCARD_PROFILE,
    };
    match RelativeModuleWildcardRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn profile_candidate_file_binds_same_named_symbol() {
    let lookup = Lookup::new().with(sym(5, "parse", "parse", "function", "project/parent.lang"));
    let imports = vec![wildcard_import("parent")];
    let got = resolve_in(&lookup, "parse", "project/consumer.lang", imports);
    assert_eq!(got, Some(5));
}

#[test]
fn no_profile_candidate_leaves_rule_inert() {
    let lookup = Lookup::new().with(sym(9, "parse", "parse", "function", "project/other.lang"));
    let imports = vec![wildcard_import("unrelated")];
    let got = resolve_in(&lookup, "parse", "project/consumer.lang", imports);
    assert_eq!(got, None);
}
