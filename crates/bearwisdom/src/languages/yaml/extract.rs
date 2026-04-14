//! YAML symbol extraction — minimal file-level descriptor + top-level keys.
//!
//! Top-level mapping keys become Field symbols so a consumer can ask
//! "does this file define a `jobs` key" / "does this pipeline set
//! `stages`". Nested structure is not surfaced — that's what the CI
//! platform docs are for.

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

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

    ExtractionResult::new(symbols, Vec::new(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_keys_become_fields() {
        let src = "name: CI\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n";
        let r = extract(src, "/a/.github/workflows/ci.yml");
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"on"));
        assert!(names.contains(&"jobs"));
        // Nested keys (build, runs-on) are NOT surfaced.
        assert!(!names.contains(&"build"));
        assert!(!names.contains(&"runs-on"));
    }

    #[test]
    fn comments_and_indent_ignored() {
        let src = "# header comment\nname: CI\n  nested: true\n";
        let r = extract(src, "ci.yml");
        let keys: Vec<&str> = r.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Field)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(keys, vec!["name"]);
    }
}
