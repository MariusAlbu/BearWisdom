// =============================================================================
// engine/type_mention_demand — demand pulls sourced from type MENTIONS
//
// A ref names a symbol; a type mention names a TYPE without ever becoming a
// ref. A callable's return type sits in its signature, a delegate-wrapped
// parameter's type argument seeds a caller's lambda binding, a member chain's
// root carries the type its receiver was declared as. None of the three emits
// an edge, so the ref-following collector never reaches the declaring file and
// the walk dies one hop later on a type that was never materialized.
//
// Each collector below reduces its mention to a bare type head and routes it
// through `demand_type_head`, which pulls only heads the tree does not already
// hold — so an already-indexed type, internal or external, costs nothing.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::languages::LanguagePlugin;
use crate::type_checker::profile::language_profile::LanguageProfile;

/// Pull the files that DEFINE a materialized external file's callables' RETURN
/// types. A method's return type is captured from its signature, not emitted as a
/// TypeRef edge, so the ref-following collector misses it — yet `db.delete(t)`
/// yields `PgDeleteBase`, whose member `.where(...)` the chain then walks. This is
/// the same next-hop reachability as following an import, sourced from the
/// signature: pull only an un-materialized, externally-defined head, so a return
/// type already indexed (internal or pulled) adds nothing. Signature shape is
/// per-language (TS/.NET `):`, Rust/Python `->`, Go's separator-less trailing
/// result) — dispatched through the owning language plugin, the same adapter
/// `populate_return_type_ids` uses for every language's own symbols.
pub(super) fn collect_return_type_files(
    symbols: &[crate::types::ExtractedSymbol],
    lang: &str,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let Some(profile) = profiles.get(lang) else {
        return;
    };
    for s in symbols {
        let Some(sig) = s.signature.as_deref() else {
            continue;
        };
        let Some(ret) = crate::languages::default_registry()
            .get(lang)
            .signature_return_type(sig)
            .or_else(|| crate::ecosystem::signature::return_type_for_language(lang, sig))
        else {
            continue;
        };
        let (head, _args) = crate::languages::signature_type_application(lang, &ret);
        demand_type_head(&head, profile, tree, loc, seen, out);
    }
}

/// Pull the files defining the types a materialized external callable's
/// CALLBACK parameters name. A signature `CreateTable(string,
/// Action<ColumnsBuilder>)` seeds a caller's lambda parameter as
/// `ColumnsBuilder` — the seed types the local from the signature STRING, so
/// nothing else ever demands the type and every member on the lambda
/// parameter misses. Scoped to the profile's delegate wrappers: those are
/// exactly the parameter positions whose type arguments become caller-local
/// bindings.
pub(super) fn collect_callback_param_type_files(
    symbols: &[crate::types::ExtractedSymbol],
    lang: &str,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let Some(profile) = profiles.get(lang) else {
        return;
    };
    for symbol in symbols {
        let Some(signature) = symbol.signature.as_deref() else {
            continue;
        };
        for argument in crate::languages::default_registry()
            .get(lang)
            .signature_delegate_argument_types(signature)
        {
            let (head, _args) = crate::languages::signature_type_application(lang, &argument);
            demand_type_head(&head, profile, tree, loc, seen, out);
        }
    }
}
/// Pull the files that DEFINE the type a member chain's ROOT segment was
/// declared as. A root's `declared_type` is stamped at extract time — from a
/// type annotation, or from a language keyword aliased to the library type it
/// names (`string` → `System.String`) — and is never a ref target, so nothing
/// else demands it and every member walked off that root misses. The declared
/// text is reduced to its bare head, so an applied generic demands the head
/// its members are declared on.
///
/// Bounded to a QUALIFIED head. A bare head (`Result`, `Task`, `Options`) is
/// high cardinality: the location index offers one entry per module that
/// happens to declare the name, so a bare-leaf pull fans the frontier out over
/// modules the root has no evidence of naming. A qualified head names exactly
/// one declaration, and `demand_type_head` picks the offering entry by it.
///
/// Seed-pass only. Internal roots are the evidence this collector runs on and
/// they form a bounded frontier — one pass over the project's own refs. A
/// materialized external file's chains need no equivalent pass: the types its
/// members walk onto are already reached as return-type and callback-parameter
/// next hops.
pub(super) fn collect_chain_root_type_files(
    refs: &[crate::types::ExtractedRef],
    lang: &str,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let Some(profile) = profiles.get(lang) else {
        return;
    };
    for r in refs {
        let Some(root) = r.chain.as_ref().and_then(|c| c.segments.first()) else {
            continue;
        };
        let Some(declared) = root.declared_type.as_deref() else {
            continue;
        };
        let (head, _args) = crate::languages::signature_type_application(lang, declared);
        if type_leaf(&head, profile) == head {
            continue;
        }
        demand_type_head(&head, profile, tree, loc, seen, out);
    }
}

