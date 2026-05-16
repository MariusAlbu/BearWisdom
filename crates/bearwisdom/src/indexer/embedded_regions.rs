// =============================================================================
// indexer/embedded_regions.rs  —  embedded-region splicing
//
// Host extractors (Vue/Svelte/Astro/Razor/HTML/PHP/MDX) emit `EmbeddedRegion`s
// for sub-language code blocks. This module sub-parses each region with the
// declared language's plugin and splices the resulting symbols/refs back
// into the host file's vectors with line/column offsets applied.
// =============================================================================

use crate::languages::LanguageRegistry;

// ---------------------------------------------------------------------------
// Embedded-region dispatch — splices sub-extracted symbols/refs back into the
// host file's vectors with line/column offsets applied.
// ---------------------------------------------------------------------------

/// Dispatch each `EmbeddedRegion` returned by a host extractor (Vue/Svelte/
/// Astro/Razor/HTML/PHP/MDX). For each region we:
///
/// 1. Punch out interpolation holes (only populated for Tier-3 string DSLs).
/// 2. Call the sub-language plugin's `extract()` against the region text.
/// 3. Run locals filtering with the sub-language's grammar+locals.scm.
/// 4. Rebase `source_symbol_index`, `parent_index`, `handler_symbol_index`,
///    and `property_symbol_index` by the current `r.symbols.len()`.
/// 5. Shift line/column positions by the region's offsets (column offset
///    applies only to positions on line 0 of the sub-extraction).
/// 6. Push spliced symbols into `r.symbols` and record `Some(language_id)`
///    in the parallel `origin_langs` vector so `write.rs` can populate the
///    `symbols.origin_language` column.
///
/// S11 supports exactly one level of embedding. Nested embedding (e.g. a
/// SQL string DSL inside a Razor `@{}` C# block) is deferred to S13+.
pub(super) fn dispatch_embedded_regions(
    file_path: &str,
    registry: &LanguageRegistry,
    regions: Vec<crate::types::EmbeddedRegion>,
    r: &mut crate::types::ExtractionResult,
    origin_langs: &mut Vec<Option<String>>,
    from_snippet: &mut Vec<bool>,
    ref_origin_langs: &mut Vec<Option<String>>,
) {
    use crate::types::EmbeddedOrigin;
    for region in regions {
        let sub_plugin = registry.get(&region.language_id);
        let sub_text = if region.holes.is_empty() {
            region.text.clone()
        } else {
            punch_holes(&region.text, &region.holes)
        };

        let mut sub = sub_plugin.extract(&sub_text, file_path, &region.language_id);
        super::local_refs::filter_local_refs(&sub_text, &region.language_id, sub_plugin, &sub.symbols, &mut sub.refs);

        let symbol_offset = r.symbols.len();
        let line_offset = region.line_offset;
        let col_offset = region.col_offset;
        // E3: Markdown fenced code, Rust doctests, and Python doctests are
        // snippet contexts — usually missing imports, so unresolved refs
        // from their symbols should be excluded from aggregate resolution
        // stats. Frontmatter (YAML/TOML/JSON) doesn't qualify.
        let region_is_snippet = matches!(region.origin, EmbeddedOrigin::MarkdownFence);

        for mut sym in sub.symbols {
            let start_on_first = sym.start_line == 0;
            let end_on_first = sym.end_line == 0;
            sym.start_line = sym.start_line.saturating_add(line_offset);
            sym.end_line = sym.end_line.saturating_add(line_offset);
            if start_on_first {
                sym.start_col = sym.start_col.saturating_add(col_offset);
            }
            if end_on_first {
                sym.end_col = sym.end_col.saturating_add(col_offset);
            }
            if let Some(parent) = sym.parent_index.as_mut() {
                *parent += symbol_offset;
            }
            // E1: host-injected wrapper prefix (e.g. Razor's synthetic
            // `__RazorBody` class) is stripped from qualified_name and
            // scope_path so user-facing names don't carry the wrapper.
            if let Some(prefix) = region.strip_scope_prefix.as_deref() {
                strip_scope_prefix_in_place(&mut sym.qualified_name, prefix);
                if let Some(sp) = sym.scope_path.as_mut() {
                    strip_scope_prefix_in_place(sp, prefix);
                }
                if let Some(sp) = sym.scope_path.as_ref() {
                    if sp.is_empty() {
                        sym.scope_path = None;
                    }
                }
            }
            r.symbols.push(sym);
            origin_langs.push(Some(region.language_id.clone()));
            from_snippet.push(region_is_snippet);
        }
        for mut rf in sub.refs {
            rf.source_symbol_index += symbol_offset;
            rf.line = rf.line.saturating_add(line_offset);
            r.refs.push(rf);
            // Tag this ref with the embedded language so the resolver
            // routes it to the correct externals/primitives table instead
            // of the host-file language.
            ref_origin_langs.push(Some(region.language_id.clone()));
        }
        for mut rt in sub.routes {
            rt.handler_symbol_index += symbol_offset;
            r.routes.push(rt);
        }
        for mut ds in sub.db_sets {
            ds.property_symbol_index += symbol_offset;
            r.db_sets.push(ds);
        }
        r.has_errors = r.has_errors || sub.has_errors;
    }
}

/// Strip a synthetic scope prefix (`"__RazorBody"`) from a dotted qualified
/// name in place. Handles both exact match and leading-with-dot forms:
///
///   * `"__RazorBody"`          → `""`   (empty — caller treats as no scope)
///   * `"__RazorBody.Foo"`      → `"Foo"`
///   * `"__RazorBody.Foo.Bar"`  → `"Foo.Bar"`
///   * `"Other.__RazorBody.X"`  → unchanged (prefix only strips at start)
fn strip_scope_prefix_in_place(name: &mut String, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    if name == prefix {
        name.clear();
        return;
    }
    let dotted = format!("{prefix}.");
    if name.starts_with(&dotted) {
        *name = name[dotted.len()..].to_string();
    }
}

/// Blank interpolation spans inside an embedded-region text, preserving byte
/// length and newlines so sub-extractor line/column numbers stay accurate.
/// Only called for Tier-3 string-DSL consumers; S11's SFC host extractors
/// leave `holes` empty and never invoke this path.
fn punch_holes(text: &str, holes: &[crate::types::Span]) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut sorted: Vec<crate::types::Span> = holes.to_vec();
    sorted.sort_by_key(|s| s.start);
    let mut cursor = 0usize;
    for hole in sorted {
        let start = hole.start.min(bytes.len());
        let end = hole.end.min(bytes.len());
        if start < cursor {
            continue; // overlapping hole; skip
        }
        out.extend_from_slice(&bytes[cursor..start]);
        for b in &bytes[start..end] {
            out.push(if *b == b'\n' { b'\n' } else { b' ' });
        }
        cursor = end;
    }
    out.extend_from_slice(&bytes[cursor..]);
    // Host extractors emitting holes are contracted to align spans on UTF-8
    // codepoint boundaries. If that invariant is violated, fall back to the
    // unpunched text — the sub-parse may fail on the interpolation but at
    // least we stay valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}
