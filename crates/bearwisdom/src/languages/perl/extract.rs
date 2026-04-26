// =============================================================================
// languages/perl/extract.rs  —  Perl symbol and reference extractor
//
// Line-oriented scanner (no tree-sitter grammar due to ABI conflict with
// tree-sitter-perl 1.1 requiring tree-sitter 0.26).
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `package Foo::Bar;`
//   Function   — `sub name { ... }` or `sub name;`
//
// REFERENCES:
//   Imports    — `use Module::Name ...;`
//   Calls      — `foo(...)` at module/function level
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Track current package for context; track current sub for call scoping
    let mut current_sub_idx: Option<usize> = None;
    let mut in_sub_depth: u32 = 0;

    for (lineno, line) in source.lines().enumerate() {
        let line_u32 = lineno as u32;
        let trimmed = line.trim();

        // Count braces to track sub body depth (rough)
        if in_sub_depth > 0 {
            for ch in trimmed.chars() {
                match ch {
                    '{' => in_sub_depth += 1,
                    '}' => {
                        if in_sub_depth > 0 { in_sub_depth -= 1; }
                        if in_sub_depth == 0 { current_sub_idx = None; }
                    }
                    _ => {}
                }
            }
        }

        if trimmed.starts_with("package ") {
            // package Foo::Bar;
            if let Some(name) = parse_package(trimmed) {
                let idx = symbols.len();
                symbols.push(make_symbol(name.clone(), name, SymbolKind::Namespace, line_u32, None));
                current_sub_idx = Some(idx);
            }
        } else if trimmed.starts_with("sub ") {
            // sub name { ... } or sub name;
            if let Some(name) = parse_sub(trimmed) {
                let idx = symbols.len();
                let sig = format!("sub {}", name);
                symbols.push(make_symbol(name.clone(), name, SymbolKind::Function, line_u32, Some(sig)));
                current_sub_idx = Some(idx);
                // Count opening brace for depth tracking
                in_sub_depth = trimmed.chars().filter(|&c| c == '{').count() as u32;
                in_sub_depth = in_sub_depth.saturating_sub(trimmed.chars().filter(|&c| c == '}').count() as u32);
            }
        } else if trimmed.starts_with("use ") && !trimmed.starts_with("use strict") && !trimmed.starts_with("use warnings") {
            // use Module::Name qw(...);
            if let Some(module) = parse_use(trimmed) {
                let src_idx = current_sub_idx.unwrap_or_else(|| symbols.len().saturating_sub(1));
                refs.push(ExtractedRef {
                    source_symbol_index: src_idx,
                    target_name: module.clone(),
                    kind: EdgeKind::Imports,
                    line: line_u32,
                    module: Some(module),
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn parse_package(line: &str) -> Option<String> {
    // `package Foo::Bar;` or `package Foo::Bar 1.0;`
    let rest = line.strip_prefix("package ")?.trim();
    let name = rest
        .split(|c: char| c == ';' || c.is_whitespace())
        .next()?
        .trim();
    if name.is_empty() { None } else { Some(name.to_string()) }
}

fn parse_sub(line: &str) -> Option<String> {
    // `sub name { ... }` or `sub name;` or `sub name (signature) { ... }`
    let rest = line.strip_prefix("sub ")?.trim();
    let name = rest
        .split(|c: char| c == '{' || c == '(' || c == ';' || c.is_whitespace())
        .next()?
        .trim();
    if name.is_empty() { None } else { Some(name.to_string()) }
}

fn parse_use(line: &str) -> Option<String> {
    // `use Module::Name ...;`
    let rest = line.strip_prefix("use ")?.trim();
    let module = rest
        .split(|c: char| c == ';' || c.is_whitespace() || c == '(')
        .next()?
        .trim();
    // Skip version numbers like `use 5.020;`
    if module.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if module.is_empty() { None } else { Some(module.to_string()) }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    line: u32,
    signature: Option<String>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}
