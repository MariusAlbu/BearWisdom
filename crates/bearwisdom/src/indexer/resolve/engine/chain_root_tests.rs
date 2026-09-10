// =============================================================================
// engine/chain_root_tests — where a member chain's walk begins
//
// Covers the namespace-anchored root: a chain whose leading segments name a
// namespace path rather than a value, plus the ordinary roots that must keep
// starting at segment 1.
// =============================================================================

use crate::indexer::resolve::engine::cause::{Cause, CauseKind};
use crate::indexer::resolve::engine::chain::bind_member_access;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry};
use crate::indexer::resolve::engine::testkit::{
    call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::languages::go::profile::GO_PROFILE;
use crate::type_checker::profile::chain_specs::{ChainQualification, QualifiedImportRoot};
use crate::type_checker::profile::import_specs::ModulePathMatch;
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};
use crate::types::{ChainSegment, MemberChain, SegmentKind};

/// A plain identifier segment — the shape every namespace/type segment takes.
fn seg(name: &str) -> ChainSegment {
    segment(name, SegmentKind::Identifier, false)
}

/// A called member segment — the chain's last hop.
fn member(name: &str) -> ChainSegment {
    segment(name, SegmentKind::Property, true)
}

fn segment(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

fn type_access(name: &str) -> ChainSegment {
    segment(name, SegmentKind::TypeAccess, false)
}

/// A `using NS;` entry — the form `build_file_context` produces for a plain
/// namespace import under a profile that treats one as a wildcard.
fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: "*".to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
        binding_kind: None,
    }
}

fn named_import(name: &str, module: &str, alias: Option<&str>) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: alias.map(str::to_string),
        is_wildcard: false,
        binding_kind: Some(SegmentKind::TypeAccess),
    }
}

fn qualified_module_path(module: &str) -> ModulePathMatch {
    ModulePathMatch::heuristic(module)
}

fn qualified_type_candidates(module: &str, _imported: &str, tail: &str) -> Vec<String> {
    vec![format!("{module}.{tail}")]
}

fn qualified_import_profile() -> LanguageProfile {
    let mut profile = DEFAULT_PROFILE;
    profile.chain_qualification = ChainQualification::SamePackageAndImportsWithQualifiedRoot(
        QualifiedImportRoot {
            module_path_adapter: qualified_module_path,
            type_candidates: qualified_type_candidates,
        },
    );
    profile
}

#[test]
fn qualified_type_import_anchors_static_receiver() {
    let lookup = Lookup::new()
        .with(sym(
            40,
            "Item",
            "alpha.models.Item",
            "class",
            "vendor/alpha/models/Item.gen",
        ))
        .with(sym(
            41,
            "Item",
            "other.Item",
            "class",
            "src/other/Item.gen",
        ))
        .with_member_id(
            40,
            sym(
                42,
                "load",
                "alpha.models.Item.load",
                "method",
                "vendor/alpha/models/Item.gen",
            ),
        );
    let fc = file_ctx(vec![named_import("models", "alpha.models", None)], None);
    let segs = vec![type_access("models.Item"), member("load")];
    let profile = qualified_import_profile();

    assert_eq!(
        drive_with_profile(&lookup, segs, &fc, &profile).expect("qualified import bind"),
        42
    );
}

#[test]
fn aliased_qualified_type_import_anchors_nested_type() {
    let lookup = Lookup::new()
        .with(sym(
            43,
            "Factory",
            "alpha.models.factories.Factory",
            "class",
            "vendor/alpha/models/factories/Factory.gen",
        ))
        .with_member_id(
            43,
            sym(
                44,
                "new",
                "alpha.models.factories.Factory.new",
                "method",
                "vendor/alpha/models/factories/Factory.gen",
            ),
        );
    let fc = file_ctx(vec![named_import("models", "alpha.models", Some("M"))], None);
    let segs = vec![type_access("M.factories.Factory"), member("new")];
    let profile = qualified_import_profile();

    assert_eq!(
        drive_with_profile(&lookup, segs, &fc, &profile).expect("aliased import bind"),
        44
    );
}

