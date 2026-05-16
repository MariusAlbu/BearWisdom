// =============================================================================
// indexer/local_refs.rs  —  locals.scm-based ref filtering
//
// Runs the per-language `locals.scm` tree-sitter query to identify
// identifiers that are resolved within the file (local variables,
// parameters, etc.) and drops the corresponding `ExtractedRef`s so
// they never enter the cross-file resolution loop.
// =============================================================================

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
    use crate::parser::local_resolver::LocalResolver;

    // Get the locals.scm query for this language.
    let Some(locals_scm) = crate::indexer::query_builtins::locals_scm_for_language(lang_id)
    else {
        return;
    };

    // Get the grammar to compile the query.
    let Some(grammar) = plugin.grammar(lang_id) else {
        return;
    };

    // Compile the locals query (consumes a clone of grammar).
    let Some(resolver) = LocalResolver::new(locals_scm, grammar.clone()) else {
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

    // Run local resolution.
    let resolution = resolver.resolve(&tree, source.as_bytes());

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

