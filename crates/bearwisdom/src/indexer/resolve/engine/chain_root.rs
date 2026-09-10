// =============================================================================
// engine/chain_root — where a member chain's walk begins
//
// The walk needs two things before its first member step: the receiver the
// chain's leading segments evaluate to, and the index of the first segment
// that receiver did NOT consume. A value or type root consumes exactly one
// segment. A root naming a NAMESPACE consumes several — `System.Console.Write`
// has no `System` to type, but `System.Console` is an indexed type, so the walk
// anchors there and its first member step is the third segment.
//
// Anchoring is index-driven: leading segments joined with the profile's
// qualified-name separator and probed against the qualified-name map, each
// probe repeated under the namespaces the file opens without qualification.
// No namespace is ever named here — the profile supplies a separator, the
// index supplies the declarations.
// =============================================================================

use crate::indexer::resolve::engine::contract::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{ChainSegment, MemberChain, SegmentKind};

use super::cause::Cause;
use super::chain::{resolve_root, Receiver};
use super::kinds::is_type_kind;

/// Upper bound on how many leading segments a single anchor may consume. Caps
/// the probe count per chain; a namespace path deeper than this carries no
/// additional evidence the shorter prefixes do not already offer.
const MAX_ANCHOR_SEGMENTS: usize = 4;

/// The receiver a chain walk starts on, paired with the segment index it starts
/// at. The ordinary root consumes segment 0 and hands back `1`; a namespace-
/// anchored root consumes the leading segments its type prefix spans.
///
/// `Err` carries the root's own diagnosable cause when it had one, and a
/// classified root cause otherwise (see `root_cause`) — a chain whose leading
/// segments name nothing the index holds has exhausted every way to be typed,
/// which is a cause, not an absence of one.
pub(super) fn anchor(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
    chain: &MemberChain,
) -> Result<(Receiver, usize), Option<Cause>> {
    let cause = match resolve_root(
        ref_ctx,
        file_ctx,
        lookup,
        arena,
        profile,
        &chain.segments[0],
    ) {
        Ok(root) => return Ok((root, 1)),
        Err(cause) => cause,
    };
    if chain.segments[0].kind == SegmentKind::BaseRef
        || lookup
            .local_reference(ref_ctx.extracted_ref.byte_offset)
            .is_some()
    {
        return Err(cause);
    }
    // Some extractors preserve a source-qualified static receiver in one
    // TypeAccess segment. Under import qualification, its first component is
    // meaningful only through an explicit named binding; do not let the
    // wildcard namespace anchor reinterpret it when that proof is absent.
    let anchored = if split_import_qualified_root(profile, &chain.segments[0]).is_some() {
        match imported_qualified_root_anchor(file_ctx, lookup, arena, profile, &chain.segments) {
            QualifiedRootAnchor::Bound(receiver) => Some((receiver, 1)),
            QualifiedRootAnchor::NotImported => namespace_anchor(
                file_ctx,
                lookup,
                arena,
                profile,
                ref_ctx.file_package_id,
                &chain.segments,
            ),
            QualifiedRootAnchor::Declined => None,
        }
    } else {
        namespace_anchor(
            file_ctx,
            lookup,
            arena,
            profile,
            ref_ctx.file_package_id,
            &chain.segments,
        )
    };
    match anchored {
        Some((recv, consumed)) => {
            crate::tracef!(
                "  ROOT ANCHOR: {} leading segment(s) -> {}",
                consumed,
                arena.format_type(recv.ty),
            );
            Ok((recv, consumed))
        }
        None => {
            Err(cause.or_else(|| Some(root_cause(ref_ctx, file_ctx, lookup, &chain.segments[0]))))
        }
    }
}

/// Split a source-qualified TypeAccess root into its locally bound head and
/// remaining type path. The profile supplies the source separator; the import
/// table must later prove what the bound head denotes.
fn split_import_qualified_root<'a>(
    profile: &LanguageProfile,
    root: &'a ChainSegment,
) -> Option<(&'a str, &'a str)> {
    if profile
        .chain_qualification
        .qualified_import_root()
        .is_none()
        || root.kind != SegmentKind::TypeAccess
        || root.is_call
        || profile.qname_separator.is_empty()
    {
        return None;
    }
    let (binding, qualified_tail) = root.name.split_once(profile.qname_separator)?;
    if binding.is_empty()
        || qualified_tail.is_empty()
        || qualified_tail
            .split(profile.qname_separator)
            .any(|part| part.is_empty())
    {
        return None;
    }
    Some((binding, qualified_tail))
}

/// Root a source-qualified TypeAccess through the file's explicit import
/// table. The binding component matches the import's local name, while exact
/// index probes use its original declared name.
///
/// A duplicate local binding that reaches two distinct indexed types is
/// ambiguous and declines rather than choosing the first import row.
enum QualifiedRootAnchor {
    NotImported,
    Bound(Receiver),
    Declined,
}

