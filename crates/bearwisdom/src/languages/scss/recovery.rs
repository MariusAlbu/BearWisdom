//! Text-scan recovery for SCSS / Sass declarations.
//!
//! Activated when the tree-sitter parse degrades to an ERROR node (any file
//! containing a construct the SCSS grammar can't handle) and as the only
//! extraction path for indented `.sass` files. Captures symbols the
//! grammar-driven walker would otherwise miss.

use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

/// Byte-level scan for `@mixin NAME` / `@function NAME` declarations.
/// Called only when the tree-sitter parse fails catastrophically and
/// zero structured symbols were extracted — a defensible fallback, not
/// a replacement for the grammar-driven path.
pub(super) fn recover_mixin_symbols_from_text(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    for (kind_label, at_keyword) in [("@mixin", "@mixin"), ("@function", "@function")] {
        let mut line_no: u32 = 0;
        let mut last_nl: usize = 0;
        let bytes = source.as_bytes();
        let kw = at_keyword.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                line_no += 1;
                last_nl = i + 1;
                i += 1;
                continue;
            }
            // Match the @keyword only at a line start (after whitespace) so
            // we don't pick up `@mixin` appearing in a selector string.
            if bytes[i] == b'@' && bytes.len() - i >= kw.len() && &bytes[i..i + kw.len()] == kw {
                // Require that the previous non-space char on the line is
                // either nothing (start of line) or whitespace — i.e. this
                // `@` begins a statement.
                let mut j = i;
                while j > last_nl {
                    let prev = bytes[j - 1];
                    if prev == b' ' || prev == b'\t' {
                        j -= 1;
                    } else {
                        break;
                    }
                }
                if j != last_nl {
                    i += 1;
                    continue;
                }
                // Skip the keyword and any trailing whitespace.
                let mut k = i + kw.len();
                while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                // Name runs until `(`, `{`, whitespace, or end-of-line.
                let name_start = k;
                while k < bytes.len()
                    && bytes[k] != b'('
                    && bytes[k] != b'{'
                    && bytes[k] != b' '
                    && bytes[k] != b'\t'
                    && bytes[k] != b'\r'
                    && bytes[k] != b'\n'
                {
                    k += 1;
                }
                if k > name_start {
                    let name = &source[name_start..k];
                    // Skip if already captured via the grammar path (the
                    // guard at call-site ensures the first pass emitted
                    // zero symbols, so this is cheap insurance).
                    let already = symbols.iter().any(|s| s.name == name);
                    if !already {
                        symbols.push(ExtractedSymbol {
                            name: name.to_string(),
                            qualified_name: name.to_string(),
                            kind: SymbolKind::Function,
                            visibility: Some(Visibility::Public),
                            start_line: line_no,
                            end_line: line_no,
                            start_col: (i - last_nl) as u32,
                            end_col: (k - last_nl) as u32,
                            signature: Some(format!("{kind_label} {name}")),
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
                i = k;
                continue;
            }
            i += 1;
        }
    }
}

/// Byte-level scan for top-level `.class-name {` rule definitions.
///
/// Called alongside `recover_mixin_symbols_from_text` when the tree has
/// parse errors — typically files that mix CSS custom property declarations
/// using `#{$variable}` interpolation inside the first rule block, which
/// causes the grammar to produce a root ERROR node that swallows subsequent
/// clean rules. Only matches lines where the dot is the first non-whitespace
/// character and the identifier contains no interpolation, so nested rules
/// (`&.modifier`) and dynamic selectors are not captured.
pub(super) fn recover_class_symbols_from_text(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        // Only lines that start a clean `.class-name {` or `.class-name{` rule.
        if !trimmed.starts_with('.') {
            continue;
        }
        let rest = &trimmed[1..];
        // Name runs until `{`, `,`, `:`, whitespace, or end-of-line.
        let name: &str = rest.split(|c: char| {
            c == '{' || c == ',' || c == ':' || c == ' ' || c == '\t' || c == '\r'
        }).next().unwrap_or("");
        if name.is_empty() {
            continue;
        }
        // Reject names that contain SCSS interpolation or look like property
        // values, pseudo-elements, or other non-identifier fragments.
        if name.contains('#') || name.contains('$') || name.contains('(')
            || name.contains(')') || name.contains('[') || name.contains('/')
            || name.contains('\\')
        {
            continue;
        }
        // The line must end (after the name and optional whitespace) with `{`
        // or a comma to be a selector, not a CSS property value that starts
        // with a dot by coincidence.
        let after_name = &rest[name.len()..];
        let after_trimmed = after_name.trim_start();
        if !after_trimmed.starts_with('{') && !after_trimmed.starts_with(',') {
            continue;
        }
        let already = symbols.iter().any(|s| s.name == name);
        if !already {
            let col = line.len() - trimmed.len();
            symbols.push(ExtractedSymbol {
                name: name.to_string(),
                qualified_name: name.to_string(),
                kind: SymbolKind::Class,
                visibility: Some(Visibility::Public),
                start_line: line_no as u32,
                end_line: line_no as u32,
                start_col: col as u32,
                end_col: (col + 1 + name.len()) as u32,
                signature: Some(format!(".{name}")),
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
}

/// Text-scan for indented Sass `=mixin-name` declarations.
///
/// The indented Sass syntax uses `=name` for mixin definitions instead of
/// `@mixin name { }`. The SCSS grammar does not handle this form, so `.sass`
/// files run this scan alongside `recover_mixin_symbols_from_text`.
pub(super) fn recover_sass_indented_symbols_from_text(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('=') {
            continue;
        }
        let rest = &trimmed[1..];
        // Name runs until `(`, whitespace, or end-of-line.
        let name: String = rest
            .chars()
            .take_while(|&c| c != '(' && c != ' ' && c != '\t' && c != '\r')
            .collect();
        if name.is_empty() {
            continue;
        }
        let already = symbols.iter().any(|s| s.name == name);
        if !already {
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name.clone(),
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: line_no as u32,
                end_line: line_no as u32,
                start_col: (line.len() - trimmed.len()) as u32,
                end_col: (line.len() - trimmed.len() + 1 + name.len()) as u32,
                signature: Some(format!("={name}")),
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
}
