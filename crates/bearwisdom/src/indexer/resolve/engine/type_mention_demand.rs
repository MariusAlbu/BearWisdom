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
use crate::indexer::resolve::engine::contract::chain_walker::{
    parse_return_type_from_signature_for_lang, parse_type_head_and_args,
};
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::profile::language_profile::LanguageProfile;

/// Pull the files that DEFINE a materialized external file's callables' RETURN
/// types. A method's return type is captured from its signature, not emitted as a
/// TypeRef edge, so the ref-following collector misses it — yet `db.delete(t)`
/// yields `PgDeleteBase`, whose member `.where(...)` the chain then walks. This is
/// the same next-hop reachability as following an import, sourced from the
/// signature: pull only an un-materialized, externally-defined head, so a return
/// type already indexed (internal or pulled) adds nothing. Signature shape is
/// per-language (TS/.NET `):`, Rust/Python `->`, Go's separator-less trailing
/// result) — dispatched through `parse_return_type_from_signature_for_lang`, the
/// same parser `populate_return_type_ids` uses for every language's own symbols.
pub(super) fn collect_return_type_files(
    symbols: &[crate::types::ExtractedSymbol],
    lang: &str,
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    for s in symbols {
        let Some(sig) = s.signature.as_deref() else {
            continue;
        };
        let Some(ret) = parse_return_type_from_signature_for_lang(sig, lang) else {
            continue;
        };
        let (head, _args) = parse_type_head_and_args(&ret);
        demand_type_head(head, tree, loc, seen, out);
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
    if profile.delegate_wrappers.is_empty() {
        return;
    }
    for s in symbols {
        let Some(sig) = s.signature.as_deref() else {
            continue;
        };
        for (wrapper, _shape) in profile.delegate_wrappers {
            let mut search_from = 0;
            while let Some(pos) = sig[search_from..].find(wrapper) {
                let abs = search_from + pos;
                search_from = abs + wrapper.len();
                // Require an applied form `Wrapper<...>` at a token boundary so
                // `Func` doesn't match inside `FuncFactory`.
                let boundary_ok = abs == 0
                    || !sig.as_bytes()[abs - 1].is_ascii_alphanumeric()
                        && sig.as_bytes()[abs - 1] != b'_';
                if !boundary_ok || !sig[search_from..].starts_with('<') {
                    continue;
                }
                let Some(args) = balanced_generic_args(&sig[search_from..]) else {
                    continue;
                };
                for arg in split_top_level_commas(args) {
                    let (head, _args) = parse_type_head_and_args(arg.trim());
                    demand_type_head(head, tree, loc, seen, out);
                }
            }
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
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    for r in refs {
        let Some(root) = r.chain.as_ref().and_then(|c| c.segments.first()) else {
            continue;
        };
        let Some(declared) = root.declared_type.as_deref() else {
            continue;
        };
        let (head, _args) = parse_type_head_and_args(declared);
        if type_leaf(head) == head {
            continue;
        }
        demand_type_head(head, tree, loc, seen, out);
    }
}

/// The text between an applied generic's outermost angle brackets, balanced:
/// for `<A, Func<B, C>>rest` returns `A, Func<B, C>`.
fn balanced_generic_args(s: &str) -> Option<&str> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a generic-argument list on commas at nesting depth zero.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Demand-pull the file(s) defining `head` when the tree does not already
/// hold it. The lookup key is the bare declared name — TS/namespace paths
/// join segments with `.` (`ns.Type`), Rust/C++ paths with `::`
/// (`gadgetcrate::Gadget`) — while the location index keys locations by leaf
/// name. The already-indexed guard compares the identity the chain walker
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
    tree: &Compilation,
    loc: &SymbolLocationIndex,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let leaf = type_leaf(head);
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

/// The bare declared name of a type head, under either qualified-name
/// separator: `gadgetcrate::Gadget` → `Gadget`, `System.String` → `String`.
fn type_leaf(head: &str) -> &str {
    let leaf = head.rsplit("::").next().unwrap_or(head);
    leaf.rsplit('.').next().unwrap_or(leaf)
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
