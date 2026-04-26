//! Shared Markdown host-scan logic — extracts file-stem symbol, ATX
//! headings, relative link refs, and fence anchor symbols.
//!
//! Used by both the Markdown plugin and the MDX plugin. MDX layers JSX
//! component refs and ES import/export regions on top of this baseline.

use super::fenced;
use super::info_string;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility,
};

/// Result of a shared host scan. `host_index` is the index of the
/// file-level host symbol in `symbols` — callers that layer extra refs
/// on top (e.g. MDX JSX refs) use it as the `source_symbol_index`.
pub struct HostScan {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub host_index: usize,
    pub file_stem: String,
}

pub fn scan(source: &str, file_path: &str) -> HostScan {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let file_name = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name.clone(),
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
    });
    let host_index: usize = 0;

    let bytes = source.as_bytes();
    let mut line_no: u32 = 0;
    let mut ls = 0usize;
    while ls < bytes.len() {
        let le = bytes[ls..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| ls + p)
            .unwrap_or(bytes.len());
        let line = &bytes[ls..le];
        if let Some((level, text)) = parse_atx_heading(line) {
            symbols.push(ExtractedSymbol {
                name: text.clone(),
                qualified_name: format!("{file_name}.{}", slugify(&text)),
                kind: SymbolKind::Field,
                visibility: Some(Visibility::Public),
                start_line: line_no,
                end_line: line_no,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("h{level}")),
                doc_comment: None,
                scope_path: Some(file_name.clone()),
                parent_index: Some(host_index),
            });
        }
        collect_link_refs(line, line_no, host_index, &mut refs);
        line_no += 1;
        ls = le + 1;
    }

    for (idx, fence) in fenced::parse_fences(source).iter().enumerate() {
        let lang = info_string::normalize(&fence.info).unwrap_or("text");
        let anchor = format!("{lang}#{idx}");
        symbols.push(ExtractedSymbol {
            name: anchor.clone(),
            qualified_name: format!("{file_name}.{anchor}"),
            kind: SymbolKind::Class,
            visibility: Some(Visibility::Public),
            start_line: fence.body_line_offset,
            end_line: fence.body_line_offset
                + fence.body.matches('\n').count() as u32,
            start_col: 0,
            end_col: 0,
            signature: Some(fence.info.clone()),
            doc_comment: None,
            scope_path: Some(file_name.clone()),
            parent_index: Some(host_index),
        });
    }

    HostScan {
        symbols,
        refs,
        host_index,
        file_stem: file_name,
    }
}

fn parse_atx_heading(line: &[u8]) -> Option<(u32, String)> {
    let mut i = 0;
    while i < line.len() && i < 3 && line[i] == b' ' {
        i += 1;
    }
    let mut level = 0u32;
    while i < line.len() && line[i] == b'#' && level < 6 {
        level += 1;
        i += 1;
    }
    if level == 0 {
        return None;
    }
    if i < line.len() && line[i] != b' ' && line[i] != b'\t' {
        return None;
    }
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    let mut end = line.len();
    while end > i && (line[end - 1] == b' ' || line[end - 1] == b'\t' || line[end - 1] == b'\r') {
        end -= 1;
    }
    while end > i && line[end - 1] == b'#' {
        end -= 1;
    }
    while end > i && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
        end -= 1;
    }
    let text = std::str::from_utf8(&line[i..end]).ok()?.to_string();
    if text.is_empty() {
        return None;
    }
    Some((level, text))
}

fn collect_link_refs(
    line: &[u8],
    line_no: u32,
    host_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let s = match std::str::from_utf8(line) {
        Ok(s) => s,
        Err(_) => return,
    };
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' || (chars[i] == '!' && chars.get(i + 1) == Some(&'[')) {
            let is_image = chars[i] == '!';
            let open = if is_image { i + 1 } else { i };
            if let Some(close) = find_match_bracket(&chars, open) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(paren_close) = find_match_paren(&chars, close + 1) {
                        if !is_image {
                            let target: String = chars[close + 2..paren_close].iter().collect();
                            if let Some(normalized) = normalize_link_target(&target) {
                                refs.push(ExtractedRef {
                                    source_symbol_index: host_index,
                                    target_name: normalized,
                                    kind: EdgeKind::Imports,
                                    line: line_no,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                        i = paren_close + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
}

fn find_match_bracket(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn find_match_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn normalize_link_target(target: &str) -> Option<String> {
    let target = target.split_whitespace().next()?;
    if target.is_empty() || target.starts_with('#') {
        return None;
    }
    if target.contains("://") || target.starts_with("mailto:") {
        return None;
    }
    // Site-absolute routes (e.g. Blazor `/WhatsNew-Archive`, Docusaurus
    // `/docs/foo`) are not filesystem paths — they route against a site
    // root, not the repo root. Skip them.
    if target.starts_with('/') {
        return None;
    }
    let mut t = target;
    if let Some(stripped) = t.strip_prefix("./") {
        t = stripped;
    }
    if let Some(pos) = t.find('#') {
        t = &t[..pos];
    }
    if t.is_empty() {
        return None;
    }
    // Bare identifiers like `caDocsUrl` are template placeholders, not
    // file paths — a real file ref has either a path separator or an
    // extension. Reject tokens with neither.
    if !t.contains('/') && !t.contains('\\') && !t.contains('.') {
        return None;
    }
    let path = std::path::Path::new(t);
    // Skip links to non-indexed asset types. Images are already filtered
    // at the `![...]()` syntax level, but bare `[...](foo.png)` still
    // lands here — same rationale applies.
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if is_non_indexable_asset(&ext.to_ascii_lowercase()) {
            return None;
        }
    }
    // Docs-site route URLs (Docusaurus, VitePress, MkDocs, etc.) render
    // as `./appendix/emojis` or `./guides/overview` in READMEs at the
    // repo root. They resolve to generated-site paths, not to any file
    // in the repo, so stem-matching against indexed files always misses
    // and the ref just pollutes `unresolved_refs`. Heuristic: if the
    // path has no extension AND is not a parent-relative reference
    // (`../` prefix, which is almost always a real file ref), treat it
    // as a site URL and skip. Parent-relative forms like `../CHANGELOG`
    // stay: they reliably target repo files.
    if path.extension().is_none()
        && !target.starts_with("../")
        && !target.starts_with("..\\")
    {
        return None;
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(t);
    let parent = path
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("");
    let normalized = if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{}/{}", parent.replace('\\', "/"), stem)
    };
    Some(normalized)
}

fn is_non_indexable_asset(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" | "tiff" | "tif"
        | "pdf" | "mp4" | "webm" | "mov" | "avi" | "mkv"
        | "mp3" | "wav" | "ogg" | "flac"
        | "zip" | "tar" | "gz" | "7z" | "rar"
        | "exe" | "dll" | "so" | "dylib"
        | "woff" | "woff2" | "ttf" | "otf" | "eot"
    )
}

pub(crate) fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    stem.to_string()
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
