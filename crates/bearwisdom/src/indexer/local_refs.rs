// =============================================================================
// indexer/local_refs.rs  —  locals.scm-based ref filtering
//
// Runs the per-language `locals.scm` tree-sitter query to identify
// identifiers that are resolved within the file (local variables,
// parameters, etc.) and drops the corresponding `ExtractedRef`s so
// they never enter the cross-file resolution loop.
// =============================================================================

use std::sync::{Arc, Mutex, OnceLock};

use crate::parser::local_resolver::LocalResolver;

/// Process-wide cache of compiled `locals.scm` resolvers. `LocalResolver::new`
/// compiles a tree-sitter query and classifies its capture indices — work
/// repeated verbatim for every file of a language. Keyed by `(grammar,
/// locals-source-pointer)`: the grammar because one locals source can serve
/// several grammars (the TS source drives the `.ts` and `.tsx` grammars), the
/// source pointer because each `locals_scm_for_language` result is a `&'static`
/// literal. A `None` (empty / malformed / too few captures) is cached too, so a
/// language without a usable locals.scm doesn't re-attempt the compile per file.
/// Shared as `Arc`; `resolve` builds its own `QueryCursor`, so concurrent use
/// across the parallel connectors is safe.
fn cached_local_resolver(
    grammar: &tree_sitter::Language,
    locals_scm: &'static str,
) -> Option<Arc<LocalResolver>> {
    static CACHE: OnceLock<
        Mutex<rustc_hash::FxHashMap<(tree_sitter::Language, usize), Option<Arc<LocalResolver>>>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(rustc_hash::FxHashMap::default()));
    let key = (grammar.clone(), locals_scm.as_ptr() as usize);
    if let Some(entry) = cache.lock().unwrap().get(&key) {
        return entry.clone();
    }
    let resolver = LocalResolver::new(locals_scm, grammar.clone()).map(Arc::new);
    cache.lock().unwrap().insert(key, resolver.clone());
    resolver
}

// ---------------------------------------------------------------------------
// Local scope resolution — filters out intra-scope refs via locals.scm
// ---------------------------------------------------------------------------

pub(super) fn filter_local_refs(
    source: &str,
    lang_id: &str,
    plugin: &dyn crate::languages::LanguagePlugin,
    symbols: &[crate::types::ExtractedSymbol],
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    let _ = symbols;
    let Some((resolver, grammar)) = local_resolver_for(lang_id, plugin) else {
        return;
    };

    // Parse the file with tree-sitter (fast — typically <1ms).
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return;
    }
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return;
    };

    apply_local_resolution(&resolver, lang_id, source, &tree, refs);
}

/// Like `filter_local_refs`, but reuses a tree the caller already parsed for this
/// source + grammar — the indexer shares one parse across locals.scm filtering
/// and flow typing.
pub(super) fn filter_local_refs_with_tree(
    source: &str,
    lang_id: &str,
    plugin: &dyn crate::languages::LanguagePlugin,
    refs: &mut Vec<crate::types::ExtractedRef>,
    tree: &tree_sitter::Tree,
) {
    let Some((resolver, _grammar)) = local_resolver_for(lang_id, plugin) else {
        return;
    };
    apply_local_resolution(&resolver, lang_id, source, tree, refs);
}

/// The locals.scm resolver + grammar for a language, or None when the language
/// has no usable locals.scm (so the caller skips local filtering entirely).
fn local_resolver_for(
    lang_id: &str,
    plugin: &dyn crate::languages::LanguagePlugin,
) -> Option<(Arc<LocalResolver>, tree_sitter::Language)> {
    let locals_scm = crate::indexer::query_builtins::locals_scm_for_language(lang_id)?;
    let grammar = plugin.grammar(lang_id)?;
    let resolver = cached_local_resolver(&grammar, locals_scm)?;
    Some((resolver, grammar))
}

