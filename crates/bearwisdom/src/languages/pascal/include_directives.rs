// =============================================================================
// languages/pascal/include_directives.rs — {$include} / {$i} directive scanning
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef};

/// Scan the raw source for `{$include 'file.inc'}` / `{$i file.inc}` directives
/// and emit one `Imports` ref per directive, keyed by the include file's bare
/// stem (directory, quotes, and extension stripped).
///
/// The `{$I}` short form is overloaded: `{$I+}` / `{$I-}` is the I/O-checking
/// compiler switch, not an include. We only treat `{$i …}` as an include when a
/// filename follows (the long `{$include …}` form is unambiguous).
pub(super) fn extract_include_directives(src: &str, refs: &mut Vec<ExtractedRef>) {
    // Anchor every directive ref to the unit/program namespace, which
    // `extract_unit` / `extract_program` push at index 0. The resolver's
    // wildcard rung reads only `target_name`, so the anchor only needs to be a
    // valid in-file symbol index.
    let anchor = 0;

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // Directive openings start with `{$`.
        if bytes[i] != b'{' || i + 1 >= len || bytes[i + 1] != b'$' {
            i += 1;
            continue;
        }
        let close = match bytes[i..].iter().position(|&b| b == b'}') {
            Some(p) => i + p,
            None => break, // unterminated directive — nothing further to scan
        };
        let inner = &src[i + 2..close];
        if let Some(stem) = parse_include_directive(inner) {
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: anchor,
                target_name: stem.clone(),
                kind: EdgeKind::Imports,
                line: line_of_byte(src, i),
                col: 0,
                module: Some(stem),
                chain: None,
                byte_offset: i as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
        i = close + 1;
    }
}

/// Parse the body of a `{$…}` directive (the text between `{$` and `}`). Returns
/// the bare include-file stem when the directive is an include, else `None`.
///
/// Accepts `include FILE`, `INCLUDE FILE`, and the `i FILE` / `I FILE` short
/// form (but not the `I+` / `I-` I/O-check switch, which carries no filename).
fn parse_include_directive(inner: &str) -> Option<String> {
    let lower = inner.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("include") {
        // Long form: a separator must follow `include` (so `includepath` etc.
        // don't match). The argument is taken from the original-cased text.
        if r.is_empty() || r.starts_with(|c: char| c.is_whitespace()) {
            inner["include".len()..].trim_start()
        } else {
            return None;
        }
    } else if let Some(r) = lower.strip_prefix('i') {
        // Short form: `{$i FILE}`. Reject the I/O-check switch `{$I+}` / `{$I-}`
        // and any directive where `i` is not immediately followed by whitespace.
        if r.starts_with(|c: char| c.is_whitespace()) {
            inner["i".len()..].trim_start()
        } else {
            return None;
        }
    } else {
        return None;
    };

    let filename = rest.trim().trim_matches(|c| c == '\'' || c == '"').trim();
    if filename.is_empty() {
        return None;
    }
    file_stem_of(filename)
}

/// Reduce an include argument (`'inc/shared_defs.inc'` → `shared_defs`) to its
/// bare file stem: drop the directory prefix and the final extension.
pub(super) fn file_stem_of(filename: &str) -> Option<String> {
    let basename = filename
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(filename);
    let stem = basename.rsplit_once('.').map(|(s, _)| s).unwrap_or(basename);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

/// 0-based line index of a byte offset, for the directive ref's `line` field.
fn line_of_byte(src: &str, byte_offset: usize) -> u32 {
    src[..byte_offset.min(src.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count() as u32
}

#[cfg(test)]
#[path = "include_directives_tests.rs"]
mod tests;
