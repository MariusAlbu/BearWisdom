// =============================================================================
// languages/common/jquery.rs — `$.fn.NAME = function(...)` plugin globals
// =============================================================================

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind};

// ---------------------------------------------------------------------------
// jQuery plugin registration: `$.fn.NAME = function(...) { ... }`
//
// Older AMD / RequireJS-era JS projects (SWISH, AngularJS-1 themes, many
// jQuery-plugin authors) register methods on `$.fn` — these become
// chainable methods on every jQuery selector (`$(elem).NAME(...)`). The
// JS extractor sees the call as a Calls ref to `NAME` (the chain root
// `$(elem)` is opaque) but `NAME` doesn't exist as a top-level symbol
// anywhere — it lives as a property on the jQuery prototype.
//
// Discovery: scan source for the literal `$.fn.NAME = function`,
// `jQuery.fn.NAME = function`, or `$.fn['NAME'] = function` patterns.
// Each match emits a synthetic Function symbol with
// `qualified_name = "__npm_globals__.NAME"` so the TS resolver's bare-
// name fallback finds it. Mirrors the Handlebars/Ember helper pattern.
//
// Only scans project source; jQuery's CORE methods (`each`, `hasClass`,
// `addClass`, ...) need jQuery's own source on disk to be indexed.
// ---------------------------------------------------------------------------

pub fn append_jquery_fn_plugin_globals(source: &str, result: &mut crate::types::ExtractionResult) {
    for name in scan_jquery_fn_plugin_names(source) {
        let qname = format!("__npm_globals__.{name}");
        if result.symbols.iter().any(|s| s.qualified_name == qname) {
            continue;
        }
        result.symbols.push(crate::types::ExtractedSymbol {
            name: name.clone(),
            qualified_name: qname,
            kind: crate::types::SymbolKind::Function,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("/* $.fn.{name} jQuery plugin */")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
}

pub(crate) fn scan_jquery_fn_plugin_names(source: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Match `$.fn` or `jQuery.fn` followed by either `.NAME` or `['NAME']`.
    let needles = ["$.fn", "jQuery.fn"];
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let mut matched_len: Option<usize> = None;
        for needle in needles {
            let nb = needle.as_bytes();
            if i + nb.len() <= bytes.len() && &bytes[i..i + nb.len()] == nb {
                // Suffix must be `.` or `[` to be the property-access form
                // we care about. Otherwise this is `$.fn` standalone or
                // `jQuery.fn` followed by something else.
                let suffix = bytes.get(i + nb.len()).copied().unwrap_or(0);
                if suffix == b'.' || suffix == b'[' {
                    matched_len = Some(nb.len() + 1); // include the `.` or `[`
                    break;
                }
            }
        }
        if let Some(needle_len) = matched_len {
            // Identifier-boundary check: prev char must NOT be alphanum/_.
            if i > 0 {
                let prev = bytes[i - 1];
                if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                    i += needle_len;
                    continue;
                }
            }
            // bytes[i + needle_len - 1] is either `.` or `[`.
            let suffix = bytes[i + needle_len - 1];
            let after = i + needle_len;
            let (name, mut k) = if suffix == b'[' {
                // `$.fn[ 'name' ]` — skip whitespace, expect quote.
                let mut j = after;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j >= bytes.len() || (bytes[j] != b'\'' && bytes[j] != b'"') {
                    i = after;
                    continue;
                }
                let q = bytes[j];
                let start = j + 1;
                let mut e = start;
                while e < bytes.len() && bytes[e] != q {
                    if bytes[e] == b'\\' && e + 1 < bytes.len() {
                        e += 2;
                    } else {
                        e += 1;
                    }
                }
                let n = std::str::from_utf8(&bytes[start..e]).ok();
                (n.map(str::to_string), e + 1)
            } else {
                // `$.fn.NAME` — bare identifier.
                let start = after;
                let mut e = start;
                while e < bytes.len()
                    && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_' || bytes[e] == b'$')
                {
                    e += 1;
                }
                let n = std::str::from_utf8(&bytes[start..e]).ok();
                (n.map(str::to_string), e)
            };
            // Now expect `=` (allow whitespace and trailing `]` from bracket form).
            while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t' || bytes[k] == b']') {
                k += 1;
            }
            if k >= bytes.len() || bytes[k] != b'=' {
                i = after;
                continue;
            }
            // Confirm RHS looks like a function (function/arrow/method).
            let mut m = k + 1;
            while m < bytes.len() && (bytes[m] == b' ' || bytes[m] == b'\t' || bytes[m] == b'\n') {
                m += 1;
            }
            let rhs_is_function = m + 8 <= bytes.len() && &bytes[m..m + 8] == b"function"
                || m < bytes.len() && bytes[m] == b'('
                || m + 5 <= bytes.len() && &bytes[m..m + 5] == b"async";
            if !rhs_is_function {
                i = after;
                continue;
            }
            if let Some(n) = name {
                let trimmed = n.trim();
                if !trimmed.is_empty()
                    && trimmed
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    && !out.iter().any(|x| x == trimmed)
                {
                    out.push(trimmed.to_string());
                }
            }
            i = m;
            continue;
        }
        i += 1;
    }
    out
}