/// Run local resolution against `tree` and drop the refs it resolves to a
/// same-file definition. Shared by the parsing and tree-reusing entry points.
fn apply_local_resolution(
    resolver: &LocalResolver,
    lang_id: &str,
    source: &str,
    tree: &tree_sitter::Tree,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    let resolution = resolver.resolve(tree, source.as_bytes());

    if resolution.resolved_count() == 0 {
        return;
    }

    // Filter out refs whose source position falls on a locally-resolved identifier.
    // We match by (line, name) since ExtractedRef stores a 0-based line number.
    // Build a set of (line, name) pairs that are locally resolved.
    let local_names_by_line = {
        let mut set = rustc_hash::FxHashSet::default();
        let line_offsets: Vec<usize> = std::iter::once(0)
            .chain(source.bytes().enumerate().filter_map(|(i, b)| {
                if b == b'\n' { Some(i + 1) } else { None }
            }))
            .collect();

        for &byte_offset in &resolution.locally_resolved {
            if byte_offset >= source.len() {
                continue;
            }
            // Convert byte offset to 0-based line number.
            // partition_point returns count of line starts <= byte_offset (1-indexed);
            // saturating_sub to get 0-based line matching ExtractedRef.line.
            let line = (line_offsets.partition_point(|&off| off <= byte_offset) as u32).saturating_sub(1);
            // Extract the identifier name at this offset.
            let end = source[byte_offset..]
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map(|i| byte_offset + i)
                .unwrap_or(source.len());
            let name = &source[byte_offset..end];
            if !name.is_empty() {
                set.insert((line, name.to_string()));
            }
        }
        set
    };

    let before = refs.len();
    refs.retain(|r| {
        // Keep refs that have a module (imports) — those are never local.
        if r.module.is_some() {
            return true;
        }
        // Keep refs that have a chain — member access is cross-scope.
        if r.chain.is_some() {
            return true;
        }
        // Keep type refs — they reference types/classes, not local variables.
        if matches!(
            r.kind,
            crate::types::EdgeKind::TypeRef
                | crate::types::EdgeKind::Inherits
                | crate::types::EdgeKind::Implements
                | crate::types::EdgeKind::Instantiates
        ) {
            return true;
        }
        // Keep refs to names that start with uppercase — likely types/classes.
        if r.target_name.starts_with(|c: char| c.is_uppercase()) {
            return true;
        }
        // Filter out locally-resolved call refs to lowercase names (variables/params).
        !local_names_by_line.contains(&(r.line, r.target_name.clone()))
    });

    let filtered = before - refs.len();
    if filtered > 0 {
        tracing::debug!(
            lang = lang_id,
            filtered,
            remaining = refs.len(),
            "Filtered locally-resolved refs via locals.scm"
        );
    }
}

/// Operator characters that, alone, make a target a language primitive rather
/// than a symbol reference.
const OPERATOR_CHARS: &str = "+-*/<>=!&|^%~.:?@";

/// Drop `Calls` refs whose target is a punctuation-only operator token,
/// optionally paren-wrapped (F# `(+)` / `(=)`, gleam `<>` / `==`, Scala `::`).
/// Operators are language primitives — `1 + 2`'s `+` has no resolvable target —
/// so emitting them as calls only inflates the unresolved count. Runs
/// unconditionally, unlike `filter_local_refs` which needs a `locals.scm`.
pub(super) fn filter_operator_refs(refs: &mut Vec<crate::types::ExtractedRef>) {
    refs.retain(|r| !is_operator_only_call(r));
}

fn is_operator_only_call(r: &crate::types::ExtractedRef) -> bool {
    if r.kind != crate::types::EdgeKind::Calls || r.module.is_some() || r.chain.is_some() {
        return false;
    }
    let t = r.target_name.trim();
    let inner = t
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(t);
    !inner.is_empty() && inner.chars().all(|c| OPERATOR_CHARS.contains(c))
}

#[cfg(test)]
#[path = "local_refs_tests.rs"]
mod tests;

