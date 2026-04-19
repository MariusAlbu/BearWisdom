// =============================================================================
// languages/common.rs  —  shared extraction utilities used by multiple plugins
//
// Functions here are language-agnostic helpers that would otherwise be
// duplicated across per-language call extractors.  They live here rather than
// in `languages/mod.rs` to keep the plugin registry and trait definitions
// uncluttered.
// =============================================================================

use crate::types::{EmbeddedOrigin, EmbeddedRegion};
use tree_sitter::{Node, Parser};

/// When a call has a chain (e.g. `Foo::bar()`, `Foo.bar()`), emit a `TypeRef`
/// for the type prefix — the segment before the final method name — if it
/// looks like a type (starts with uppercase).
pub fn emit_chain_type_ref(
    chain: &Option<crate::types::MemberChain>,
    source_symbol_index: usize,
    func_node: &tree_sitter::Node,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    let c = match chain.as_ref() {
        Some(c) if c.segments.len() >= 2 => c,
        _ => return,
    };
    let type_seg = &c.segments[c.segments.len() - 2];
    if type_seg.name.chars().next().map_or(false, |ch| ch.is_uppercase()) {
        refs.push(crate::types::ExtractedRef {
            source_symbol_index,
            target_name: type_seg.name.clone(),
            kind: crate::types::EdgeKind::TypeRef,
            line: func_node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_vue_sfc_script_and_style_blocks() {
        let src = "<template>\n  <div>Hello</div>\n</template>\n\n<script setup lang=\"ts\">\nimport { ref } from 'vue'\nconst count = ref(0)\n</script>\n\n<style lang=\"scss\" scoped>\n.foo { color: red; }\n</style>\n";
        let regions = extract_html_script_style_regions(src);
        assert_eq!(regions.len(), 2, "expected one script + one style region");

        let script = &regions[0];
        assert_eq!(script.language_id, "typescript");
        assert_eq!(script.origin, EmbeddedOrigin::ScriptBlock);
        assert!(script.text.contains("import { ref } from 'vue'"));
        assert!(script.text.contains("const count = ref(0)"));
        // tree-sitter-html's `raw_text` node begins immediately after the
        // start tag's `>`, so it starts at the trailing newline on the same
        // line as `<script setup lang="ts">` (line 4). The sub-extracted
        // `import` line is at region-line 1, which the dispatcher rewrites
        // to file-line 5.
        assert_eq!(script.line_offset, 4);

        let style = &regions[1];
        assert_eq!(style.language_id, "scss");
        assert_eq!(style.origin, EmbeddedOrigin::StyleBlock);
        assert!(style.text.contains(".foo { color: red; }"));
    }

    #[test]
    fn plain_script_defaults_to_javascript() {
        let src = "<template><p/></template>\n<script>\nconsole.log('hi')\n</script>\n";
        let regions = extract_html_script_style_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
    }

    #[test]
    fn json_ld_script_is_skipped() {
        // application/ld+json is not executable JavaScript; sub-dispatch would
        // treat it as JS and emit garbage. The helper must drop it.
        let src = "<script type=\"application/ld+json\">{\"@context\":\"https://schema.org\"}</script>\n";
        let regions = extract_html_script_style_regions(src);
        assert!(regions.is_empty(), "ld+json must be skipped, not sub-parsed");
    }

    #[test]
    fn unsupported_style_lang_is_skipped() {
        // less / stylus aren't wired to any plugin yet — skip rather than
        // hand the text to the CSS extractor and produce wrong results.
        let src = "<style lang=\"less\">.foo { color: red; }</style>\n";
        let regions = extract_html_script_style_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn line_offset_matches_raw_text_start_for_multiline_file() {
        // Line offsets point at tree-sitter's `raw_text` node, which starts
        // immediately after the opening tag's `>` — on the same line as
        // `<script>`. The sub-extractor sees a region whose first character
        // is `\n`, so `let x = 1` sits on region-line 1; adding the
        // line_offset of 3 resolves it back to file-line 4.
        let src = "<template>\n  <div/>\n</template>\n<script lang=\"ts\">\nlet x = 1\n</script>\n";
        let regions = extract_html_script_style_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].line_offset, 3);
    }

    #[test]
    fn extracts_astro_frontmatter_block() {
        let src = "---\nimport Layout from '../layouts/Layout.astro';\nconst title = 'Home';\n---\n<Layout title={title}>\n  <h1>Hello</h1>\n</Layout>\n";
        let fm = extract_astro_frontmatter(src).expect("frontmatter");
        assert_eq!(fm.language_id, "typescript");
        assert_eq!(fm.origin, EmbeddedOrigin::Frontmatter);
        assert!(fm.text.contains("import Layout"));
        assert!(fm.text.contains("const title = 'Home';"));
        // Opening fence `---\n` is line 0; body starts on line 1.
        assert_eq!(fm.line_offset, 1);
    }

    #[test]
    fn missing_astro_frontmatter_returns_none() {
        let src = "<h1>No frontmatter here</h1>\n";
        assert!(extract_astro_frontmatter(src).is_none());
    }

    #[test]
    fn astro_frontmatter_respects_leading_whitespace() {
        // Astro allows (ignores) leading newlines before the opening fence.
        let src = "\n\n---\nconst x = 1;\n---\n<p/>\n";
        let fm = extract_astro_frontmatter(src).expect("frontmatter");
        assert!(fm.text.contains("const x = 1;"));
    }
}
