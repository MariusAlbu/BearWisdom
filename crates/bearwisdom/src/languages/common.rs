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

// ---------------------------------------------------------------------------
// Handlebars helper-export detection (Ember + Ghost-style themes)
//
// Helpers and modifiers invoked from Handlebars templates are stored on
// disk as JavaScript / TypeScript modules whose default export wraps a
// helper-callable. The invocation name is the file stem (kebab → snake by
// the Handlebars→JS wrapper), but the inner function name doesn't match
// it. Without injecting a synthetic file-stem symbol, every template call
// like `{{gh-pluralize ...}}` lands in unresolved_refs.
//
// Three patterns are detected, all gated on per-file content signals to
// avoid false-positive matches in unrelated `helpers/` directories:
//
//   1. Ember helper:    `**/app/helpers/<name>.{js,ts,gjs,gts}`
//                       with `@ember/component/helper` or `template-only`.
//   2. Ember modifier:  `**/app/modifiers/<name>.{js,ts,gjs,gts}`
//                       with `@ember/component/modifier` or `@ember/render-modifiers`.
//   3. Ghost theme:     `**/<...>/helpers/<name>.{js,ts}` (any depth)
//                       with `require('...handlebars...')` or `services/handlebars`.
//
// All three append a Function symbol with `qualified_name = "__npm_globals__.<name>"`
// so the TS resolver's bare-name fallback finds it.
// ---------------------------------------------------------------------------