#[test]
fn unmatched_qualified_root_falls_through_but_matched_unlinked_import_declines() {
    let lookup = Lookup::new()
        .with(sym(
            45,
            "Item",
            "models.Item",
            "class",
            "src/models/Item.gen",
        ))
        .with_member_id(
            45,
            sym(
                46,
                "load",
                "models.Item.load",
                "method",
                "src/models/Item.gen",
            ),
        );
    let segs = vec![type_access("models.Item"), member("load")];
    let profile = qualified_import_profile();

    assert_eq!(
        drive_with_profile(&lookup, segs.clone(), &file_ctx(vec![], None), &profile)
            .expect("a non-imported qualified root may be a namespace path"),
        46
    );
    let wrong_import = file_ctx(vec![named_import("models", "other", None)], None);
    assert!(drive_with_profile(&lookup, segs, &wrong_import, &profile).is_err());
}

/// The symbol id the chain binds, or `None` when it stays unresolved.
fn bind(lookup: &Lookup, segs: Vec<ChainSegment>, fc: &FileContext) -> Option<i64> {
    drive(lookup, segs, fc).ok()
}

/// The recorded cause on the failure path.
fn bind_cause(lookup: &Lookup, segs: Vec<ChainSegment>, fc: &FileContext) -> Option<Cause> {
    drive(lookup, segs, fc).err().flatten()
}

fn drive(lookup: &Lookup, segs: Vec<ChainSegment>, fc: &FileContext) -> Result<i64, Option<Cause>> {
    drive_with_profile(lookup, segs, fc, &DEFAULT_PROFILE)
}

