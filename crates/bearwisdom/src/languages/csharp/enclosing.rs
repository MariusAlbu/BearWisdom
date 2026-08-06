// =============================================================================
// csharp/enclosing.rs — per-line innermost-symbol attribution for AST scans.
//
// The post-traversal type-position scan walks the raw tree without symbol
// context. This map answers "which extracted symbol encloses line N?" so a
// scanned ref is attributed to its innermost declaring member — never to the
// file's namespace symbol — and deduplicates against refs the symbol-driven
// passes already emitted at the same site.
// =============================================================================

use std::collections::HashSet;

use crate::types::{ExtractedRef, ExtractedSymbol};

pub(super) struct ScanAttribution {
    /// line → innermost enclosing symbol index. Wider spans are written
    /// first so narrower (inner) symbols overwrite: namespace → class →
    /// member. Lines outside every span fall back to symbol 0.
    by_line: Vec<usize>,
    /// `(target_name, line)` of every ref an earlier pass emitted, any kind.
    /// The scanner is a gap-filler; a site the symbol passes already covered
    /// needs no second emission — a member CALL at the site makes a scanned
    /// TypeRef of the same name pure chain noise, and a same-name TypeRef
    /// under a second source symbol duplicates the ref.
    seen: HashSet<(String, u32)>,
}

impl ScanAttribution {
    pub(super) fn build(symbols: &[ExtractedSymbol], refs: &[ExtractedRef]) -> Self {
        let max_line = symbols.iter().map(|s| s.end_line).max().unwrap_or(0) as usize;
        let mut by_line = vec![0usize; max_line + 1];
        let mut order: Vec<usize> = (0..symbols.len()).collect();
        order.sort_by_key(|&i| {
            std::cmp::Reverse(symbols[i].end_line.saturating_sub(symbols[i].start_line))
        });
        for &i in &order {
            let s = &symbols[i];
            let end = (s.end_line as usize).min(max_line);
            for slot in by_line
                .iter_mut()
                .take(end + 1)
                .skip(s.start_line as usize)
            {
                *slot = i;
            }
        }
        let seen = refs
            .iter()
            .map(|r| (r.target_name.clone(), r.line))
            .collect();
        Self { by_line, seen }
    }

    /// Innermost enclosing symbol index for a ref at `line`.
    pub(super) fn source_at(&self, line: u32) -> usize {
        self.by_line.get(line as usize).copied().unwrap_or(0)
    }

    /// Register a scanned TypeRef site; `false` when an identical
    /// `(name, line)` TypeRef already exists and the emission must be skipped.
    pub(super) fn claim(&mut self, name: &str, line: u32) -> bool {
        self.seen.insert((name.to_string(), line))
    }
}

#[cfg(test)]
#[path = "enclosing_tests.rs"]
mod tests;