/// Demand-pull the file(s) defining `head` when the tree does not already
/// hold it. The lookup key is the bare declared name, split with the active
/// profile's qualified-name separator, while the location index keys locations
/// by leaf name. The already-indexed guard compares the identity the chain walker
/// will look the type up by: for a QUALIFIED head the bare-leaf check is
/// wrong in both directions — a same-named type from an unrelated module
/// satisfies `by_name` and suppresses the pull of the one the signature
/// names, while the walk needs the qualified type and misses. An unqualified
/// head keeps the name check.
///
/// The PULL is qualified-aware the same way: when any offering entry's path
/// addresses the qualified name, only those entries are pulled — one
/// declaration, one file, instead of every module declaring the leaf. Entries
/// keyed by an on-disk source path can never carry a qualified name, so a head
/// with no addressable entry falls back to the full leaf offering.
fn demand_type_head(
    head: &str,
    profile: &LanguageProfile,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let leaf = type_leaf(head, profile);
    if leaf.is_empty() {
        return;
    }
    let already_indexed = if leaf == head {
        !tree.by_name(leaf).is_empty()
    } else {
        tree.by_qualified_name(head).is_some()
    };
    if already_indexed {
        return;
    }
    let offered = loc.find_by_name(leaf);
    // A qualified head is addressable only when at least one entry carries the
    // qualified name; that entry set then replaces the leaf offering.
    let addressable = leaf != head
        && offered
            .iter()
            .any(|(_module, file)| path_addresses_type(file, head));
    for (_module, file) in offered {
        if addressable && !path_addresses_type(file, head) {
            continue;
        }
        let file = file.to_path_buf();
        if seen.insert(file.clone()) {
            out.push(file.clone());
        }
    }
}

/// The bare declared name of a type head under the active profile's
/// qualified-name separator.
fn type_leaf<'a>(head: &'a str, profile: &LanguageProfile) -> &'a str {
    type_leaf_for_separator(head, profile.qname_separator)
}

fn type_leaf_for_separator<'a>(head: &'a str, separator: &str) -> &'a str {
    (!separator.is_empty())
        .then(|| head.rsplit(separator).next().unwrap_or(head))
        .unwrap_or(head)
}

/// True when an offering entry's path ADDRESSES `qualified` — a metadata
/// virtual path carries the type's qualified name as its trailing component
/// (`…dll!!Assembly!!System.String`). Matched on a separator boundary so a
/// longer sibling name (`My.System.String`) cannot satisfy a shorter one.
fn path_addresses_type(file: &Path, qualified: &str) -> bool {
    let path = file.to_string_lossy().replace('\\', "/");
    match path.strip_suffix(qualified) {
        Some(prefix) => prefix.is_empty() || prefix.ends_with(|c| matches!(c, '!' | '/' | ':')),
        None => false,
    }
}

#[cfg(test)]
#[path = "type_mention_demand_tests.rs"]
mod tests;
