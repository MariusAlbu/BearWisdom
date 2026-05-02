//! EJS host extraction — file-stem class symbol + `include('path')` Imports.
//!
//! EJS templates use `include()` to embed partial templates by path:
//! `<%- include('./partials/header') %>`. The argument is a relative path
//! (typically with explicit `./` prefix) to another `.ejs` file. The path
//! is resolved against the source file's directory at template-render time.
//!
//! We extract these include calls at the host level so they become
//! Imports refs the EJS resolver can match against the partial's
//! file-stem Class symbol. The companion change in `embedded.rs` strips
//! `include(...)` from the embedded JS so the JS extractor doesn't
//! double-emit them as unresolvable Calls.

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let file_name = file_stem(file_path);
    let symbols = vec![ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name,
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }];

    let refs = collect_include_refs(source);

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}

/// Scan the EJS source for `include('path')` / `include("path")` calls
/// inside `<%- %>` and `<%= %>` tags and emit one Imports ref per call.
/// Only the first string-literal argument is captured; the optional second
/// argument (a locals object) is irrelevant for path resolution.
///
/// The scan is intentionally permissive about whitespace and tag form:
///   * `<%- include('partials/x') %>`
///   * `<%= include("./layout") %>`
///   * `<% include('legacy-no-output') %>` — old-style EJS 1.x
fn collect_include_refs(source: &str) -> Vec<ExtractedRef> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Find next `include(`. Could appear inside `<% %>` tags or in
        // free HTML; we capture all occurrences and let the resolver
        // decide what's reachable. False positives (`include` inside a
        // string literal in HTML body) are rare in practice and produce
        // an unresolved Imports ref, not a wrong call edge.
        let Some(rel) = source[i..].find("include(") else { break };
        let absolute = i + rel;
        // Verify the preceding identifier boundary — we want `include(`
        // standing alone, not `_include(` / `xinclude(`.
        if absolute > 0 {
            let prev = bytes[absolute - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                i = absolute + 8;
                continue;
            }
        }
        let arg_start = absolute + 8; // after `include(`
        // Skip whitespace.
        let mut j = arg_start;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
            j += 1;
        }
        if j >= bytes.len() {
            i = absolute + 8;
            continue;
        }
        let quote = bytes[j];
        if quote != b'\'' && quote != b'"' && quote != b'`' {
            i = absolute + 8;
            continue;
        }
        let path_start = j + 1;
        let mut k = path_start;
        // Capture until matching unescaped quote.
        while k < bytes.len() {
            if bytes[k] == b'\\' && k + 1 < bytes.len() {
                k += 2;
                continue;
            }
            if bytes[k] == quote {
                break;
            }
            k += 1;
        }
        if k >= bytes.len() {
            i = absolute + 8;
            continue;
        }
        let raw = &source[path_start..k];
        let target = raw.trim();
        if !target.is_empty() {
            let line = line_at(bytes, absolute);
            out.push(ExtractedRef {
                source_symbol_index: 0,
                target_name: target.to_string(),
                kind: EdgeKind::Imports,
                line,
                module: Some(target.to_string()),
                chain: None,
                byte_offset: absolute as u32,
                namespace_segments: Vec::new(),
            });
        }
        i = k + 1;
    }
    out
}

fn line_at(bytes: &[u8], byte_pos: usize) -> u32 {
    let mut line: u32 = 0;
    for &b in &bytes[..byte_pos] {
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
