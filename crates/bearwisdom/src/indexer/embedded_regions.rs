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
    host_content: &str,
    registry: &LanguageRegistry,
    regions: Vec<crate::types::EmbeddedRegion>,
    r: &mut crate::types::ExtractionResult,
    origin_langs: &mut Vec<Option<String>>,
    from_snippet: &mut Vec<bool>,
    ref_origin_langs: &mut Vec<Option<String>>,
) {
    use crate::types::EmbeddedOrigin;
    let line_starts = build_line_starts(host_content);
    // A Svelte SFC's `<script>` is embedded TypeScript, but `$store` (auto-
    // subscribe) is Svelte syntax the TS extractor emits verbatim. Desugar it
    // to the underlying store identifier here, while the host is known to be
    // Svelte — the TS sub-extraction can't tell it apart from a real `$`-named
    // identifier in a plain `.ts` file.
    let host_is_svelte = file_path.ends_with(".svelte");
    for region in regions {
        let region_host_byte = host_byte_for_position(
            &line_starts,
            host_content,
            region.line_offset,
            region.col_offset,
        );
        let sub_plugin = registry.get(&region.language_id);
        let sub_text = if region.holes.is_empty() {
            region.text.clone()
        } else {
            punch_holes(&region.text, &region.holes)
        };

        let mut sub = sub_plugin.extract(&sub_text, file_path, &region.language_id);
        super::local_refs::filter_local_refs(&sub_text, &region.language_id, sub_plugin, &sub.symbols, &mut sub.refs);
        super::local_refs::filter_operator_refs(&mut sub.refs);

        let symbol_offset = r.symbols.len();
        let line_offset = region.line_offset;
        let col_offset = region.col_offset;
        // E3: Markdown fenced code, Rust doctests, and Python doctests are
        // snippet contexts — usually missing imports, so unresolved refs
        // from their symbols should be excluded from aggregate resolution
        // stats. Frontmatter (YAML/TOML/JSON) doesn't qualify.
        let region_is_snippet = matches!(region.origin, EmbeddedOrigin::MarkdownFence);

        // Phase 1 — strip prefix and identify synthetic wrapper symbols.
        //
        // A synthetic wrapper (e.g. `__RazorBody`) has its qualified_name
        // reduced to the empty string after prefix stripping because its name
        // IS the prefix. Such symbols are host-injected scaffolding that must
        // not appear in the final index.  We drop them and promote their direct
        // children to top-level (parent_index cleared) so the canonical-form
        // invariants hold:
        //   SYM-001 — qualified_name ends with name
        //   SYM-002 — scope_path matches parent qualified_name
        //
        // `sub_remap[sub_idx]` maps each sub-symbol's original index to its
        // final index inside `r.symbols`, or `None` if the symbol is dropped.
        let nsub = sub.symbols.len();
        let mut processed: Vec<Option<crate::types::ExtractedSymbol>> = Vec::with_capacity(nsub);
        let mut sub_remap: Vec<Option<usize>> = vec![None; nsub];

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
            // Symbols whose qualified_name is now empty are synthetic
            // wrappers (the name itself was the stripped prefix).  Drop them.
            if sym.qualified_name.is_empty() {
                processed.push(None);
            } else {
                processed.push(Some(sym));
            }
        }

        // Phase 2 — assign final indices, remap parent_index, push.
        //
        // A child whose parent was a synthetic wrapper (parent maps to None)
        // is promoted to top-level: parent_index cleared, scope_path left as
        // None (which is correct for a top-level symbol).
        let mut next_final = symbol_offset;
        for (sub_idx, slot) in processed.iter().enumerate() {
            if slot.is_some() {
                sub_remap[sub_idx] = Some(next_final);
                next_final += 1;
            }
        }
        for slot in processed {
            let Some(mut sym) = slot else { continue };
            let had_parent = sym.parent_index.is_some();
            sym.parent_index = sym.parent_index.and_then(|old_parent_sub_idx| {
                sub_remap[old_parent_sub_idx]
            });
            if had_parent && sym.parent_index.is_none() {
                // The parent was a synthetic wrapper that was dropped.
                // Promote this symbol to top-level by clearing scope_path.
                // (scope_path is already None here because the prefix strip
                // above reduced it to the empty string and cleared it.)
                sym.scope_path = None;
            }
            r.symbols.push(sym);
            origin_langs.push(Some(region.language_id.clone()));
            from_snippet.push(region_is_snippet);
        }

        for mut rf in sub.refs {
            if host_is_svelte {
                crate::languages::svelte::hooks::desugar_store_ref_in_place(&mut rf);
            }
            // Remap source_symbol_index through the sub→final table.
            // If the owning symbol was a synthetic wrapper that was dropped,
            // fall back to the host file's root symbol (index 0).
            let old_sub_idx = rf.source_symbol_index;
            rf.source_symbol_index = sub_remap
                .get(old_sub_idx)
                .and_then(|m| *m)
                .unwrap_or(0);
            rf.line = rf.line.saturating_add(line_offset);
            rf.byte_offset = rf.byte_offset.saturating_add(region_host_byte);
            r.refs.push(rf);
            // Tag this ref with the embedded language so the resolver
            // routes it to the correct externals/primitives table instead
            // of the host-file language.
            ref_origin_langs.push(Some(region.language_id.clone()));
        }
        for mut rt in sub.routes {
            rt.handler_symbol_index = sub_remap
                .get(rt.handler_symbol_index)
                .and_then(|m| *m)
                .unwrap_or(symbol_offset);
            r.routes.push(rt);
        }
        for mut ds in sub.db_sets {
            ds.property_symbol_index = sub_remap
                .get(ds.property_symbol_index)
                .and_then(|m| *m)
                .unwrap_or(symbol_offset);
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

/// Build a Vec where entry `i` holds the byte offset of the start of line `i`
/// in `content`. Used to translate (line, col) positions back to host-file
/// byte offsets without re-scanning the file for each region.
fn build_line_starts(content: &str) -> Vec<u32> {
    let mut starts = Vec::with_capacity(content.len() / 40);
    starts.push(0u32);
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

/// Compute the host-file byte offset of `(line, col)` against `content`. Col
/// is a tree-sitter byte column on the target line. Out-of-range positions
/// clamp to the file's byte length.
fn host_byte_for_position(line_starts: &[u32], content: &str, line: u32, col: u32) -> u32 {
    let line_start = line_starts
        .get(line as usize)
        .copied()
        .unwrap_or(content.len() as u32);
    line_start.saturating_add(col).min(content.len() as u32)
}
