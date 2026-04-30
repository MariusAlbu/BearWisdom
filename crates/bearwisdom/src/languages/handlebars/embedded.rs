//! Handlebars embedded regions — JS expressions per mustache tag,
//! plus JS/CSS script/style blocks in the surrounding HTML.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);

    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let triple = bytes.get(i + 2).copied() == Some(b'{');
            let expr_start = if triple { i + 3 } else { i + 2 };
            let close_needle_len = if triple { 3 } else { 2 };
            let mut j = expr_start;
            let mut found = None;
            while j + close_needle_len <= bytes.len() {
                if bytes[j] == b'}'
                    && bytes[j + 1] == b'}'
                    && (!triple || bytes[j + 2] == b'}')
                {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            let Some(expr_end) = found else {
                i += 2;
                continue;
            };
            if let Some(text) = source.get(expr_start..expr_end) {
                let trimmed = text.trim();
                // Skip comments and block open/close markers — no JS there.
                let first = trimmed.chars().next();
                let is_code = !matches!(first, Some('!') | Some('#') | Some('/') | Some('>'));
                if is_code && !trimmed.is_empty() {
                    let js_expr = handlebars_to_js(trimmed);
                    if !js_expr.is_empty() {
                        let (line, col) = line_col_at(bytes, expr_start);
                        regions.push(EmbeddedRegion {
                            language_id: "javascript".to_string(),
                            text: format!(
                                "function __HbsExpr{idx}() {{ return ({js_expr}); }}\n"
                            ),
                            line_offset: line,
                            col_offset: col,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(),
                            strip_scope_prefix: None,
                        });
                        idx += 1;
                    }
                }
            }
            i = expr_end + close_needle_len;
            continue;
        }
        i += 1;
    }
    regions
}

/// Convert a Handlebars expression body to valid JavaScript so the embedded
/// TS/JS parser produces real call refs instead of error-tree noise.
///
/// Examples:
///   `name`                     → `name`
///   `user.name`                → `user.name`
///   `concat "x" this.y`        → `concat("x", this.y)`
///   `action "click" key="v"`   → `action("click", "v")`     (hash key dropped)
///   `(or a b)`                 → `or(a, b)`                  (sub-expr unwrapped)
///   `..`                       → `_HBS_PARENT`               (Handlebars parent context)
///   `@index` / `@key`          → `_HBS_index` / `_HBS_key`   (data variables)
///   `@partial-block`           → empty (skipped — no JS analogue)
fn handlebars_to_js(expr: &str) -> String {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Strip optional leading `~` and trailing `~` (whitespace-control markers).
    let trimmed = trimmed.trim_matches('~').trim();
    if trimmed.is_empty() || trimmed.starts_with("@partial-block") {
        return String::new();
    }
    // Sub-expression form: `(helper a b)` — unwrap and process inside.
    if trimmed.starts_with('(') && trimmed.ends_with(')') {
        return handlebars_to_js(&trimmed[1..trimmed.len() - 1]);
    }
    let tokens = tokenize_hbs(trimmed);
    if tokens.is_empty() {
        return String::new();
    }
    if tokens.len() == 1 {
        return rewrite_token(&tokens[0]);
    }
    let helper = rewrite_token(&tokens[0]);
    let args: Vec<String> = tokens[1..]
        .iter()
        .map(|t| {
            // Hash args `key=value` → take only the value side.
            if let Some(eq) = top_level_eq(t) {
                rewrite_token(&t[eq + 1..])
            } else {
                rewrite_token(t)
            }
        })
        .filter(|s| !s.is_empty())
        .collect();
    if args.is_empty() {
        format!("{helper}()")
    } else {
        format!("{helper}({})", args.join(", "))
    }
}

/// Split a Handlebars expression body on top-level whitespace, preserving
/// quoted strings and parenthesized sub-expressions as single tokens.
fn tokenize_hbs(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut depth: i32 = 0;
    let mut in_dquote = false;
    let mut in_squote = false;
    for ch in s.chars() {
        if in_dquote {
            buf.push(ch);
            if ch == '"' {
                in_dquote = false;
            }
            continue;
        }
        if in_squote {
            buf.push(ch);
            if ch == '\'' {
                in_squote = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_dquote = true;
                buf.push(ch);
            }
            '\'' => {
                in_squote = true;
                buf.push(ch);
            }
            '(' => {
                depth += 1;
                buf.push(ch);
            }
            ')' => {
                depth -= 1;
                buf.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !buf.is_empty() {
                    tokens.push(std::mem::take(&mut buf));
                }
            }
            _ => buf.push(ch),
        }
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }
    tokens
}

/// Find the first `=` outside of quotes/parens (hash-argument separator).
fn top_level_eq(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_dquote = false;
    let mut in_squote = false;
    for (i, ch) in s.char_indices() {
        if in_dquote {
            if ch == '"' { in_dquote = false; }
            continue;
        }
        if in_squote {
            if ch == '\'' { in_squote = false; }
            continue;
        }
        match ch {
            '"' => in_dquote = true,
            '\'' => in_squote = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            '=' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Map a single Handlebars token to a JS-valid expression. Sub-expressions
/// recurse through `handlebars_to_js`.
fn rewrite_token(tok: &str) -> String {
    let t = tok.trim();
    if t.is_empty() {
        return String::new();
    }
    // Quoted string literal — pass through.
    if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
        return t.to_string();
    }
    // Numeric literal, true/false/null, undefined — pass through.
    if t.parse::<f64>().is_ok() || matches!(t, "true" | "false" | "null" | "undefined") {
        return t.to_string();
    }
    // Sub-expression: `(helper args)`.
    if t.starts_with('(') && t.ends_with(')') {
        return handlebars_to_js(&t[1..t.len() - 1]);
    }
    // `..` (parent context) and `../foo` (parent path access).
    if t == ".." {
        return "_HBS_PARENT".to_string();
    }
    if let Some(rest) = t.strip_prefix("../") {
        return format!("_HBS_PARENT.{}", rest.replace('/', "."));
    }
    // `@index`, `@key`, `@first`, `@last`, etc. — Handlebars data variables.
    if let Some(name) = t.strip_prefix('@') {
        return format!("_HBS_{}", name.replace('-', "_"));
    }
    // `this` and `this.x` — JS-compatible.
    if t == "this" || t.starts_with("this.") {
        return t.to_string();
    }
    // Bare identifier or dotted path (`user.name.first`).
    if t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-' || c == '/') {
        // Slashes in paths (`user/name`) are Handlebars-only; flatten to dots.
        // Hyphens make it not a valid JS identifier; rewrite to underscores so
        // tree-sitter sees a single identifier token instead of a subtraction.
        return t.replace('/', ".").replace('-', "_");
    }
    // Anything else (operators, punctuation) — leave as-is and let the JS
    // parser cope. Worst case it produces an error tree, but the host
    // extractor only emits Calls refs from clean Call AST nodes.
    t.to_string()
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