pub fn append_ember_helper_default_export(
    file_path: &str,
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    let Some((stem, signature_hint)) = handlebars_helper_stem(file_path, source) else {
        return;
    };
    let invocation_name = stem.replace('-', "_");
    let qname = format!("__npm_globals__.{invocation_name}");
    if result.symbols.iter().any(|s| s.qualified_name == qname) {
        return;
    }
    result.symbols.push(crate::types::ExtractedSymbol {
        name: invocation_name.clone(),
        qualified_name: qname,
        kind: crate::types::SymbolKind::Function,
        visibility: Some(crate::types::Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* {signature_hint} export of {stem} */")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

/// Detect a Handlebars-callable export and return (stem, signature_hint)
/// suitable for emitting the synthetic symbol. Returns None if the file
/// doesn't match any known convention.
fn handlebars_helper_stem(file_path: &str, source: &str) -> Option<(String, &'static str)> {
    let norm = file_path.replace('\\', "/");

    // 1. Ember helper: `app/helpers/<name>.{js,ts,gjs,gts}`
    if let Some(stem) = path_stem_after_segment(&norm, "/app/helpers/", &EMBER_EXTENSIONS) {
        if source.contains("@ember/component/helper")
            || source.contains("@ember/component/template-only")
        {
            return Some((stem, "Ember helper"));
        }
    }

    // 2. Ember modifier: `app/modifiers/<name>.{js,ts,gjs,gts}`
    if let Some(stem) = path_stem_after_segment(&norm, "/app/modifiers/", &EMBER_EXTENSIONS) {
        if source.contains("@ember/component/modifier")
            || source.contains("@ember/render-modifiers")
            || source.contains("ember-modifier")
        {
            return Some((stem, "Ember modifier"));
        }
    }

    // 3. Ghost-style theme helper: any `**/helpers/<name>.{js,ts}` with a
    //    handlebars-services import. Excludes the `app/helpers/` case
    //    already handled above (different content signal).
    if !norm.contains("/app/helpers/") {
        if let Some(stem) = path_stem_after_segment(&norm, "/helpers/", &THEME_EXTENSIONS) {
            if is_ghost_style_theme_helper(source) {
                return Some((stem, "Handlebars theme helper"));
            }
        }
    }

    None
}

const EMBER_EXTENSIONS: [&str; 4] = [".js", ".ts", ".gjs", ".gts"];
const THEME_EXTENSIONS: [&str; 2] = [".js", ".ts"];

/// Find the file stem that sits directly inside `segment` (no nested subdir).
/// Nested helper paths (`segment/sub/name.js`) return None — they require
/// dotted-name resolution which the bare-name fallback doesn't cover.
fn path_stem_after_segment(norm: &str, segment: &str, exts: &[&str]) -> Option<String> {
    let idx = norm.rfind(segment)?;
    let after = &norm[idx + segment.len()..];
    let after = after.trim_start_matches('/');
    if after.contains('/') {
        return None;
    }
    for ext in exts {
        if let Some(stem) = after.strip_suffix(ext) {
            if !stem.is_empty() {
                return Some(stem.to_string());
            }
        }
    }
    None
}

fn is_ghost_style_theme_helper(source: &str) -> bool {
    // Ghost theme helpers `require('../services/handlebars')` for SafeString
    // and friends; other Handlebars-host frameworks (Express-Handlebars,
    // hbs-engine) use `Handlebars.registerHelper`. Either signal qualifies.
    source.contains("services/handlebars")
        || source.contains("Handlebars.registerHelper")
        || source.contains("handlebars').SafeString")
        || source.contains("handlebars\").SafeString")
}

// ---------------------------------------------------------------------------
// AMD `define([deps], function(params) { ... })` — RequireJS modules
//
// The classic AMD pattern (RequireJS, used by SWISH, AngularJS-1
// projects, jQuery plugin authors, etc.) is conceptually identical to
// ES module imports but wraps everything in a single `define()` call:
//
//   define([ "jquery", "./config", "preferences" ],
//          function($, config, preferences) {
//              // body uses $, config, preferences as locals
//          });
//
// Each dep string in the array maps positionally to a function param.
// Without recognising this shape, every reference to `$.each`,
// `config.foo`, or `preferences.bar` inside the callback lands in
// unresolved_refs because the params aren't declared anywhere the
// resolver thinks of as an import.
//
// `append_amd_define_imports` scans the source for the literal
// `define([...] , function(...) { ... })` shape and emits one
// `EdgeKind::Imports` ref per (dep, param) pair, with
// `target_name = param`, `module = Some(dep)`. The shared
// `resolve_common` path then handles them like any other named
// import.
// ---------------------------------------------------------------------------

pub fn append_amd_define_imports(
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    let pairs = scan_amd_define_pairs(source);
    if pairs.is_empty() {
        return;
    }
    for (dep, param, line) in pairs {
        // Skip dummy AMD names (`require`, `exports`, `module`) — these
        // are AMD bookkeeping, not real deps.
        if matches!(dep.as_str(), "require" | "exports" | "module") {
            continue;
        }
        result.refs.push(crate::types::ExtractedRef {
            source_symbol_index: 0,
            target_name: param,
            kind: crate::types::EdgeKind::Imports,
            line,
            module: Some(dep),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
        });
    }
}

/// Returns a `(dep, param, line)` triple for every dep position in every
/// `define([...], function(...) {...})` block in `source`. Tolerant to
/// whitespace, line breaks, single/double-quoted dep strings, and
/// `define("name", [...], function(...){...})` (named modules — the
/// leading string is ignored).
pub(crate) fn scan_amd_define_pairs(source: &str) -> Vec<(String, String, u32)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(rel) = source[i..].find("define(") else { break };
        let start = i + rel;
        // Identifier-boundary check.
        if start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                i = start + 7;
                continue;
            }
        }
        let after_open = start + 7; // past `define(`
        let mut j = skip_ws(bytes, after_open);
        // Optional leading string (named module).
        if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
            let q = bytes[j];
            j += 1;
            while j < bytes.len() && bytes[j] != q {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                } else {
                    j += 1;
                }
            }
            if j < bytes.len() {
                j += 1;
            }
            j = skip_ws(bytes, j);
            if j < bytes.len() && bytes[j] == b',' {
                j += 1;
                j = skip_ws(bytes, j);
            }
        }
        if j >= bytes.len() || bytes[j] != b'[' {
            i = after_open;
            continue;
        }
        // Parse the dep array.
        j += 1;
        let mut deps: Vec<(String, u32)> = Vec::new();
        loop {
            j = skip_ws(bytes, j);
            if j >= bytes.len() {
                break;
            }
            if bytes[j] == b']' {
                j += 1;
                break;
            }
            if bytes[j] == b',' {
                j += 1;
                continue;
            }
            if bytes[j] == b'\'' || bytes[j] == b'"' {
                let q = bytes[j];
                let dep_start = j + 1;
                let mut k = dep_start;
                while k < bytes.len() && bytes[k] != q {
                    if bytes[k] == b'\\' && k + 1 < bytes.len() {
                        k += 2;
                    } else {
                        k += 1;
                    }
                }
                if k > bytes.len() {
                    break;
                }
                let dep = source[dep_start..k].to_string();
                let line = line_at(bytes, dep_start);
                deps.push((dep, line));
                j = k + 1;
            } else {
                // Unrecognised token (variable ref, etc.) — bail.
                return out;
            }
        }
        j = skip_ws(bytes, j);
        if j >= bytes.len() || bytes[j] != b',' {
            i = after_open;
            continue;
        }
        j += 1;
        j = skip_ws(bytes, j);
        // Expect `function(` (allow `function name(` and arrow-style
        // `(a, b) =>` too).
        let params = parse_callback_params(bytes, &mut j);
        let Some(params) = params else {
            i = after_open;
            continue;
        };
        for (idx, (dep, line)) in deps.iter().enumerate() {
            let Some(param) = params.get(idx) else { break };
            if param.is_empty() {
                continue;
            }
            out.push((dep.clone(), param.clone(), *line));
        }
        i = j;
    }
    out
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            // Line comment.
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // Block comment.
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            _ => return i,
        }
    }
    i
}

