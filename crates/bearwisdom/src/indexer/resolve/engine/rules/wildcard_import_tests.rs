use super::*;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    DEFAULT_PROFILE, LanguageProfile, WildcardMatch,
};

static FILESTEM_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    wildcard_match: WildcardMatch::FileStem {
        underscore_prefix: false,
    },
    ..DEFAULT_PROFILE
};

static PACKAGE_ROOT_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    wildcard_match: WildcardMatch::PackageRoot,
    ..DEFAULT_PROFILE
};

fn wildcard_import(name: &str, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

fn resolve(lookup: &Lookup, target: &str, imports: Vec<ImportEntry>) -> Option<i64> {
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
        profile: &DEFAULT_PROFILE,
    };
    match WildcardImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_direct_qname_member_under_wildcard() {
    // `use my::mod::*` brings `my::mod::Foo` into scope; target `Foo` resolves.
    let lookup = Lookup::new().with(sym(10, "Foo", "my.mod.Foo", "class", "src/mod.rs"));
    let imports = vec![wildcard_import("*", "my.mod")];
    assert_eq!(resolve(&lookup, "Foo", imports), Some(10));
}

#[test]
fn declines_when_no_wildcard_imports_present() {
    let lookup = Lookup::new().with(sym(10, "Foo", "my.mod.Foo", "class", "src/mod.rs"));
    // A non-wildcard import — WildcardImportRule must decline.
    let imports = vec![ImportEntry {
        imported_name: "Foo".to_string(),
        module_path: Some("my.mod".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

#[test]
fn declines_dotted_target() {
    let lookup = Lookup::new().with(sym(10, "Foo", "my.mod.Foo", "class", "src/mod.rs"));
    let imports = vec![wildcard_import("*", "my.mod")];
    // Dotted targets are qualified — this rule declines them.
    assert_eq!(resolve(&lookup, "my.mod.Foo", imports), None);
}

#[test]
fn declines_when_symbol_is_not_direct_member() {
    // `my.mod.sub.Foo` is two segments deeper — NOT a direct member of `my.mod`.
    let lookup =
        Lookup::new().with(sym(10, "Foo", "my.mod.sub.Foo", "class", "src/mod.rs"));
    let imports = vec![wildcard_import("*", "my.mod")];
    assert_eq!(resolve(&lookup, "Foo", imports), None);
}

#[test]
fn filestem_mode_binds_by_file_basename() {
    // WildcardMatch::FileStem — the symbol's file stem must match the module name.
    let lookup = Lookup::new().with(sym(20, "Bar", "Bar", "class", "src/utils.rs"));
    let imports = vec![wildcard_import("*", "utils")];
    let r = call_ref("Bar");
    let s = source_symbol("caller");
    let fc = file_ctx(imports, None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &FILESTEM_PROFILE,
    };
    match WildcardImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 20),
        _ => panic!("expected Resolved"),
    }
}

// --- PackageRoot mode ---------------------------------------------------------

#[test]
fn package_root_mode_binds_external_candidate_by_package_segment() {
    // `import 'package:flutter/material.dart'` reduces to package identity
    // "flutter" on `module`; a candidate declared in a DIFFERENT file under
    // the same `ext:flutter-sdk:flutter/…` package still binds — PackageRoot
    // reaches through the barrel without requiring the candidate's own file
    // to be named "material".
    let lookup = Lookup::new().with(sym(
        10,
        "BuildContext",
        "BuildContext",
        "class",
        "ext:flutter-sdk:flutter/src/widgets/framework.dart",
    ));
    let imports = vec![wildcard_import("*", "flutter")];
    assert_eq!(
        resolve_with_profile(&lookup, "BuildContext", imports, &PACKAGE_ROOT_PROFILE),
        Some(10)
    );
}

#[test]
fn package_root_mode_declines_a_different_package() {
    let lookup = Lookup::new().with(sym(
        10,
        "Foo",
        "Foo",
        "class",
        "ext:dart:some_other_pkg/lib/foo.dart",
    ));
    let imports = vec![wildcard_import("*", "flutter")];
    assert_eq!(
        resolve_with_profile(&lookup, "Foo", imports, &PACKAGE_ROOT_PROFILE),
        None
    );
}

#[test]
fn package_root_mode_falls_back_to_file_stem_for_internal_candidate() {
    // A schemeless (relative, same-project) wildcard's `module` carries the
    // bare file stem, not a package id — PackageRoot falls back to the same
    // file-stem check FileStem uses so a same-project wildcard import keeps
    // resolving exactly as it did before this mode existed.
    let lookup = Lookup::new().with(sym(20, "Bar", "Bar", "class", "src/widgets.dart"));
    let imports = vec![wildcard_import("*", "widgets")];
    assert_eq!(
        resolve_with_profile(&lookup, "Bar", imports, &PACKAGE_ROOT_PROFILE),
        Some(20)
    );
}

#[test]
fn package_root_mode_internal_candidate_ignores_package_name_match() {
    // An internal candidate is never package-segment matched (only an
    // external `ext:` file carries a package segment) — a same-named wildcard
    // module must still line up via the file-stem fallback, not a bare
    // string coincidence.
    let lookup = Lookup::new().with(sym(30, "Baz", "Baz", "class", "src/other.dart"));
    let imports = vec![wildcard_import("*", "flutter")];
    assert_eq!(
        resolve_with_profile(&lookup, "Baz", imports, &PACKAGE_ROOT_PROFILE),
        None
    );
}

// --- qname-distinct hit counting + implicit namespaces -----------------------

static NAMESPACE_WILDCARD_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    namespace_imports_are_wildcards: true,
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
    match WildcardImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn same_qname_duplicate_rows_are_one_unambiguous_hit() {
    // One declaration surfaced as several rows under a single qname (arity
    // overloads: IEquatable / IEquatable<T>) is NOT ambiguous.
    let lookup = Lookup::new()
        .with(sym(10, "IEquatable", "System.IEquatable", "interface", "ext:dotnet:CoreLib/CoreLib"))
        .with(sym(11, "IEquatable", "System.IEquatable", "interface", "ext:dotnet:CoreLib/CoreLib"))
        .with(sym(12, "IEquatable", "System.IEquatable", "interface", "ext:dotnet:CoreLib/CoreLib"));
    let imports = vec![wildcard_import("System", "System")];
    assert_eq!(resolve(&lookup, "IEquatable", imports), Some(10));
}

#[test]
fn two_distinct_qnames_stay_ambiguous() {
    let lookup = Lookup::new()
        .with(sym(10, "Color", "System.Color", "class", "ext:dotnet:CoreLib/CoreLib"))
        .with(sym(11, "Color", "MyApp.Color", "class", "src/Color.cs"));
    let imports = vec![
        wildcard_import("System", "System"),
        wildcard_import("MyApp", "MyApp"),
    ];
    assert_eq!(resolve(&lookup, "Color", imports), None);
}

#[test]
fn manifest_implicit_namespaces_open_bare_scope() {
    // `<ImplicitUsings>enable</ImplicitUsings>` — no `using System;` in the
    // file, yet bare `Guid` binds to `System.Guid`. Internal or external
    // origin is immaterial; the candidate set is the whole index.
    let lookup = Lookup::new()
        .with(sym(20, "Guid", "System.Guid", "struct", "ext:dotnet:CoreLib/CoreLib"))
        .with_implicit_namespaces(&["System"]);
    assert_eq!(
        resolve_with_profile(&lookup, "Guid", vec![], &NAMESPACE_WILDCARD_PROFILE),
        Some(20)
    );
}

#[test]
fn implicit_namespaces_gated_on_the_profile_flag() {
    let lookup = Lookup::new()
        .with(sym(20, "Guid", "System.Guid", "struct", "ext:dotnet:CoreLib/CoreLib"))
        .with_implicit_namespaces(&["System"]);
    assert_eq!(
        resolve_with_profile(&lookup, "Guid", vec![], &DEFAULT_PROFILE),
        None
    );
}

#[test]
fn internal_namespace_member_binds_the_same_as_external() {
    // `using Microsoft.FluentUI.AspNetCore.Components;` + bare `Icon` where
    // Icon is an INTERNAL class in that namespace — no origin distinction.
    let lookup = Lookup::new().with(sym(
        30,
        "Icon",
        "Microsoft.FluentUI.AspNetCore.Components.Icon",
        "class",
        "src/Components/Icon.cs",
    ));
    let imports = vec![wildcard_import(
        "Microsoft.FluentUI.AspNetCore.Components",
        "Microsoft.FluentUI.AspNetCore.Components",
    )];
    assert_eq!(resolve(&lookup, "Icon", imports), Some(30));
}
