// =============================================================================
// languages/common/amd.rs — AMD `define([deps], function(params) { ... })` imports
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractionResult};

// ---------------------------------------------------------------------------
// AMD `define([deps], function(params) { ... })` — RequireJS modules
//
// The classic AMD pattern (RequireJS, used by SWISH, AngularJS-1
// projects, jQuery plugin authors, etc.) is conceptually identical to
// ES module imports but wraps everything in a single `define()` call:
//
//   define([ "jquery", "./config", "preferences" ],
//          function($, config, preferences) {
//              // body uses $, config, preferences as locals
//          });
//
// Each dep string in the array maps positionally to a function param.
// Without recognising this shape, every reference to `$.each`,
// `config.foo`, or `preferences.bar` inside the callback lands in
// unresolved_refs because the params aren't declared anywhere the
// resolver thinks of as an import.
//
// `append_amd_define_imports` scans the source for the literal
// `define([...] , function(...) { ... })` shape and emits one
// `EdgeKind::Imports` ref per (dep, param) pair, with
// `target_name = param`, `module = Some(dep)`. The shared
// `resolve_common` path then handles them like any other named
// import.
// ---------------------------------------------------------------------------

pub fn append_amd_define_imports(
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    let pairs = scan_amd_define_pairs(source);
    if pairs.is_empty() {
        return;
    }
    let line_starts: Vec<u32> = {
        let mut offsets = vec![0u32];
        let mut pos: u32 = 0;
        for b in source.bytes() {
            pos += 1;
            if b == b'\n' { offsets.push(pos); }
        }
        offsets
    };
    for (dep, param, line) in pairs {
        // Skip dummy AMD names (`require`, `exports`, `module`) — these
        // are AMD bookkeeping, not real deps.
        if matches!(dep.as_str(), "require" | "exports" | "module") {
            continue;
        }
        result.refs.push(crate::types::ExtractedRef {
            source_symbol_index: 0,
            target_name: param,
            kind: crate::types::EdgeKind::Imports,
            line,
            module: Some(dep),
            chain: None,
            byte_offset: line_starts.get(line as usize).copied().unwrap_or(0),
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

/// Returns a `(dep, param, line)` triple for every dep position in every
/// `define([...], function(...) {...})` block in `source`. Tolerant to
/// whitespace, line breaks, single/double-quoted dep strings, and
/// `define("name", [...], function(...){...})` (named modules — the
/// leading string is ignored).
pub(crate) fn scan_amd_define_pairs(source: &str) -> Vec<(String, String, u32)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(rel) = source[i..].find("define(") else { break };
        let start = i + rel;
        // Identifier-boundary check.
        if start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                i = start + 7;
                continue;
            }
        }
        let after_open = start + 7; // past `define(`
        let mut j = skip_ws(bytes, after_open);
        // Optional leading string (named module).
        if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
            let q = bytes[j];
            j += 1;
            while j < bytes.len() && bytes[j] != q {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                } else {
                    j += 1;
                }
            }
            if j < bytes.len() {
                j += 1;
            }
            j = skip_ws(bytes, j);
            if j < bytes.len() && bytes[j] == b',' {
                j += 1;
                j = skip_ws(bytes, j);
            }
        }
        if j >= bytes.len() || bytes[j] != b'[' {
            i = after_open;
            continue;
        }
        // Parse the dep array.
        j += 1;
        let mut deps: Vec<(String, u32)> = Vec::new();
        loop {
            j = skip_ws(bytes, j);
            if j >= bytes.len() {
                break;
            }
            if bytes[j] == b']' {
                j += 1;
                break;
            }
            if bytes[j] == b',' {
                j += 1;
                continue;
            }
            if bytes[j] == b'\'' || bytes[j] == b'"' {
                let q = bytes[j];
                let dep_start = j + 1;
                let mut k = dep_start;
                while k < bytes.len() && bytes[k] != q {
                    if bytes[k] == b'\\' && k + 1 < bytes.len() {
                        k += 2;
                    } else {
                        k += 1;
                    }
                }
                if k > bytes.len() {
                    break;
                }
                let dep = source[dep_start..k].to_string();
                let line = line_at(bytes, dep_start);
                deps.push((dep, line));
                j = k + 1;
            } else {
                // Unrecognised token (variable ref, etc.) — bail.
                return out;
            }
        }
        j = skip_ws(bytes, j);
        if j >= bytes.len() || bytes[j] != b',' {
            i = after_open;
            continue;
        }
        j += 1;
        j = skip_ws(bytes, j);
        // Expect `function(` (allow `function name(` and arrow-style
        // `(a, b) =>` too).
        let params = parse_callback_params(bytes, &mut j);
        let Some(params) = params else {
            i = after_open;
            continue;
        };
        for (idx, (dep, line)) in deps.iter().enumerate() {
            let Some(param) = params.get(idx) else { break };
            if param.is_empty() {
                continue;
            }
            out.push((dep.clone(), param.clone(), *line));
        }
        i = j;
    }
    out
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            // Line comment.
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // Block comment.
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            _ => return i,
        }
    }
    i
}

/// Parse a `function(p1, p2)` or `(p1, p2) =>` parameter list starting
/// at `*pos`. On success, advances `*pos` past the `(` ... `)` and
/// returns the param names (stripped). Returns None for shapes the
/// scanner doesn't recognise.
fn parse_callback_params(bytes: &[u8], pos: &mut usize) -> Option<Vec<String>> {
    let mut j = *pos;
    j = skip_ws(bytes, j);
    // `function`-form: `function( ... )`, `function name( ... )`.
    if j + 8 <= bytes.len() && &bytes[j..j + 8] == b"function" {
        let after = j + 8;
        let next = bytes.get(after).copied().unwrap_or(0);
        if !next.is_ascii_alphanumeric() && next != b'_' {
            j = after;
            j = skip_ws(bytes, j);
            // Optional named function: `function name(`.
            if j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
                {
                    j += 1;
                }
                j = skip_ws(bytes, j);
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let params = read_paren_params(bytes, j)?;
                let new_j = find_matching_paren(bytes, j)?;
                *pos = new_j + 1;
                return Some(params);
            }
        }
    }
    // Arrow-style: `(p1, p2) =>` or single `p =>`.
    if j < bytes.len() && bytes[j] == b'(' {
        let params = read_paren_params(bytes, j)?;
        let new_j = find_matching_paren(bytes, j)?;
        *pos = new_j + 1;
        return Some(params);
    }
    None
}

fn read_paren_params(bytes: &[u8], open: usize) -> Option<Vec<String>> {
    let close = find_matching_paren(bytes, open)?;
    let inner = std::str::from_utf8(&bytes[open + 1..close]).ok()?;
    Some(
        inner
            .split(',')
            .map(|s| {
                let t = s.trim();
                // Strip default-arg / type-annotations: keep the head
                // identifier only.
                let mut k = 0;
                let bytes = t.as_bytes();
                while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_' || bytes[k] == b'$') {
                    k += 1;
                }
                String::from_utf8_lossy(&bytes[..k]).to_string()
            })
            .collect(),
    )
}

fn find_matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn line_at(bytes: &[u8], pos: usize) -> u32 {
    let mut line = 0u32;
    for &b in &bytes[..pos.min(bytes.len())] {
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

