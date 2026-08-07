use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::{
    ExtMatch, ExternalByImport, DEFAULT_PROFILE,
};

fn resolve(
    lookup: &Lookup,
    target: &str,
    imports: Vec<crate::indexer::resolve::engine::contract::ImportEntry>,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> Option<i64> {
    use crate::indexer::resolve::engine::testkit::call_ref;
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
    match ExternalByImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Gate is `Off` by default (no `external_by_import` set) — rule passes.
#[test]
fn passes_when_gate_off() {
    let lookup = Lookup::new().with(sym(1, "useState", "useState", "function", "ext:ts:react/index.d.ts"));
    assert_eq!(
        resolve(&lookup, "useState", vec![import("react", Some("react"))], &DEFAULT_PROFILE),
        None
    );
}

/// `PkgSegment` mode: external file's pkg segment matches an import root.
#[test]
fn pkg_segment_matches_import_root() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            external_by_import: Some(ExternalByImport),
            ext_match: ExtMatch::PkgSegment,
            ..DEFAULT_PROFILE
        };
    let lookup =
        Lookup::new().with(sym(10, "useState", "useState", "function", "ext:ts:react/index.d.ts"));
    let imports = vec![import("useState", Some("react"))];
    assert_eq!(resolve(&lookup, "useState", imports, &PROFILE), Some(10));
}

/// `PkgSegment` mode: gem family match — `aws-sdk-s3` under import root `aws`.
#[test]
fn pkg_segment_matches_family_prefix() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            external_by_import: Some(ExternalByImport),
            ext_match: ExtMatch::PkgSegment,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(
        20,
        "Client",
        "Client",
        "class",
        "ext:ruby:aws-sdk-s3/lib/client.rb",
    ));
    let imports = vec![import("Aws", Some("aws"))];
    assert_eq!(resolve(&lookup, "Client", imports, &PROFILE), Some(20));
}

/// `FileStemOrDir` mode: external file's basename-stem matches import leaf.
#[test]
fn file_stem_or_dir_matches_import_leaf() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            external_by_import: Some(ExternalByImport),
            ext_match: ExtMatch::FileStemOrDir,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(
        30,
        "newHttpClient",
        "newHttpClient",
        "function",
        "ext:nim:httpclient/httpclient.nim",
    ));
    let imports = vec![import("httpclient", Some("httpclient"))];
    assert_eq!(resolve(&lookup, "newHttpClient", imports, &PROFILE), Some(30));
}

/// Non-external symbol is skipped even when the name matches.
#[test]
fn skips_non_external_symbols() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            external_by_import: Some(ExternalByImport),
            ext_match: ExtMatch::PkgSegment,
            ..DEFAULT_PROFILE
        };
    // File path does NOT start with `ext:`.
    let lookup =
        Lookup::new().with(sym(40, "useState", "useState", "function", "src/hooks/state.ts"));
    let imports = vec![import("useState", Some("react"))];
    assert_eq!(resolve(&lookup, "useState", imports, &PROFILE), None);
}
