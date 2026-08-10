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
use crate::types::{ChainSegment, MemberChain};

use super::cause::{Cause, CauseKind};
use super::chain::{resolve_root, Receiver};
use super::support::is_type_kind;

/// Upper bound on how many leading segments a single anchor may consume. Caps
/// the probe count per chain; a namespace path deeper than this carries no
/// additional evidence the shorter prefixes do not already offer.
const MAX_ANCHOR_SEGMENTS: usize = 4;

/// The receiver a chain walk starts on, paired with the segment index it starts
/// at. The ordinary root consumes segment 0 and hands back `1`; a namespace-
/// anchored root consumes the leading segments its type prefix spans.
///
/// `Err` carries the root's own diagnosable cause when it had one, and an
/// `UnboundRoot` cause otherwise — a chain whose leading segments name nothing
/// the index holds has exhausted every way to be typed, which is a cause, not
/// an absence of one.
pub(super) fn anchor(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
    chain: &MemberChain,
) -> Result<(Receiver, usize), Option<Cause>> {
    let cause = match resolve_root(ref_ctx, file_ctx, lookup, arena, &chain.segments[0]) {
        Ok(root) => return Ok((root, 1)),
        Err(cause) => cause,
    };
    let anchored = namespace_anchor(
        file_ctx,
        lookup,
        arena,
        profile,
        ref_ctx.file_package_id,
        &chain.segments,
    );
    match anchored {
        Some((recv, consumed)) => {
            crate::tracef!(
                "  ROOT ANCHOR: {} leading segment(s) -> {}",
                consumed,
                arena.format_type(recv.ty),
            );
            Ok((recv, consumed))
        }
        None => Err(cause.or(Some(Cause::new(None, CauseKind::UnboundRoot)))),
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
        if k > 1 {
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
    Some(Receiver::new(arena.class(&sym.qualified_name), sym.id))
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
    if profile.namespace_imports_are_wildcards {
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
