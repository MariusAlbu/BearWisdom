//! Builtin and keyword names harvested from a tree-sitter `highlights.scm`:
//! string literals in `[...] @keyword` / `[...] @*.builtin` blocks, inline
//! `"word" @keyword` / `"word" @*.builtin` captures, and the alternatives of
//! `#match?` / `#eq?` predicates on `@*.builtin` captures.

use std::collections::BTreeSet;

pub(crate) fn extract(content: &str, names: &mut BTreeSet<String>) {
    extract_match_predicates(content, names);
    extract_eq_predicates(content, names);
    extract_keyword_strings(content, names);
    extract_builtin_strings(content, names);
}

fn extract_match_predicates(content: &str, names: &mut BTreeSet<String>) {
    let re = regex::Regex::new(r#"#match\?\s+@\w+(?:\.\w+)?\s+"[^^]*\^?\(([^)]+)\)\$?""#).unwrap();
    for cap in re.captures_iter(content) {
        if let Some(alt) = cap.get(1) {
            for name in alt.as_str().split('|') {
                let name = name.trim();
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
    }
}

fn extract_eq_predicates(content: &str, names: &mut BTreeSet<String>) {
    let re = regex::Regex::new(r#"#eq\?\s+@\w+(?:\.\w+)?\s+"([^"]+)""#).unwrap();
    for cap in re.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
}

fn extract_keyword_strings(content: &str, names: &mut BTreeSet<String>) {
    // Inline: "word" @keyword
    let re = regex::Regex::new(r#""([a-zA-Z_][a-zA-Z0-9_!?]*(?:::)?[a-zA-Z0-9_!?]*)"\s+@keyword"#)
        .unwrap();
    for cap in re.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
    // Bracket blocks: [...] @keyword
    extract_bracket_block_names(content, "@keyword", names);
}

fn extract_builtin_strings(content: &str, names: &mut BTreeSet<String>) {
    // Inline: "word" @*.builtin
    let re = regex::Regex::new(r#""([a-zA-Z_][a-zA-Z0-9_!?]*)"\s+@\w+\.builtin"#).unwrap();
    for cap in re.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
    // Bracket blocks: [...] @type.builtin, [...] @constant.builtin, etc.
    for tag in [
        "@type.builtin",
        "@constant.builtin",
        "@function.builtin",
        "@variable.builtin",
    ] {
        extract_bracket_block_names(content, tag, names);
    }
}

/// Extract quoted names from `[ "w1" "w2" ... ] @tag` blocks.
fn extract_bracket_block_names(content: &str, tag: &str, names: &mut BTreeSet<String>) {
    let re_quoted = regex::Regex::new(r#""([a-zA-Z_][a-zA-Z0-9_!?]*)""#).unwrap();
    let needle = format!("] {}", tag);
    let mut pos = 0;
    while let Some(found) = content[pos..].find(&needle) {
        let abs = pos + found;
        if let Some(open) = content[..abs].rfind('[') {
            for cap in re_quoted.captures_iter(&content[open..abs]) {
                if let Some(name) = cap.get(1) {
                    names.insert(name.as_str().to_string());
                }
            }
        }
        pos = abs + needle.len();
    }
}