/// Parse a `function(p1, p2)` or `(p1, p2) =>` parameter list starting
/// at `*pos`. On success, advances `*pos` past the `(` ... `)` and
/// returns the param names (stripped). Returns None for shapes the
/// scanner doesn't recognise.
fn parse_callback_params(bytes: &[u8], pos: &mut usize) -> Option<Vec<String>> {
    let mut j = *pos;
    j = skip_ws(bytes, j);
    // `function`-form: `function( ... )`, `function name( ... )`.
    if j + 8 <= bytes.len() && &bytes[j..j + 8] == b"function" {
        let after = j + 8;
        let next = bytes.get(after).copied().unwrap_or(0);
        if !next.is_ascii_alphanumeric() && next != b'_' {
            j = after;
            j = skip_ws(bytes, j);
            // Optional named function: `function name(`.
            if j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
                {
                    j += 1;
                }
                j = skip_ws(bytes, j);
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let params = read_paren_params(bytes, j)?;
                let new_j = find_matching_paren(bytes, j)?;
                *pos = new_j + 1;
                return Some(params);
            }
        }
    }
    // Arrow-style: `(p1, p2) =>` or single `p =>`.
    if j < bytes.len() && bytes[j] == b'(' {
        let params = read_paren_params(bytes, j)?;
        let new_j = find_matching_paren(bytes, j)?;
        *pos = new_j + 1;
        return Some(params);
    }
    None
}

fn read_paren_params(bytes: &[u8], open: usize) -> Option<Vec<String>> {
    let close = find_matching_paren(bytes, open)?;
    let inner = std::str::from_utf8(&bytes[open + 1..close]).ok()?;
    Some(
        inner
            .split(',')
            .map(|s| {
                let t = s.trim();
                // Strip default-arg / type-annotations: keep the head
                // identifier only.
                let mut k = 0;
                let bytes = t.as_bytes();
                while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_' || bytes[k] == b'$') {
                    k += 1;
                }
                String::from_utf8_lossy(&bytes[..k]).to_string()
            })
            .collect(),
    )
}

