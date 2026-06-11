//! Angular binding-expression detection.
//!
//! Scans the template text for:
//!
//!   * `{{ expr }}`        — interpolation
//!   * `[prop]="expr"`     — property binding
//!   * `(event)="expr"`    — event binding
//!   * `*ngIf="expr"`, `*ngFor="let x of expr"` — structural directives
//!
//! Each expression body becomes a TypeScript `StringDsl` region so
//! identifiers inside the expression resolve against the project's
//! TS symbol index. We don't invoke the HTML grammar here — a text
//! scanner is simpler and handles Angular-specific attribute
//! syntax (`[...]`, `(...)`, `*...`) that tree-sitter-html doesn't
//! treat as distinct tokens.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    // Template reference variables (`#grid`, `#userForm`) are local declarations
    // scoped to the template, not component members. Collect them once and seed
    // every binding region with a local so their uses resolve in-region instead
    // of emitting an unresolved ref to a non-existent component property.
    let template_refs = collect_template_ref_vars(source);
    collect_interpolations(source, &template_refs, &mut regions);
    collect_binding_attributes(source, &template_refs, &mut regions);
    regions
}

/// Collect the names of Angular template reference variables declared in the
/// source — the `#name` (and legacy `ref-name`) attribute form. These are
/// template-local bindings; identifier uses elsewhere in the template refer to
/// them, never to a component member.
fn collect_template_ref_vars(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut in_tag = false;
    while i < bytes.len() {
        let b = bytes[i];
        if !in_tag && b == b'<' {
            in_tag = true;
            i += 1;
            continue;
        }
        if in_tag && b == b'>' {
            in_tag = false;
            i += 1;
            continue;
        }
        // Skip quoted attribute values — a `#` inside a value (an href fragment,
        // a CSS color) is data, not a reference-variable declaration.
        if in_tag && (b == b'"' || b == b'\'') {
            let quote = b;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            i += 1;
            continue;
        }
        // A `#` inside a tag, at an attribute boundary (preceded by whitespace or
        // the tag open), introduces a reference variable.
        if in_tag && b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'<') {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'$') {
                j += 1;
            }
            if j > start {
                if let Some(name) = source.get(start..j) {
                    let owned = name.to_string();
                    if !names.contains(&owned) {
                        names.push(owned);
                    }
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    names
}

/// Build the `let name: any;` local-declaration prelude that seeds a binding
/// region with the template's reference variables. Empty when none are declared.
fn template_ref_prelude(refs: &[String]) -> String {
    let mut out = String::new();
    for name in refs {
        out.push_str(&format!("let {name}: any; "));
    }
    out
}

/// Normalize an Angular binding expression into plain TypeScript. `$any(expr)`
/// is the template language's no-op cast — rewriting the `$any(` token to a bare
/// `(` keeps the parentheses balanced and drops the synthetic `$any` call that
/// would otherwise be emitted as an unresolved ref.
fn normalize_ng_expr(expr: &str) -> String {
    expr.replace("$any(", "(")
}

fn collect_interpolations(
    source: &str,
    template_refs: &[String],
    regions: &mut Vec<EmbeddedRegion>,
) {
    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let expr_start = i + 2;
            let Some(end_rel) = find_double_close(&bytes[expr_start..]) else {
                i += 2;
                continue;
            };
            let expr_end = expr_start + end_rel;
            if let Some(text) = source.get(expr_start..expr_end) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let (line, col) = line_col_at(bytes, expr_start);
                    regions.push(make_binding_region(trimmed, template_refs, line, col, idx));
                    idx += 1;
                }
            }
            i = expr_end + 2;
            continue;
        }
        i += 1;
    }
}

fn find_double_close(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Scan for Angular binding attributes — `[prop]="..."`, `(evt)="..."`,
/// `*ngIf="..."`, `*ngFor="let x of xs"`. We look for the leading
/// punctuation (`[`, `(`, `*`) inside HTML start tags and parse the
/// quoted RHS as a TypeScript expression.
fn collect_binding_attributes(
    source: &str,
    template_refs: &[String],
    regions: &mut Vec<EmbeddedRegion>,
) {
    let bytes = source.as_bytes();
    let prelude = template_ref_prelude(template_refs);
    let mut idx = 1000u32; // distinct from interpolation indices
    let mut i = 0usize;
    let mut in_tag = false;
    while i < bytes.len() {
        let b = bytes[i];
        if !in_tag && b == b'<' {
            in_tag = true;
            i += 1;
            continue;
        }
        if in_tag && b == b'>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if in_tag && matches!(b, b'[' | b'(' | b'*') {
            if let Some((expr_start, expr_end)) = find_attr_expression(bytes, i) {
                if let Some(text) = source.get(expr_start..expr_end) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        let (line, col) = line_col_at(bytes, expr_start);
                        let is_structural = b == b'*' && trimmed.starts_with("let ");
                        let expr = normalize_ng_expr(trimmed);
                        let wrapped = if is_structural {
                            format!("function __NgExpr{idx}() {{ {prelude}for ({expr}) {{}} }}\n")
                        } else {
                            format!("function __NgExpr{idx}() {{ {prelude}return ({expr}); }}\n")
                        };
                        regions.push(EmbeddedRegion {
                            language_id: "typescript".to_string(),
                            text: wrapped,
                            line_offset: line,
                            col_offset: col,
                            origin: EmbeddedOrigin::StringDsl,
                            holes: Vec::new(),
                            strip_scope_prefix: None,
                        });
                        idx += 1;
                    }
                }
                i = expr_end + 1;
                continue;
            }
        }
        if b == b'"' || b == b'\'' {
            // Skip past unrelated attribute values when not a binding.
            let quote = b;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
        }
        i += 1;
    }
}

fn make_binding_region(
    expr: &str,
    template_refs: &[String],
    line: u32,
    col: u32,
    idx: u32,
) -> EmbeddedRegion {
    let prelude = template_ref_prelude(template_refs);
    let body = normalize_ng_expr(expr);
    EmbeddedRegion {
        language_id: "typescript".to_string(),
        text: format!("function __NgInterp{idx}() {{ {prelude}return ({body}); }}\n"),
        line_offset: line,
        col_offset: col,
        origin: EmbeddedOrigin::StringDsl,
        holes: Vec::new(),
        strip_scope_prefix: None,
    }
}

/// Given a position pointing at `[`, `(`, or `*` inside a start tag,
/// find the `="..."` expression body's byte range. Returns
/// `(start, end)` of the expression text (inside the quotes).
fn find_attr_expression(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    // Find the closing `]` or `)` (for `[...]=` and `(...)=`), or just
    // skip the identifier (for `*ngIf=`).
    let open = bytes[start];
    let mut i = start + 1;
    if open == b'[' {
        while i < bytes.len() && bytes[i] != b']' {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        i += 1; // past `]`
    } else if open == b'(' {
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        i += 1; // past `)`
    } else {
        // `*directive` — advance past identifier.
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
        {
            i += 1;
        }
    }
    // Expect `="..."` — optional whitespace around `=`.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let expr_start = i + 1;
    let mut j = expr_start;
    while j < bytes.len() && bytes[j] != quote {
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    Some((expr_start, j))
}

fn line_col_at(bytes: &[u8], byte_pos: usize) -> (u32, u32) {
    let mut line: u32 = 0;
    let mut last_nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(byte_pos) {
        if *b == b'\n' {
            line += 1;
            last_nl = i + 1;
        }
    }
    (line, (byte_pos - last_nl) as u32)
}

#[cfg(test)]
#[path = "embedded_tests.rs"]
mod tests;
