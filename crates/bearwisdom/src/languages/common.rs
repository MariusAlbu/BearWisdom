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
                byte_offset: 0,
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
                byte_offset: 0,
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
                byte_offset: 0,
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
                byte_offset: 0,
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
            col: 0,
            module: None,
            chain: None,
            byte_offset: func_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}


mod html;
mod handlebars;
mod amd;
mod jquery;

pub use html::{extract_script_refs, extract_html_script_style_regions, extract_astro_frontmatter, ScriptRef};
pub use handlebars::{append_ember_helper_default_export, append_handlebars_register_helper_globals};
pub use amd::append_amd_define_imports;
pub use jquery::append_jquery_fn_plugin_globals;
