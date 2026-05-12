//! MDX host-level extraction.
//!
//! Shared baseline (headings, fence anchors, link refs, file-stem
//! symbol) delegates to `markdown::host_scan`. MDX layers JSX
//! component refs on top: every `<Capital…>` or `<Foo.Bar>` opening
//! tag OUTSIDE of fenced code blocks becomes a `Calls` ref against
//! the host file symbol. Resolution later matches those refs against
//! components imported at the top of the file (dispatched as a TS
//! `ScriptBlock` region by `embedded.rs`) and against project-wide
//! component symbols.

use super::super::markdown::{fenced, host_scan};
use crate::types::{EdgeKind, ExtractedRef, ExtractionResult};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut scan = host_scan::scan(source, file_path);
    collect_jsx_refs(source, scan.host_index, &mut scan.refs);
    ExtractionResult {
        symbols: scan.symbols,
        refs: scan.refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn collect_jsx_refs(source: &str, host_index: usize, refs: &mut Vec<ExtractedRef>) {
    let fence_ranges = fence_byte_ranges(source);
    let inline_code_ranges = inline_code_byte_ranges(source);
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if inside_any_range(i, &fence_ranges) {
            // Skip to the end of the current fence.
            if let Some(end) = fence_ranges.iter().find(|(s, e)| i >= *s && i < *e).map(|(_, e)| *e) {
                i = end;
                continue;
            }
        }
        if inside_any_range(i, &inline_code_ranges) {
            // Skip past the end of the current inline-code span.
            if let Some(end) = inline_code_ranges.iter().find(|(s, e)| i >= *s && i < *e).map(|(_, e)| *e) {
                i = end;
                continue;
            }
        }
        if bytes[i] == b'<' {
            if let Some((name, consumed)) = scan_jsx_tag(&bytes[i..]) {
                let line = line_of_byte(bytes, i);
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
                i += consumed;
                continue;
            }
        }
        i += 1;
    }
}

/// Collect byte ranges covering inline-code spans — `` `code` `` and
/// ``` `` code `` ``` (the double-backtick form used when the content
/// itself contains a single backtick). The goal is to skip
/// `` `ICommand<TResponse>` `` and similar prose where `<T>` is a TypeScript
/// generic-type fragment, not JSX. CommonMark defines an inline-code span
/// as a run of N backticks terminated by the next run of exactly N
/// backticks, with the content taken literally — we approximate that.
fn inline_code_byte_ranges(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        // Count consecutive backticks.
        let run_start = i;
        let mut run_len = 0usize;
        while i < bytes.len() && bytes[i] == b'`' {
            run_len += 1;
            i += 1;
        }
        // Find a closing run of exactly run_len backticks on the same
        // line or up to end-of-file. Unmatched opens are prose text.
        let content_start = i;
        let mut j = i;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let mut close_len = 0usize;
                let close_start = j;
                while j < bytes.len() && bytes[j] == b'`' {
                    close_len += 1;
                    j += 1;
                }
                if close_len == run_len {
                    ranges.push((run_start, j));
                    i = j;
                    break;
                }
                // A differently-sized backtick run — keep searching.
                let _ = close_start;
            } else if bytes[j] == b'\n' {
                // Inline code doesn't span paragraph breaks in most
                // CommonMark dialects; a lone backtick run is treated
                // as literal text. Abort scanning this run.
                i = content_start;
                break;
            } else {
                j += 1;
            }
        }
        // If we ran off the end without a match, advance past the
        // opening run so we don't loop.
        if i < run_start + run_len {
            i = run_start + run_len;
        }
    }
    ranges
}

/// Collect (start, end) byte ranges covering each fenced code block's
/// body (so JSX scanning skips inside fences).
fn fence_byte_ranges(source: &str) -> Vec<(usize, usize)> {
    fenced::parse_fences(source)
        .into_iter()
        .map(|f| (f.body_byte_offset, f.body_byte_offset + f.body.len()))
        .collect()
}

fn inside_any_range(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|(s, e)| pos >= *s && pos < *e)
}

/// If `bytes` starts with `<Name[.Qualifier]*` followed by a valid JSX
/// terminator (`>`, `/`, whitespace, or `\n`), return the tag name and
/// the number of bytes consumed (up to and including the `<Name…`
/// portion). Returns `None` for HTML lowercase tags, end tags
/// (`</...>`), fragments (`<>`), and comments (`<!--`).
fn scan_jsx_tag(bytes: &[u8]) -> Option<(String, usize)> {
    if bytes.first() != Some(&b'<') {
        return None;
    }
    if bytes.len() < 2 {
        return None;
    }
    let b1 = bytes[1];
    // Skip end tags, fragments, comments, doctypes.
    if b1 == b'/' || b1 == b'>' || b1 == b'!' || b1 == b'?' {
        return None;
    }
    // Must start with uppercase ASCII letter (or lowercase-then-dot, to
    // accept `<motion.div>` patterns). We accept lowercase starts only
    // when followed by a dot — that's the MDX convention for
    // namespaced imports like `framer-motion`'s `motion.`.
    if !b1.is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1usize;
    let first_segment_start = i;
    while i < bytes.len() && is_jsx_ident_byte(bytes[i]) {
        i += 1;
    }
    let first_segment = std::str::from_utf8(&bytes[first_segment_start..i]).ok()?.to_string();
    let first_is_upper = first_segment
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase());
    let mut dotted_tail = String::new();
    let mut is_dotted = false;
    while i < bytes.len() && bytes[i] == b'.' {
        is_dotted = true;
        i += 1;
        let seg_start = i;
        while i < bytes.len() && is_jsx_ident_byte(bytes[i]) {
            i += 1;
        }
        if i == seg_start {
            return None;
        }
        let seg = std::str::from_utf8(&bytes[seg_start..i]).ok()?;
        dotted_tail.push('.');
        dotted_tail.push_str(seg);
    }
    // Must be terminated by whitespace, `/`, `>` — NOT by a further
    // identifier byte (we already consumed all of them).
    let term = bytes.get(i).copied().unwrap_or(b' ');
    let is_terminator = term == b' '
        || term == b'\t'
        || term == b'\n'
        || term == b'\r'
        || term == b'/'
        || term == b'>'
        || term == b'{';
    if !is_terminator {
        return None;
    }
    // Reject lowercase-starting tags unless they're dotted (e.g.
    // `motion.div`).
    if !first_is_upper && !is_dotted {
        return None;
    }
    let name = format!("{first_segment}{dotted_tail}");
    // Fragment is a JSX built-in slot wrapper — no import needed, not a
    // user component. Suppress it so it doesn't accumulate in unresolved_refs.
    if name == "Fragment" {
        return None;
    }
    Some((name, i))
}

fn is_jsx_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn line_of_byte(bytes: &[u8], pos: usize) -> u32 {
    let mut line: u32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= pos {
            break;
        }
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
