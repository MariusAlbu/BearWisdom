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
    collect_interpolations(source, &mut regions);
    collect_binding_attributes(source, &mut regions);
    regions
}

fn collect_interpolations(source: &str, regions: &mut Vec<EmbeddedRegion>) {
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
                    regions.push(make_binding_region(trimmed, line, col, idx));
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
fn collect_binding_attributes(source: &str, regions: &mut Vec<EmbeddedRegion>) {
    let bytes = source.as_bytes();
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
                        let is_structural =
                            b == b'*' && trimmed.starts_with("let ");
                        let wrapped = if is_structural {
                            format!(
                                "function __NgExpr{idx}() {{ for ({trimmed}) {{}} }}\n"
                            )
                        } else {
                            format!(
                                "function __NgExpr{idx}() {{ return ({trimmed}); }}\n"
                            )
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

fn make_binding_region(expr: &str, line: u32, col: u32, idx: u32) -> EmbeddedRegion {
    EmbeddedRegion {
        language_id: "typescript".to_string(),
        text: format!("function __NgInterp{idx}() {{ return ({expr}); }}\n"),
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
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_') {
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
mod tests {
    use super::*;

    #[test]
    fn interpolation_becomes_region() {
        let src = "<div>{{ userName }}</div>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("userName")));
    }

    #[test]
    fn property_binding_becomes_region() {
        let src = r#"<img [src]="avatarUrl" />"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("avatarUrl")));
    }

    #[test]
    fn event_binding_becomes_region() {
        let src = r#"<button (click)="handleClick($event)">Go</button>"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("handleClick")));
    }

    #[test]
    fn ng_for_becomes_for_of_region() {
        let src = r#"<li *ngFor="let user of users">{{user.name}}</li>"#;
        let regions = detect_regions(src);
        assert!(
            regions
                .iter()
                .any(|r| r.text.contains("for (let user of users)")),
            "got regions: {regions:#?}"
        );
    }

    #[test]
    fn ng_if_becomes_region() {
        let src = r#"<div *ngIf="isActive">x</div>"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("isActive")));
    }

    #[test]
    fn empty_interpolation_skipped() {
        let src = "{{  }}";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn full_input_line_with_two_bindings_produces_two_regions() {
        let src = r#"<input [value]="formName" (input)="handleInput($event)" />"#;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 2, "expected 2 regions, got {regions:#?}");
    }
}
