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
    call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
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

/// A `using NS;` entry — the form `build_file_context` produces for a plain
/// namespace import under a profile that treats one as a wildcard.
fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: "*".to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
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
    let leaf = segs.last().unwrap().name.clone();
    let mut r = call_ref(&leaf);
    r.chain = Some(MemberChain { segments: segs });
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(&rc, fc, lookup, &DEFAULT_PROFILE).map(|res| res.target_symbol_id)
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
/// carries a classified unbound cause rather than dying causeless, so the ref
/// stays attributable. A root name absent from imports, enclosing scopes, the
/// external index, and the project classifies as `NameUnknown`.
#[test]
fn unanchorable_root_carries_a_classified_unbound_cause() {
    let lookup = Lookup::new().with(sym(1, "Widget", "Alpha.Widget", "class", "src/widget.cs"));
    let segs = vec![seg("Nowhere"), seg("Missing"), member("Call")];
    let fc = file_ctx(vec![], None);

    assert_eq!(bind(&lookup, segs.clone(), &fc), None);
    let cause = bind_cause(&lookup, segs, &fc).expect("an unanchorable root must carry a cause");
    assert_eq!(cause.kind, CauseKind::NameUnknown);
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
