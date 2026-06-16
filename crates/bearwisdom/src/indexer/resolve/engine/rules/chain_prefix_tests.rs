use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{ChainSegment, MemberChain, SegmentKind};

fn make_chain(segments: Vec<&str>) -> MemberChain {
    MemberChain {
        segments: segments
            .into_iter()
            .map(|n| ChainSegment {
                name: n.to_string(),
                node_kind: String::new(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            })
            .collect(),
    }
}

fn resolve(
    lookup: &Lookup,
    target: &str,
    chain_segments: Vec<&str>,
    imports: Vec<ImportEntry>,
) -> Option<i64> {
    let mut r = call_ref(target);
    if !chain_segments.is_empty() {
        r.chain = Some(make_chain(chain_segments));
    }
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match ChainPrefixRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Chain prefix matches an import; the target symbol's file path matches the
/// module specifier.
#[test]
fn binds_via_matching_import_module() {
    let lookup = Lookup::new().with(sym(10, "helper", "helper", "function", "src/sdk.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Sdk".to_string(),
        module_path: Some("./sdk".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    // Chain: [Sdk, helper] — prefix is Sdk, target is helper.
    let got = resolve(&lookup, "helper", vec!["Sdk", "helper"], imports);
    assert_eq!(got, Some(10));
}

/// Chain prefix matches via an import alias.
#[test]
fn binds_via_import_alias() {
    let lookup = Lookup::new().with(sym(11, "doWork", "doWork", "function", "src/utils.ts"));
    let imports = vec![ImportEntry {
        imported_name: "utils".to_string(),
        module_path: Some("./utils".to_string()),
        alias: Some("U".to_string()),
        is_wildcard: false,
    }];
    let got = resolve(&lookup, "doWork", vec!["U", "doWork"], imports);
    assert_eq!(got, Some(11));
}

/// No matching import for the prefix — rule declines.
#[test]
fn declines_when_no_import_matches_prefix() {
    let lookup = Lookup::new().with(sym(12, "helper", "helper", "function", "src/sdk.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Other".to_string(),
        module_path: Some("./other".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    let got = resolve(&lookup, "helper", vec!["Sdk", "helper"], imports);
    assert_eq!(got, None);
}

/// Chain has fewer than 2 segments — rule declines immediately.
#[test]
fn declines_chain_too_short() {
    let lookup = Lookup::new().with(sym(13, "helper", "helper", "function", "src/sdk.ts"));
    let imports = vec![ImportEntry {
        imported_name: "Sdk".to_string(),
        module_path: Some("./sdk".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    let got = resolve(&lookup, "helper", vec!["helper"], imports);
    assert_eq!(got, None);
}