fn drive_with_profile(
    lookup: &Lookup,
    segs: Vec<ChainSegment>,
    fc: &FileContext,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> Result<i64, Option<Cause>> {
    let leaf = segs.last().unwrap().name.clone();
    let mut r = call_ref(&leaf);
    r.chain = Some(MemberChain { segments: segs });
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(&rc, fc, lookup, profile).map(|res| res.target_symbol_id)
}

/// `System.Console.WriteLine(...)`: the root segment names a NAMESPACE, so no
/// local, annotation, import, or by-name arm can type it. The two leading
/// segments join to an indexed type, and the walk's first member step is the
/// third segment.
#[test]
fn namespace_root_anchors_on_the_indexed_type_prefix() {
    let lookup = Lookup::new()
        .with(sym(1, "Console", "System.Console", "class", "ext:dotnet:corelib.cs"))
        .with_member_id(
            1,
            sym(2, "WriteLine", "System.Console.WriteLine", "method", "ext:dotnet:corelib.cs"),
        );
    let segs = vec![seg("System"), seg("Console"), member("WriteLine")];

    assert_eq!(bind(&lookup, segs, &file_ctx(vec![], None)), Some(2));
}

/// A three-segment namespace path anchors on the whole prefix: the LONGEST
/// leading run that names a type wins, so the walk starts on `Alpha.Beta.Widget`
/// rather than on any shorter prefix.
#[test]
fn three_segment_namespace_prefix_anchors_on_the_longest_run() {
    let lookup = Lookup::new()
        .with(sym(1, "Widget", "Alpha.Beta.Widget", "class", "src/widget.cs"))
        .with_member_id(1, sym(2, "Spin", "Alpha.Beta.Widget.Spin", "method", "src/widget.cs"));
    let segs = vec![seg("Alpha"), seg("Beta"), seg("Widget"), member("Spin")];

    assert_eq!(bind(&lookup, segs, &file_ctx(vec![], None)), Some(2));
}

/// A declaration indexed under its FULL qualified name is invisible to the
/// by-name root arms, so `Console.WriteLine(...)` under an open `System`
/// namespace anchors by qualifying the single root segment with that namespace
/// — the same qualification the wildcard-import rung applies to a bare target.
#[test]
fn open_namespace_qualifies_a_single_segment_root() {
    let lookup = Lookup::new()
        .with(sym(1, "System.Console", "System.Console", "class", "ext:dotnet:corelib.cs"))
        .with_member_id(
            1,
            sym(2, "WriteLine", "System.Console.WriteLine", "method", "ext:dotnet:corelib.cs"),
        );
    let fc = file_ctx(vec![wildcard_import("System")], None);
    let segs = vec![seg("Console"), member("WriteLine")];

    assert_eq!(bind(&lookup, segs, &fc), Some(2));
}

/// A leading run that names nothing the index holds stays unresolved — but it
/// carries a classified cause rather than dying causeless, so the ref stays
/// attributable. A plain-identifier root absent from imports, enclosing
/// scopes, the external index, and the project is a value expression whose
/// type never reached the walk: `UntypedRoot`, not a missing name.
#[test]
fn unanchorable_identifier_root_is_an_untyped_root() {
    let lookup = Lookup::new().with(sym(1, "Widget", "Alpha.Widget", "class", "src/widget.cs"));
    let segs = vec![seg("nowhere"), seg("missing"), member("call")];
    let fc = file_ctx(vec![], None);

    assert_eq!(bind(&lookup, segs.clone(), &fc), None);
    let cause = bind_cause(&lookup, segs, &fc).expect("an unanchorable root must carry a cause");
    assert_eq!(cause.kind, CauseKind::UntypedRoot);
    assert_eq!(cause.symbol_id, None);
}

/// The same run rooted on a segment the extractor marked as a namespace
/// qualifier names a declaration path, so its death is a bare-name death:
/// nothing by that name anywhere is `NameUnknown`.
#[test]
fn unanchorable_namespace_root_is_a_name_death() {
    let lookup = Lookup::new().with(sym(1, "Widget", "Alpha.Widget", "class", "src/widget.cs"));
    let segs = vec![
        segment("Nowhere", SegmentKind::NamespaceAccess, false),
        segment("Missing", SegmentKind::NamespaceAccess, false),
        member("Call"),
    ];
    let fc = file_ctx(vec![], None);

    assert_eq!(bind(&lookup, segs.clone(), &fc), None);
    let cause = bind_cause(&lookup, segs, &fc).expect("an unanchorable root must carry a cause");
    assert_eq!(cause.kind, CauseKind::NameUnknown);
}

/// An untyped root whose name IS imported dies with the import: the evidence
/// probes run for a chain root exactly as they do for a bare name.
#[test]
fn untyped_root_bound_by_an_import_blames_the_import() {
    let lookup = Lookup::new();
    let segs = vec![seg("client"), member("get")];
    let fc = file_ctx(vec![import("client", Some("./client"))], None);

    let cause = bind_cause(&lookup, segs, &fc).expect("an unanchorable root must carry a cause");
    assert_eq!(cause.kind, CauseKind::ImportUnlinked);
}

/// Go package aliases bind the qualified call root even when their imported
/// package has no materialized symbol surface.
#[test]
fn unlinked_go_package_alias_roots_blame_the_import() {
    let lookup = Lookup::new();
    let fc = file_ctx(
        vec![import("utilsstrings", None), import("clientpkg", None)],
        None,
    );

    for (root, method_name) in [("utilsstrings", "ToLower"), ("clientpkg", "New")] {
        let cause = drive_with_profile(
            &lookup,
            vec![seg(root), member(method_name)],
            &fc,
            &GO_PROFILE,
        )
        .err()
        .flatten()
        .expect("an unlinked package alias must carry an import cause");
        assert_eq!(
            cause.kind,
            CauseKind::ImportUnlinked,
            "{root}.{method_name}"
        );
    }
}

/// A bare type name still roots on segment 0 and steps to segment 1 — the
/// anchor runs only after the ordinary root arms have all missed.
#[test]
fn bare_type_root_still_starts_at_the_second_segment() {
    let lookup = Lookup::new()
        .with(sym(1, "Widget", "Widget", "class", "src/widget.cs"))
        .with_member_id(1, sym(2, "Spin", "Widget.Spin", "method", "src/widget.cs"));
    let segs = vec![seg("Widget"), member("Spin")];

    assert_eq!(bind(&lookup, segs, &file_ctx(vec![], None)), Some(2));
}