fn imported_qualified_root_anchor(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
    segments: &[ChainSegment],
) -> QualifiedRootAnchor {
    let Some((binding, qualified_tail)) = segments
        .first()
        .and_then(|root| split_import_qualified_root(profile, root))
    else {
        return QualifiedRootAnchor::NotImported;
    };
    let Some(config) = profile.chain_qualification.qualified_import_root() else {
        return QualifiedRootAnchor::NotImported;
    };
    let mut matched: Option<(String, Receiver)> = None;
    let mut imported = false;

    for import in &file_ctx.imports {
        if import.is_wildcard
            || import.binding_kind != Some(SegmentKind::TypeAccess)
            || import.bound_name() != binding
            || import.imported_name.is_empty()
        {
            continue;
        }
        imported = true;
        let Some(module) = import
            .module_path
            .as_deref()
            .filter(|module| !module.is_empty())
        else {
            continue;
        };
        for qname in (config.type_candidates)(module, &import.imported_name, qualified_tail) {
            let Some(receiver) = type_receiver(lookup, arena, &qname) else {
                continue;
            };
            if let Some((previous_qname, _)) = &matched {
                if previous_qname != &qname {
                    return QualifiedRootAnchor::Declined;
                }
                continue;
            }
            matched = Some((qname, receiver));
        }
    }

    match matched {
        Some((_, receiver)) => QualifiedRootAnchor::Bound(receiver),
        None if imported => QualifiedRootAnchor::Declined,
        None => QualifiedRootAnchor::NotImported,
    }
}

/// The cause recorded for a root no arm could type. A root the extractor
/// marked as a declaration path — a namespace qualifier, a type's static
/// access, a construction — dies as a bare name does: an unlinked import, an
/// unreachable or absent declaration. Any other root is a value expression
/// whose type was never captured, and is classified as such rather than as a
/// name the project lacks.
fn root_cause(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    root: &ChainSegment,
) -> Cause {
    match root.kind {
        SegmentKind::NamespaceAccess | SegmentKind::TypeAccess | SegmentKind::Construction => {
            super::unbound_cause::classify_unbound_root(
                &root.name,
                &ref_ctx.scope_chain,
                file_ctx,
                lookup,
                ref_ctx.file_package_id,
            )
        }
        _ => super::unbound_cause::classify_untyped_root(
            &root.name,
            &ref_ctx.scope_chain,
            file_ctx,
            lookup,
            ref_ctx.file_package_id,
        ),
    }
}

/// Root the walk on the LONGEST leading-segment prefix the index holds as a
/// type, with the number of segments that prefix consumed.
///
/// Each prefix is probed as written and once per namespace the file opens
/// without qualification, so a chain written under an open namespace anchors on
/// the same evidence a sibling bare type reference binds by. A single bare
/// segment is probed only in its qualified forms — `resolve_root` already
/// exhausted the by-name arms for it. At least one segment is always left for
/// the member walk, so a chain whose entire text is one qualified type name is
/// declined here; the bare ladder owns that shape.
fn namespace_anchor(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
    package_id: Option<i64>,
    segments: &[ChainSegment],
) -> Option<(Receiver, usize)> {
    let sep = profile.qname_separator;
    let max = MAX_ANCHOR_SEGMENTS.min(segments.len().checked_sub(1)?);
    let open = open_namespaces(file_ctx, lookup, profile, package_id);
    for k in (1..=max).rev() {
        // A called segment is a VALUE step (`factory().Member`), never part of
        // the qualified name of a declaration.
        if segments[..k].iter().any(|s| s.is_call) {
            continue;
        }
        let joined = join_segments(&segments[..k], sep);
        if k > 1 || (!sep.is_empty() && joined.contains(sep)) {
            if let Some(recv) = type_receiver(lookup, arena, &joined) {
                return Some((recv, k));
            }
        }
        for ns in &open {
            if let Some(recv) = type_receiver(lookup, arena, &format!("{ns}{sep}{joined}")) {
                return Some((recv, k));
            }
        }
    }
    None
}

/// The segment names joined with the profile's qualified-name separator.
fn join_segments(segments: &[ChainSegment], sep: &str) -> String {
    segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(sep)
}

/// The receiver `qname` names, when the index holds a TYPE declaration under
/// it. A namespace, function, or value row carries no member set a walk can
/// step off, so only a type-kind declaration anchors.
fn type_receiver(lookup: &dyn SymbolLookup, arena: &TypeArena, qname: &str) -> Option<Receiver> {
    let sym = lookup
        .by_qualified_name(qname)
        .filter(|s| is_type_kind(&s.kind))?;
    Some(Receiver::new(
        super::head_decl::nominal_head(lookup, arena, sym),
        sym.id,
    ))
}

/// The namespaces this file opens WITHOUT qualification: its wildcard imports,
/// plus the manifest-declared implicit namespaces when the profile treats a
/// plain namespace import as a wildcard. The same set the wildcard-import rung
/// qualifies a bare target against, so a chain root and a sibling type
/// reference on the same line resolve through one scope.
fn open_namespaces(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
    package_id: Option<i64>,
) -> Vec<String> {
    let mut out: Vec<String> = file_ctx
        .imports
        .iter()
        .filter(|imp| imp.is_wildcard)
        .filter_map(|imp| imp.module_path.clone())
        .filter(|m| !m.is_empty())
        .collect();
    if profile.imports.namespace_imports_are_wildcards {
        out.extend(
            lookup
                .implicit_wildcard_namespaces(package_id)
                .iter()
                .cloned(),
        );
    }
    out
}

#[cfg(test)]
#[path = "chain_root_tests.rs"]
mod tests;
