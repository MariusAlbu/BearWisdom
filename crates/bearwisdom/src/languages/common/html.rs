// =============================================================================
// languages/common/html.rs — script-tag refs, HTML embedded regions, Astro frontmatter
// =============================================================================

use crate::types::{EmbeddedOrigin, EmbeddedRegion};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Script-tag `src=` extraction — works on any HTML-like source
// (HTML, Razor, cshtml, Vue, Svelte, Astro, ERB, Blade, etc.)
// ---------------------------------------------------------------------------

/// A `<script src="…">` reference discovered in an HTML-dialect source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRef {
    /// Raw URL as it appears in the `src` attribute (before any `~/` → webroot
    /// rewriting — that's the indexer's job, not the extractor's).
    pub url: String,
    /// 0-based line of the opening `<script` tag.
    pub line: u32,
    /// Byte offset of the `<script` token within the full source string.
    pub byte_offset: u32,
}

/// Scan `source` for every `<script … src="…" …></script>` (or self-closing)
/// tag and return the referenced URLs.
///
/// Byte-level scan — deliberately does not go through tree-sitter-html so
/// Razor / cshtml / Blade / ERB files with `@`, `{{`, `<%%>` syntax don't
/// trip the HTML parser. Case-insensitive tag match. Handles double-quoted,
/// single-quoted, and unquoted attribute values.
///
/// Skips:
///   * Absolute URLs (`http://…`, `https://…`, `//cdn.example.com/…`) — these
///     are CDN references, not filesystem paths.
///   * `data:` URIs.
///
/// Inline `<script>…</script>` blocks (with no `src`) are ignored here —
/// the embedded-region pipeline handles those.
pub fn extract_script_refs(source: &str) -> Vec<ScriptRef> {
    let bytes = source.as_bytes();
    let mut refs = Vec::new();
    let mut line: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if !case_insensitive_prefix(bytes, i + 1, b"script") {
            i += 1;
            continue;
        }
        // Must be followed by a tag-boundary char (whitespace, `>`, or `/`).
        let after = bytes.get(i + 7).copied();
        if !matches!(after, Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/')) {
            i += 1;
            continue;
        }
        let tag_start = i;
        let Some(tag_end) = memchr_byte(bytes, tag_start + 7, b'>') else {
            break;
        };
        let attrs = &bytes[tag_start + 7..tag_end];
        if let Some(url) = find_attribute_value(attrs, b"src") {
            if is_extractable_script_url(&url) {
                refs.push(ScriptRef {
                    url,
                    line,
                    byte_offset: tag_start as u32,
                });
            }
        }
        // Advance past the tag; line counter picks up newlines inside the tag.
        i = tag_end + 1;
    }
    refs
}

