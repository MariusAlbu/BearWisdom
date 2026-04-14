//! HEEx host extraction — file-stem symbol + `<.component />` Calls refs.

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
    let mut symbols = vec![ExtractedSymbol {
        name: stem.clone(), qualified_name: stem.clone(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    }];
    let _ = &mut symbols; // keep mut to allow future extensions
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Scan for `<.ComponentName` or `<Module.Function` opening tags.
    for (line_no, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'.' {
                let start = i + 2;
                let mut j = start;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'.') {
                    j += 1;
                }
                let name = line.get(start..j).unwrap_or("").to_string();
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: 0,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: line_no as u32,
                        module: None, chain: None,
                    });
                }
                i = j;
                continue;
            }
            i += 1;
        }
    }

    ExtractionResult { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_component_becomes_calls_ref() {
        let src = "<div>\n<.button label=\"go\" />\n</div>";
        let r = extract(src, "form.heex");
        assert!(r.refs.iter().any(|r| r.target_name == "button"));
    }
}
