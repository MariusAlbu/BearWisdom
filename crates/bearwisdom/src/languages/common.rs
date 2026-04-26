// =============================================================================
// languages/common.rs  —  shared extraction utilities used by multiple plugins
//
// Functions here are language-agnostic helpers that would otherwise be
// duplicated across per-language call extractors.  They live here rather than
// in `languages/mod.rs` to keep the plugin registry and trait definitions
// uncluttered.
// =============================================================================

use crate::types::{
    ChainSegment, EmbeddedOrigin, EmbeddedRegion, MemberChain, SegmentKind,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Shared chain builder — language-agnostic, works for any grammar that uses
// the standard tree-sitter JS/TS node kinds (member_expression, identifier,
// call_expression, subscript_expression, this, super).
// ---------------------------------------------------------------------------

/// Build a structured member-access chain from a tree-sitter function node.
///
/// Returns `None` when the node isn't a recognisable chain root (e.g. an
/// anonymous arrow function as the callee, which can't be named).
///
/// Works with both the TypeScript and JavaScript grammars — both grammars
/// share the same node kinds for all patterns covered here.
pub fn build_member_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "this" | "super" => {
            segments.push(ChainSegment {
                name: node_text_bytes(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" | "property_identifier" => {
            segments.push(ChainSegment {
                name: node_text_bytes(node, src),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;

            let is_optional = (0..node.child_count()).any(|i| {
                node.child(i)
                    .map(|c| c.kind() == "optional_chain")
                    .unwrap_or(false)
            });

            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text_bytes(property, src),
                node_kind: property.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: is_optional,
            });
            Some(())
        }

        "subscript_expression" => {
            let object = node.child_by_field_name("object")?;
            let index = node.child_by_field_name("index")?;

            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text_bytes(index, src),
                node_kind: "subscript_expression".to_string(),
                kind: SegmentKind::ComputedAccess,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.b().c()` — walk into the function child.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // Non-chainable node (arrow, conditional, etc.) — abort.
        _ => None,
    }
}

/// Extract text for a node from the raw byte buffer.
fn node_text_bytes(node: Node, src: &[u8]) -> String {
    src.get(node.start_byte()..node.end_byte())
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or("")
        .to_string()
}

/// True when `name` is bound as a parameter of any enclosing JS/TS
/// function in the AST. Shared by both the JavaScript and TypeScript
/// extractors — both grammars use the same node kinds for function-like
/// constructs and their parameter list nodes (formal_parameters,
/// required_parameter, etc.) and destructuring patterns (object_pattern,
/// array_pattern, rest_pattern, assignment_pattern).
///
/// Walks the parent chain from `at` up to the program root. Returns true
/// the first time it finds a function whose parameter list binds `name`.
/// Used to filter ref-emission for chain receivers, callees, and for-loop
/// iterables whose identifier is a local parameter rather than a type or
/// declared function.
pub fn is_enclosing_js_function_parameter(at: Node, src: &[u8], name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut cur = at;
    while let Some(parent) = cur.parent() {
        if matches!(
            parent.kind(),
            "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "generator_function_declaration"
                | "generator_function"
        ) {
            let params = parent
                .child_by_field_name("parameters")
                .or_else(|| parent.child_by_field_name("parameter"));
            if let Some(params) = params {
                if js_parameter_list_binds(params, src, name) {
                    return true;
                }
            }
        }
        cur = parent;
    }
    false
}

fn js_parameter_list_binds(params: Node, src: &[u8], name: &str) -> bool {
    if js_pattern_binds_name(params, src, name) {
        return true;
    }
    let mut cursor = params.walk();
    for child in params.named_children(&mut cursor) {
        if js_pattern_binds_name(child, src, name) {
            return true;
        }
    }
    false
}

fn js_pattern_binds_name(node: Node, src: &[u8], name: &str) -> bool {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => node_text_bytes(node, src) == name,
        "rest_pattern" | "spread_element" => node
            .named_child(0)
            .map(|c| js_pattern_binds_name(c, src, name))
            .unwrap_or(false),
        "assignment_pattern" => node
            .child_by_field_name("left")
            .or_else(|| node.named_child(0))
            .map(|c| js_pattern_binds_name(c, src, name))
            .unwrap_or(false),
        "object_pattern" | "array_pattern" | "object_assignment_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if js_pattern_binds_name(child, src, name) {
                    return true;
                }
            }
            false
        }
        "pair_pattern" => node
            .child_by_field_name("value")
            .map(|c| js_pattern_binds_name(c, src, name))
            .unwrap_or(false),
        "required_parameter" | "optional_parameter" | "formal_parameters" => {
            let inner = node
                .child_by_field_name("pattern")
                .or_else(|| node.named_child(0));
            inner
                .map(|c| js_pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// When a call has a chain (e.g. `Foo::bar()`, `Foo.bar()`, or the nested-
/// namespace form `Stripe.Event.create()`), emit a `TypeRef` for the type
/// prefix — the segment immediately before the final method name — if it
/// looks like a type (starts with uppercase) **AND** the chain root is
/// itself a type / namespace entry point (also uppercase).
///
/// The root-uppercase guard matters because intermediate chain segments
/// with PascalCase names are overwhelmingly property accesses when the
/// root is a lowercase identifier (parameter, local variable, `this`).
/// `item.App.toLowerCase()` has chain `[item, App, toLowerCase]`; without
/// the guard the old logic emitted `App` as a TypeRef — but `App` is a
/// property name on the array-literal element `{ App: string }`, not a
/// type. Those TypeRefs never resolve and pollute `unresolved_refs` with
/// every field access that happens to be PascalCase (see `App`, `Color`,
/// `Name` in fluentui-blazor's ColorsUtils.ts).
///
/// With the guard: `Stripe.Event.create()` still emits `Event` as
/// TypeRef (root `Stripe` is uppercase → a namespace), while
/// `item.App.toLowerCase()` emits nothing from this helper.
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
    let root_seg = &c.segments[0];
    let root_is_type_like = root_seg
        .name
        .chars()
        .next()
        .map_or(false, |ch| ch.is_uppercase());
    if !root_is_type_like {
        return;
    }
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
                    namespace_segments: Vec::new(),
});
    }
}

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
                refs.push(ScriptRef { url, line });
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

    #[test]
    fn script_ref_double_quoted_url() {
        let src = r#"<html><head>
<script src="~/lib/jquery/jquery.js"></script>
</head></html>"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "~/lib/jquery/jquery.js");
        assert_eq!(refs[0].line, 1);
    }

    #[test]
    fn script_ref_single_quoted_url() {
        let src = "<script src='/js/app.js'></script>\n";
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "/js/app.js");
    }

    #[test]
    fn script_ref_unquoted_url() {
        let src = "<script src=lib/foo.js></script>\n";
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "lib/foo.js");
    }

    #[test]
    fn script_ref_with_extra_attrs() {
        // Typical ASP.NET MVC pattern with tag helper before src.
        let src = r#"<script simpl-append-version="true" src="~/lib/bootstrap/dist/js/bootstrap.js"></script>"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "~/lib/bootstrap/dist/js/bootstrap.js");
    }

    #[test]
    fn script_ref_cdn_skipped() {
        let src = r#"
<script src="https://cdn.jsdelivr.net/npm/vue"></script>
<script src="//cdn.example.com/jquery.js"></script>
<script src="http://localhost/foo.js"></script>
<script src="data:application/javascript,console.log(1)"></script>
"#;
        assert!(extract_script_refs(src).is_empty());
    }

    #[test]
    fn inline_script_without_src_ignored() {
        let src = "<script>console.log('hi');</script>";
        assert!(extract_script_refs(src).is_empty());
    }

    #[test]
    fn multiple_script_refs_collected() {
        let src = r#"
<script src="~/lib/jquery/jquery.js"></script>
<script src="~/lib/angular/angular.js"></script>
<script src="/custom/app.js"></script>
"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].url, "~/lib/jquery/jquery.js");
        assert_eq!(refs[1].url, "~/lib/angular/angular.js");
        assert_eq!(refs[2].url, "/custom/app.js");
    }

    #[test]
    fn script_ref_skips_similar_tag_names() {
        // `<scripts>` and `<scriptoid>` must not trigger a match — only
        // `<script` followed by whitespace, `>`, or `/`.
        let src = r#"
<scripts src="foo.js"></scripts>
<scriptoid src="bar.js"></scriptoid>
"#;
        assert!(extract_script_refs(src).is_empty());
    }

    #[test]
    fn script_ref_case_insensitive_tag() {
        let src = r#"<SCRIPT SRC="lib/foo.js"></SCRIPT>"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "lib/foo.js");
    }

    #[test]
    fn script_ref_survives_razor_at_syntax() {
        // Razor files mix `@...` directives with HTML; our byte scan must
        // not choke on them.
        let src = r#"@{
    Layout = "_Layout";
}
<script src="~/lib/jquery/jquery.js"></script>
@section Scripts {
    <script src="~/js/page.js"></script>
}"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 2);
    }
}