fn case_insensitive_prefix(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > bytes.len() {
        return false;
    }
    bytes[start..start + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn memchr_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    (start..bytes.len()).find(|&i| bytes[i] == needle)
}

/// Scan a slice of attribute bytes for `name=VALUE` and return the value.
/// Returns `None` when the attribute isn't present.
fn find_attribute_value(attrs: &[u8], name: &[u8]) -> Option<String> {
    let mut i = 0;
    while i < attrs.len() {
        // Skip whitespace.
        while i < attrs.len() && attrs[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= attrs.len() {
            break;
        }
        let name_start = i;
        while i < attrs.len()
            && !attrs[i].is_ascii_whitespace()
            && attrs[i] != b'='
            && attrs[i] != b'/'
        {
            i += 1;
        }
        let attr_name = &attrs[name_start..i];
        // Skip whitespace before `=`.
        while i < attrs.len() && attrs[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= attrs.len() || attrs[i] != b'=' {
            // Valueless attribute.
            continue;
        }
        i += 1;
        // Skip whitespace after `=`.
        while i < attrs.len() && attrs[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= attrs.len() {
            break;
        }
        let value = match attrs[i] {
            b'"' | b'\'' => {
                let quote = attrs[i];
                i += 1;
                let v_start = i;
                while i < attrs.len() && attrs[i] != quote {
                    i += 1;
                }
                let v = std::str::from_utf8(&attrs[v_start..i]).ok()?.to_string();
                if i < attrs.len() {
                    i += 1;
                } // past closing quote
                v
            }
            _ => {
                let v_start = i;
                while i < attrs.len() && !attrs[i].is_ascii_whitespace() {
                    i += 1;
                }
                let v = std::str::from_utf8(&attrs[v_start..i]).ok()?.to_string();
                // Self-closing tag: `<script src=foo.js/>` — strip the
                // trailing `/` that belongs to the tag, not the value.
                v.strip_suffix('/').map(str::to_string).unwrap_or(v)
            }
        };
        if attr_name.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

/// True when a `src` URL points at a repo-local file that the indexer can
/// resolve. Filters out CDN URLs, data URIs, and obviously non-path values.
fn is_extractable_script_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() {
        return false;
    }
    if u.starts_with("http://")
        || u.starts_with("https://")
        || u.starts_with("//")
        || u.starts_with("data:")
        || u.starts_with("javascript:")
        || u.starts_with("blob:")
    {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Embedded-region extraction for HTML-dialect host files
// (Svelte / Vue / Astro / HTML — Razor uses its own regex splitter)
// ---------------------------------------------------------------------------

/// Parse `source` as HTML and return an `EmbeddedRegion` for every top-level
/// `<script>` and `<style>` block. The `language_id` of each region is derived
/// from the block's `lang` / `type` attribute (or the sensible default):
///
///   * `<script>`                        → `"javascript"`
///   * `<script lang="ts">`              → `"typescript"`
///   * `<script lang="tsx">`             → `"typescript"` (tsx variant)
///   * `<script type="application/ld+json">` → skipped (not executable code)
///   * `<style>`                         → `"css"`
///   * `<style lang="scss">`             → `"scss"`
///   * `<style lang="sass">`             → `"scss"` (sass maps to scss plugin)
///   * `<style lang="less">` / `"stylus"` → skipped (no plugin yet)
///
/// Used by Svelte, Vue, and Astro host extractors. Astro additionally calls
/// `extract_astro_frontmatter` to pick up the `---`-delimited TS block at the
/// top of the file.
pub fn extract_html_script_style_regions(source: &str) -> Vec<EmbeddedRegion> {
    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let mut regions = Vec::new();
    collect_blocks(&tree.root_node(), source, &mut regions);
    regions
}

/// Recursive walker for the HTML tree; appends script/style blocks.
/// Only top-level script/style elements matter for SFC extractors, but we
/// walk the whole tree so nested `<template>`s inside Vue are handled.
fn collect_blocks(node: &Node, source: &str, regions: &mut Vec<EmbeddedRegion>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "script_element" || kind == "style_element" {
            if let Some(region) = build_region_from_element(&child, source) {
                regions.push(region);
            }
            // Don't recurse into the block — its content is raw text handled
            // by the sub-extractor, not more HTML.
            continue;
        }
        if kind == "element" {
            collect_blocks(&child, source, regions);
        }
    }
}

/// Build an `EmbeddedRegion` from a `script_element` or `style_element`.
/// Returns `None` when the block has no content, the `lang`/`type` attribute
/// maps to an unsupported sub-language, or the element has no body (e.g.
/// `<script src="…">` with no inline text).
fn build_region_from_element(element: &Node, source: &str) -> Option<EmbeddedRegion> {
    let is_script = element.kind() == "script_element";
    let is_style = element.kind() == "style_element";
    if !is_script && !is_style {
        return None;
    }

    // Find <start_tag>, <raw_text>, and <end_tag> children.
    let mut start_tag: Option<Node> = None;
    let mut raw_text: Option<Node> = None;
    let mut cursor = element.walk();
    for child in element.children(&mut cursor) {
        match child.kind() {
            "start_tag" => start_tag = Some(child),
            "raw_text" => raw_text = Some(child),
            _ => {}
        }
    }
    let start_tag = start_tag?;
    let raw_text = raw_text?;

    // Parse the `lang` (SFC style) or `type` (plain HTML) attribute off the
    // start tag to pick the sub-extractor. Default is JS for script, CSS for
    // style.
    let lang_attr = read_attribute(&start_tag, source, "lang");
    let type_attr = read_attribute(&start_tag, source, "type");
    let language_id = if is_script {
        match lang_attr.as_deref() {
            Some("ts") | Some("typescript") | Some("tsx") => "typescript",
            Some("js") | Some("javascript") | Some("mjs") | None => {
                // `type="application/ld+json"`, `type="text/x-template"`, etc.
                // are not executable JS — skip.
                match type_attr.as_deref() {
                    None
                    | Some("text/javascript")
                    | Some("application/javascript")
                    | Some("module") => "javascript",
                    _ => return None,
                }
            }
            _ => return None, // unknown script lang (coffee, livescript, …)
        }
    } else {
        match lang_attr.as_deref() {
            Some("scss") | Some("sass") => "scss",
            Some("css") | None => "css",
            _ => return None, // less / stylus / postcss — no plugin yet
        }
    };

    // Slice the file text for the raw body and capture start position.
    let start_byte = raw_text.start_byte();
    let end_byte = raw_text.end_byte();
    if end_byte <= start_byte {
        return None;
    }
    let text = source.get(start_byte..end_byte)?.to_string();
    let start_pos = raw_text.start_position();

    Some(EmbeddedRegion {
        language_id: language_id.to_string(),
        text,
        line_offset: start_pos.row as u32,
        col_offset: start_pos.column as u32,
        origin: if is_script {
            EmbeddedOrigin::ScriptBlock
        } else {
            EmbeddedOrigin::StyleBlock
        },
        holes: Vec::new(),
        strip_scope_prefix: None,
    })
}

/// Read an attribute value from a `start_tag` node. Returns `None` when the
/// attribute isn't present or its value is unquoted in a way we don't parse.
fn read_attribute(start_tag: &Node, source: &str, name: &str) -> Option<String> {
    let mut cursor = start_tag.walk();
    for child in start_tag.children(&mut cursor) {
        if child.kind() != "attribute" {
            continue;
        }
        let mut attr_cursor = child.walk();
        let mut got_name = false;
        let mut value: Option<String> = None;
        for attr_child in child.children(&mut attr_cursor) {
            match attr_child.kind() {
                "attribute_name" => {
                    let n = source.get(attr_child.start_byte()..attr_child.end_byte())?;
                    got_name = n.eq_ignore_ascii_case(name);
                }
                "quoted_attribute_value" => {
                    // <quoted_attribute_value> has one child: <attribute_value>
                    let mut v_cursor = attr_child.walk();
                    for v_child in attr_child.children(&mut v_cursor) {
                        if v_child.kind() == "attribute_value" {
                            value = source
                                .get(v_child.start_byte()..v_child.end_byte())
                                .map(str::to_string);
                        }
                    }
                }
                "attribute_value" => {
                    value = source
                        .get(attr_child.start_byte()..attr_child.end_byte())
                        .map(str::to_string);
                }
                _ => {}
            }
        }
        if got_name {
            return value;
        }
    }
    None
}

/// Extract the Astro frontmatter region — the `---`-delimited TypeScript
/// block that must appear as the first non-whitespace content of an `.astro`
/// file. Returns `None` when no frontmatter is present.
///
/// The returned region uses `language_id = "typescript"` with
/// `EmbeddedOrigin::Frontmatter`. Line/column offsets point at the first
/// character after the opening `---\n`, matching Astro's own semantics.
pub fn extract_astro_frontmatter(source: &str) -> Option<EmbeddedRegion> {
    // Skip leading whitespace / blank lines before the opening fence.
    let trimmed_start = source
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(0);
    let rest = &source[trimmed_start..];
    if !rest.starts_with("---") {
        return None;
    }
    // Opening fence is `---` followed by (optionally) the rest of the line and
    // a newline. Find the end of the opening fence line.
    let after_open_fence = rest.find('\n')? + 1;
    let body_start_in_rest = after_open_fence;
    let body_slice = &rest[body_start_in_rest..];
    // Find the closing fence: a line that is exactly `---` (possibly with
    // trailing whitespace). Match at line start.
    let closing_rel = find_closing_fence(body_slice)?;
    let body_text = &body_slice[..closing_rel];

    // Compute the absolute line/col of the body start in the original source.
    let body_abs_byte = trimmed_start + body_start_in_rest;
    let (line_offset, col_offset) = byte_to_line_col(source, body_abs_byte);

    Some(EmbeddedRegion {
        language_id: "typescript".to_string(),
        text: body_text.to_string(),
        line_offset,
        col_offset,
        origin: EmbeddedOrigin::Frontmatter,
        holes: Vec::new(),
        strip_scope_prefix: None,
    })
}

/// Find the byte offset of the `---` closing fence at the start of a line
/// within `body`. Returns `None` when no closing fence exists.
fn find_closing_fence(body: &str) -> Option<usize> {
    // Iterate lines with their starting offsets.
    let mut offset = 0usize;
    for line in body.split_inclusive('\n') {
        let content_end = line.trim_end_matches(['\r', '\n']).len();
        let line_content = &line[..content_end];
        if line_content == "---" {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

/// Translate a byte offset inside `source` into a 0-based (line, column) pair.
/// Column is measured in bytes for now; callers downstream compare against
/// tree-sitter point columns which are also byte-counted.
fn byte_to_line_col(source: &str, byte: usize) -> (u32, u32) {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = match prefix.rfind('\n') {
        Some(nl) => (byte - nl - 1) as u32,
        None => byte as u32,
    };
    (line, col)
}
