//! YAML symbol extraction — file-level descriptor, top-level keys, and
//! GitHub-Actions `uses:` reference extraction.
//!
//! Top-level mapping keys become Field symbols so a consumer can ask
//! "does this file define a `jobs` key" / "does this pipeline set
//! `stages`". Nested structure is not surfaced — that's what the CI
//! platform docs are for.
//!
//! GitHub Actions workflows reference reusable workflows and local
//! composite actions via `uses:` directives. Local refs (paths
//! beginning with `./` or `../`) become Imports refs that the
//! [`super::resolve::YamlResolver`] resolves against the target
//! action.yml or workflow file. External refs (`actions/checkout@v4`
//! shape) are ignored — they live outside the project and have no
//! on-disk source to bind against here.

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let stem = norm.rsplit('/').next().unwrap_or(&norm).to_string();

    let mut symbols = vec![ExtractedSymbol {
        name: stem.clone(),
        qualified_name: stem.clone(),
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

    // Walk source for lines of the form `key:` at column 0 (top-level).
    for (line_no, line) in source.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // Must start at column 0 (no indent) and end with `:` or `:<space>value`.
        if line.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim();
            // A single identifier-like token.
            if !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            {
                symbols.push(ExtractedSymbol {
                    name: key.to_string(),
                    qualified_name: format!("{stem}.{key}"),
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: line_no as u32,
                    end_line: line_no as u32,
                    start_col: 0,
                    end_col: key.len() as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(stem.clone()),
                    parent_index: Some(0),
                });
            }
        }
    }

    let refs = collect_uses_refs(source, &norm);

    ExtractionResult::new(symbols, refs, false)
}

/// Scan a GitHub-Actions YAML for `uses: <local-path>` directives.
/// Only fires inside files whose path looks like a GHA workflow or
/// composite action (`.github/workflows/*` or `action.{yml,yaml}` at
/// any depth) — outside those layouts `uses:` is just a regular YAML
/// key with arbitrary user-defined meaning.
///
/// Captures `./...` and `../...` paths only. External-action refs
/// (`<owner>/<name>@<version>`) are discarded: they live outside the
/// project and there's no on-disk source to bind them to from here.
fn collect_uses_refs(source: &str, file_path: &str) -> Vec<ExtractedRef> {
    if !is_github_actions_path(file_path) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let Some((_, raw_value)) = parse_uses_line(line) else { continue };
        let value = strip_inline_comment(raw_value).trim();
        // Strip surrounding quotes if present.
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        if value.is_empty() {
            continue;
        }
        // Local refs only — relative paths starting with `./` or `../`.
        // Bare paths (`my-action/sub`) are ambiguous in GHA syntax and
        // canonical workflows always prefix with `./`, so we require it.
        if !value.starts_with("./") && !value.starts_with("../") {
            continue;
        }
        out.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: value.to_string(),
            kind: EdgeKind::Imports,
            line: line_no as u32,
            module: Some(value.to_string()),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
        });
    }
    out
}

/// Return true if `file_path` looks like a GitHub Actions workflow or
/// composite-action definition.
fn is_github_actions_path(file_path: &str) -> bool {
    let p = file_path.replace('\\', "/").to_ascii_lowercase();
    if p.contains(".github/workflows/") {
        return true;
    }
    let stem = p.rsplit('/').next().unwrap_or(&p);
    stem == "action.yml" || stem == "action.yaml"
}

/// Parse a `<indent>uses: <value>` (or `<indent>- uses: <value>`) line.
/// Returns the (key, value) pair when the trimmed key is exactly
/// `uses`. Tolerates list-dash prefix and arbitrary indentation.
fn parse_uses_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let colon = after_dash.find(':')?;
    let key = after_dash[..colon].trim();
    if key != "uses" {
        return None;
    }
    let value = after_dash[colon + 1..].trim_start();
    Some((key, value))
}

/// YAML inline comments start with ` #` (whitespace + hash). Strip them
/// from the value.
fn strip_inline_comment(value: &str) -> &str {
    if let Some(idx) = value.find(" #") {
        &value[..idx]
    } else {
        value
    }
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