fn find_matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn line_at(bytes: &[u8], pos: usize) -> u32 {
    let mut line = 0u32;
    for &b in &bytes[..pos.min(bytes.len())] {
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

// ---------------------------------------------------------------------------
// jQuery plugin registration: `$.fn.NAME = function(...) { ... }`
//
// Older AMD / RequireJS-era JS projects (SWISH, AngularJS-1 themes, many
// jQuery-plugin authors) register methods on `$.fn` — these become
// chainable methods on every jQuery selector (`$(elem).NAME(...)`). The
// JS extractor sees the call as a Calls ref to `NAME` (the chain root
// `$(elem)` is opaque) but `NAME` doesn't exist as a top-level symbol
// anywhere — it lives as a property on the jQuery prototype.
//
// Discovery: scan source for the literal `$.fn.NAME = function`,
// `jQuery.fn.NAME = function`, or `$.fn['NAME'] = function` patterns.
// Each match emits a synthetic Function symbol with
// `qualified_name = "__npm_globals__.NAME"` so the TS resolver's bare-
// name fallback finds it. Mirrors the Handlebars/Ember helper pattern.
//
// Only scans project source; jQuery's CORE methods (`each`, `hasClass`,
// `addClass`, ...) need jQuery's own source on disk to be indexed.
// ---------------------------------------------------------------------------

pub fn append_jquery_fn_plugin_globals(
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    for name in scan_jquery_fn_plugin_names(source) {
        let qname = format!("__npm_globals__.{name}");
        if result.symbols.iter().any(|s| s.qualified_name == qname) {
            continue;
        }
        result.symbols.push(crate::types::ExtractedSymbol {
            name: name.clone(),
            qualified_name: qname,
            kind: crate::types::SymbolKind::Function,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("/* $.fn.{name} jQuery plugin */")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

pub(crate) fn scan_jquery_fn_plugin_names(source: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Match `$.fn` or `jQuery.fn` followed by either `.NAME` or `['NAME']`.
    let needles = ["$.fn", "jQuery.fn"];
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let mut matched_len: Option<usize> = None;
        for needle in needles {
            let nb = needle.as_bytes();
            if i + nb.len() <= bytes.len() && &bytes[i..i + nb.len()] == nb {
                // Suffix must be `.` or `[` to be the property-access form
                // we care about. Otherwise this is `$.fn` standalone or
                // `jQuery.fn` followed by something else.
                let suffix = bytes.get(i + nb.len()).copied().unwrap_or(0);
                if suffix == b'.' || suffix == b'[' {
                    matched_len = Some(nb.len() + 1); // include the `.` or `[`
                    break;
                }
            }
        }
        if let Some(needle_len) = matched_len {
            // Identifier-boundary check: prev char must NOT be alphanum/_.
            if i > 0 {
                let prev = bytes[i - 1];
                if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                    i += needle_len;
                    continue;
                }
            }
            // bytes[i + needle_len - 1] is either `.` or `[`.
            let suffix = bytes[i + needle_len - 1];
            let after = i + needle_len;
            let (name, mut k) = if suffix == b'[' {
                // `$.fn[ 'name' ]` — skip whitespace, expect quote.
                let mut j = after;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j >= bytes.len() || (bytes[j] != b'\'' && bytes[j] != b'"') {
                    i = after;
                    continue;
                }
                let q = bytes[j];
                let start = j + 1;
                let mut e = start;
                while e < bytes.len() && bytes[e] != q {
                    if bytes[e] == b'\\' && e + 1 < bytes.len() {
                        e += 2;
                    } else {
                        e += 1;
                    }
                }
                let n = std::str::from_utf8(&bytes[start..e]).ok();
                (n.map(str::to_string), e + 1)
            } else {
                // `$.fn.NAME` — bare identifier.
                let start = after;
                let mut e = start;
                while e < bytes.len()
                    && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_' || bytes[e] == b'$')
                {
                    e += 1;
                }
                let n = std::str::from_utf8(&bytes[start..e]).ok();
                (n.map(str::to_string), e)
            };
            // Now expect `=` (allow whitespace and trailing `]` from bracket form).
            while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t' || bytes[k] == b']') {
                k += 1;
            }
            if k >= bytes.len() || bytes[k] != b'=' {
                i = after;
                continue;
            }
            // Confirm RHS looks like a function (function/arrow/method).
            let mut m = k + 1;
            while m < bytes.len() && (bytes[m] == b' ' || bytes[m] == b'\t' || bytes[m] == b'\n') {
                m += 1;
            }
            let rhs_is_function = m + 8 <= bytes.len() && &bytes[m..m + 8] == b"function"
                || m < bytes.len() && bytes[m] == b'('
                || m + 5 <= bytes.len() && &bytes[m..m + 5] == b"async";
            if !rhs_is_function {
                i = after;
                continue;
            }
            if let Some(n) = name {
                let trimmed = n.trim();
                if !trimmed.is_empty()
                    && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    && !out.iter().any(|x| x == trimmed)
                {
                    out.push(trimmed.to_string());
                }
            }
            i = m;
            continue;
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Handlebars.RegisterHelper("name", ...) — runtime registration scan
//
// Some hosts (Bitwarden's C# Handlebars.Net, JS server-side templating)
// register helpers imperatively rather than by file convention. The helper
// name is a string literal in the registration call, and the consuming
// templates invoke it bare. Without scanning these registrations the
// invocations land in unresolved_refs.
//
// The scan is regex-free deliberately — a small state machine that
// recognizes the literal `Handlebars.RegisterHelper(` or
// `Handlebars.registerHelper(` token, then captures the next quoted string
// argument. Works for C#, JS, TS, and any language that calls the same
// API. Each captured name is appended as a Function symbol with
// `qualified_name = "__npm_globals__.<name>"` so the TS resolver's
// bare-name fallback finds it from a Handlebars-embedded template call.
// ---------------------------------------------------------------------------

pub fn append_handlebars_register_helper_globals(
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    for name in scan_register_helper_names(source) {
        let invocation_name = name.replace('-', "_");
        let qname = format!("__npm_globals__.{invocation_name}");
        if result.symbols.iter().any(|s| s.qualified_name == qname) {
            continue;
        }
        result.symbols.push(crate::types::ExtractedSymbol {
            name: invocation_name.clone(),
            qualified_name: qname,
            kind: crate::types::SymbolKind::Function,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("/* Handlebars.RegisterHelper(\"{name}\", ...) */")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

fn scan_register_helper_names(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needles = [
        "Handlebars.RegisterHelper(",
        "Handlebars.registerHelper(",
        "handlebars.RegisterHelper(",
        "handlebars.registerHelper(",
    ];
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let mut matched_len: Option<usize> = None;
        for needle in needles {
            let nb = needle.as_bytes();
            if i + nb.len() <= bytes.len() && &bytes[i..i + nb.len()] == nb {
                matched_len = Some(nb.len());
                break;
            }
        }
        if let Some(after_open) = matched_len {
            let mut j = i + after_open;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r') {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                let quote = bytes[j];
                let start = j + 1;
                let mut k = start;
                while k < bytes.len() && bytes[k] != quote {
                    if bytes[k] == b'\\' && k + 1 < bytes.len() {
                        k += 2;
                    } else {
                        k += 1;
                    }
                }
                if k <= bytes.len() && k > start {
                    if let Ok(name) = std::str::from_utf8(&bytes[start..k]) {
                        let trimmed = name.trim();
                        if !trimmed.is_empty()
                            && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                        {
                            out.push(trimmed.to_string());
                        }
                    }
                }
            }
            i += after_open;
        } else {
            i += 1;
        }
    }
    out
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

    // -----------------------------------------------------------------------
    // Ember helper-export detection
    // -----------------------------------------------------------------------

    fn empty_result() -> crate::types::ExtractionResult {
        crate::types::ExtractionResult {
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: false,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }

    #[test]
    fn ember_helper_appends_npm_globals_symbol() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "ghost/admin/app/helpers/gh-pluralize.js",
            src,
            &mut r,
        );
        let sym = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "__npm_globals__.gh_pluralize")
            .expect("expected synthetic helper symbol");
        assert_eq!(sym.name, "gh_pluralize");
        assert_eq!(sym.kind, crate::types::SymbolKind::Function);
    }

    #[test]
    fn ember_helper_skipped_when_no_ember_import() {
        let mut r = empty_result();
        let src = "// just a regular module\nexport function thing() { return 1; }";
        append_ember_helper_default_export(
            "myproject/app/helpers/random.js",
            src,
            &mut r,
        );
        assert!(
            r.symbols.is_empty(),
            "non-Ember files in helpers/ should not get the synthetic; got: {:?}",
            r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ember_helper_skipped_outside_app_helpers_dir() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "ghost/admin/app/lib/random.js",
            src,
            &mut r,
        );
        assert!(r.symbols.is_empty());
    }

    #[test]
    fn ember_helper_skipped_for_nested_helper_paths() {
        // `app/helpers/blog/post-card.js` — invocation would be `blog/post-card`
        // (rewritten to `blog.post_card` by the Handlebars wrapper). That's a
        // dotted lookup, not a bare-name fallback target — out of scope here.
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "ghost/admin/app/helpers/blog/post-card.js",
            src,
            &mut r,
        );
        assert!(r.symbols.is_empty());
    }

    #[test]
    fn ember_helper_handles_typescript_extension() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "myapp/app/helpers/format-date.ts",
            src,
            &mut r,
        );
        assert!(r.symbols.iter().any(|s| s.name == "format_date"));
    }

    #[test]
    fn ember_helper_idempotent_on_repeat_calls() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "myapp/app/helpers/eq.js",
            src,
            &mut r,
        );
        append_ember_helper_default_export(
            "myapp/app/helpers/eq.js",
            src,
            &mut r,
        );
        assert_eq!(
            r.symbols.iter().filter(|s| s.qualified_name == "__npm_globals__.eq").count(),
            1,
            "duplicate detection should keep the symbol unique"
        );
    }

    #[test]
    fn ember_modifier_appends_npm_globals_symbol() {
        let mut r = empty_result();
        let src = "import Modifier from '@ember/component/modifier';\nexport default class extends Modifier { modify() {} }";
        append_ember_helper_default_export(
            "ghost/admin/app/modifiers/react-render.js",
            src,
            &mut r,
        );
        let sym = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "__npm_globals__.react_render")
            .expect("expected synthetic modifier symbol");
        assert_eq!(sym.name, "react_render");
        assert!(sym.signature.as_ref().unwrap().contains("Ember modifier"));
    }

    #[test]
    fn ember_modifier_via_render_modifiers_import() {
        let mut r = empty_result();
        let src = "import { modifier } from 'ember-modifier';\nexport default modifier((el) => {});";
        append_ember_helper_default_export(
            "myapp/app/modifiers/on-key.js",
            src,
            &mut r,
        );
        assert!(r.symbols.iter().any(|s| s.name == "on_key"));
    }

    #[test]
    fn ghost_theme_helper_appends_npm_globals_symbol() {
        let mut r = empty_result();
        let src = "const {SafeString} = require('../services/handlebars');\nmodule.exports = function tiers(options) { return new SafeString(''); };";
        append_ember_helper_default_export(
            "ghost/core/core/frontend/helpers/tiers.js",
            src,
            &mut r,
        );
        let sym = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "__npm_globals__.tiers")
            .expect("expected synthetic theme-helper symbol");
        assert_eq!(sym.name, "tiers");
        assert!(sym.signature.as_ref().unwrap().contains("theme helper"));
    }

    #[test]
    fn ghost_theme_helper_via_register_helper_pattern() {
        let mut r = empty_result();
        let src = "const Handlebars = require('handlebars');\nHandlebars.registerHelper('formatDate', function(d) { return d; });\nmodule.exports = formatDate;";
        append_ember_helper_default_export(
            "myapp/lib/helpers/format-date.js",
            src,
            &mut r,
        );
        assert!(
            r.symbols.iter().any(|s| s.name == "format_date"),
            "Handlebars.registerHelper pattern should activate detection"
        );
    }

    #[test]
    fn random_helpers_dir_without_handlebars_signal_skipped() {
        let mut r = empty_result();
        // A folder named `helpers/` but with no Handlebars signal — could be
        // a generic JS utility module. Don't claim it as a template helper.
        let src = "export function helper() {}\nexport default helper;";
        append_ember_helper_default_export(
            "src/helpers/utility.js",
            src,
            &mut r,
        );
        assert!(r.symbols.is_empty(), "no Handlebars signal → no synthetic");
    }

    // -----------------------------------------------------------------------
    // Handlebars.RegisterHelper("name", ...) scan
    // -----------------------------------------------------------------------

    #[test]
    fn register_helper_csharp_double_quoted_captures_name() {
        let mut r = empty_result();
        let src = "Handlebars.RegisterHelper(\"usd\", (writer, ctx, args) => writer.Write(args[0]));";
        append_handlebars_register_helper_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s|
            s.name == "usd" && s.qualified_name == "__npm_globals__.usd"
        ));
    }

    #[test]
    fn register_helper_js_lowercase_captures_name() {
        let mut r = empty_result();
        let src = "Handlebars.registerHelper('format-date', function(d) { return d; });";
        append_handlebars_register_helper_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s|
            s.name == "format_date" && s.qualified_name == "__npm_globals__.format_date"
        ));
    }

    #[test]
    fn register_helper_multiple_in_one_file() {
        let mut r = empty_result();
        let src = r#"
            Handlebars.RegisterHelper("date", X);
            Handlebars.RegisterHelper("usd", Y);
            Handlebars.RegisterHelper("plurality", Z);
        "#;
        append_handlebars_register_helper_globals(src, &mut r);
        for n in ["date", "usd", "plurality"] {
            assert!(r.symbols.iter().any(|s| s.name == n), "missing {n}");
        }
    }

    #[test]
    fn register_helper_idempotent_on_duplicate_registration() {
        let mut r = empty_result();
        let src = "Handlebars.RegisterHelper(\"eq\", X);\nHandlebars.RegisterHelper(\"eq\", Y);";
        append_handlebars_register_helper_globals(src, &mut r);
        assert_eq!(
            r.symbols.iter().filter(|s| s.name == "eq").count(),
            1,
            "duplicate registrations of the same name should yield one symbol"
        );
    }

    #[test]
    fn register_helper_skips_non_string_first_arg() {
        let mut r = empty_result();
        // Variable as helper name — can't statically capture it.
        let src = "Handlebars.RegisterHelper(myHelperName, fn);";
        append_handlebars_register_helper_globals(src, &mut r);
        assert!(r.symbols.is_empty());
    }

    // -----------------------------------------------------------------------
    // AMD `define([deps], function(params) {...})` scan
    // -----------------------------------------------------------------------

    #[test]
    fn amd_define_emits_imports_per_dep() {
        let src = "define([ \"jquery\", \"./config\", \"preferences\" ],\n        function($, config, preferences) { return $.fn; });";
        let pairs = scan_amd_define_pairs(src);
        let by_dep: std::collections::HashMap<&str, &str> =
            pairs.iter().map(|(d, p, _)| (d.as_str(), p.as_str())).collect();
        assert_eq!(by_dep.get("jquery"), Some(&"$"));
        assert_eq!(by_dep.get("./config"), Some(&"config"));
        assert_eq!(by_dep.get("preferences"), Some(&"preferences"));
    }

    #[test]
    fn amd_define_handles_named_module_form() {
        // `define("modname", [...], function(...) {...})` — leading
        // string is the module name, ignored.
        let src = "define(\"my/mod\", [\"jquery\"], function($) {});";
        let pairs = scan_amd_define_pairs(src);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "jquery");
        assert_eq!(pairs[0].1, "$");
    }

    #[test]
    fn amd_define_arrow_callback_works() {
        let src = "define([\"jquery\"], ($) => $.fn);";
        let pairs = scan_amd_define_pairs(src);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].1, "$");
    }

    #[test]
    fn amd_define_multiline_dep_array() {
        let src = "define([\n  \"a\",\n  \"b\",\n  \"c\"\n], function(a, b, c) {});";
        let pairs = scan_amd_define_pairs(src);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].0, "a");
        assert_eq!(pairs[2].1, "c");
    }

    #[test]
    fn append_amd_define_imports_skips_amd_bookkeeping() {
        let mut r = empty_result();
        // `require`, `exports`, `module` are AMD bookkeeping pseudo-deps.
        let src = "define([\"require\", \"exports\", \"./real\"], function(req, exp, real) {});";
        append_amd_define_imports(src, &mut r);
        let names: Vec<&str> = r
            .refs
            .iter()
            .filter(|x| matches!(x.kind, crate::types::EdgeKind::Imports))
            .map(|x| x.target_name.as_str())
            .collect();
        assert_eq!(names, vec!["real"]);
    }

    #[test]
    fn amd_define_xhelper_define_no_match() {
        // `xdefine([...])` must not match `define([...])`.
        let src = "xdefine([\"jquery\"], function($){});";
        let pairs = scan_amd_define_pairs(src);
        assert!(pairs.is_empty());
    }

    // -----------------------------------------------------------------------
    // `$.fn.NAME = function(...)` jQuery plugin registration scan
    // -----------------------------------------------------------------------

    #[test]
    fn jquery_fn_plugin_emits_npm_globals() {
        let mut r = empty_result();
        let src = "$.fn.prologEditor = function(method) { return this; };";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s|
            s.name == "prologEditor" && s.qualified_name == "__npm_globals__.prologEditor"
        ));
    }

    #[test]
    fn jquery_fn_plugin_handles_full_jquery_prefix() {
        let mut r = empty_result();
        let src = "jQuery.fn.tooltip = function(opts) {};";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s| s.name == "tooltip"));
    }

    #[test]
    fn jquery_fn_plugin_handles_bracketed_form() {
        let mut r = empty_result();
        let src = "$.fn['nbCell'] = function(method) {};";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s| s.name == "nbCell"));
    }

    #[test]
    fn jquery_fn_plugin_skips_non_function_rhs() {
        let mut r = empty_result();
        // Plain value assignment, not a callable plugin.
        let src = "$.fn.version = '1.0';";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.is_empty());
    }

    #[test]
    fn jquery_fn_plugin_dedupes_same_name() {
        let mut r = empty_result();
        let src = "$.fn.foo = function() {};\n$.fn.foo = function() {};";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert_eq!(r.symbols.iter().filter(|s| s.name == "foo").count(), 1);
    }
}
