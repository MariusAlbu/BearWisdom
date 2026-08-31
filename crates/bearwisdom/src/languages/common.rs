// =============================================================================
// languages/common.rs  —  shared extraction utilities used by multiple plugins
//
// Functions here are language-agnostic helpers that would otherwise be
// duplicated across per-language call extractors.  They live here rather than
// in `languages/mod.rs` to keep the plugin registry and trait definitions
// uncluttered.
// =============================================================================

use crate::type_checker::core::types::TypeArena;
use crate::types::{
    ChainSegment, EmbeddedOrigin, EmbeddedRegion, ExtractionResult, MemberChain, SegmentKind,
    SymbolKind,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Shared TypeId population — used by `extract_with_arena_and_demand` overrides
// ---------------------------------------------------------------------------

/// Populate `ExtractedSymbol` TypeId fields (return_type, param_types)
/// against the workspace `TypeArena` for the canonical "type-defining +
/// callable" pattern shared by every typed language plugin. Plugins
/// override `LanguagePlugin::extract_with_arena_and_demand`, call their
/// existing extract logic, then invoke this helper with their `lang_id`.
///
///   - Type-defining kinds (Class / Interface / Struct / Trait / Enum /
///     TypeAlias) get `return_type = arena.class(qualified_name)` —
///     the canonical "callable type yields itself" rule.
///   - Callable kinds (Method / Function / Constructor) get the return
///     type and parameter types parsed from their signature via
///     `parse_return_type_from_signature` and the language-aware
///     `parse_param_types_from_signature_for_lang`, then interned
///     through `arena.intern_type_str` so generic applications decompose
///     into structural `Apply { base, args }`.
///
/// Idempotent: skips symbols whose `return_type` / `param_types` are
/// already populated.
pub fn populate_return_type_ids(result: &mut ExtractionResult, arena: &TypeArena, lang_id: &str) {
    for sym in &mut result.symbols {
        match sym.kind {
            SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Struct
            | SymbolKind::Trait
            | SymbolKind::Enum
            | SymbolKind::TypeAlias => {
                if sym.return_type.is_none() {
                    sym.return_type = Some(arena.class(&sym.qualified_name));
                }
            }
            SymbolKind::Method | SymbolKind::Function | SymbolKind::Constructor => {
                let Some(sig) = sym.signature.as_deref() else {
                    continue;
                };
                if sym.return_type.is_none() {
                    if let Some(rt) =
                        crate::indexer::resolve::engine::contract::chain_walker::parse_return_type_from_signature_for_lang(
                            sig,
                            lang_id,
                        )
                    {
                        // A bare (unqualified) return-type name is only safe to
                        // intern here when the symbol has no enclosing scope: a
                        // method nested under a namespace/module whose signature
                        // just says `Foo` may mean ITS OWN scope's `Foo`, not a
                        // same-named top-level declaration — and this pass runs
                        // per-file, before the cross-file symbol table exists, so
                        // it cannot scope-qualify the name itself. Leaving the
                        // slot empty defers to the later scope-aware derivation
                        // (`resolve_type_name_in_scope`, run once the full symbol
                        // table is built), which reads the same signature text
                        // plus this symbol's `scope_path`.
                        let is_bare = !rt.contains('.') && !rt.contains("::");
                        let has_scope = sym.scope_path.as_deref().is_some_and(|s| !s.is_empty());
                        if !rt.is_empty() && !(is_bare && has_scope) {
                            sym.return_type = Some(arena.intern_type_str(&rt));
                        }
                    }
                }
                if sym.param_types.is_empty() {
                    if let Some(params) =
                        crate::indexer::resolve::engine::contract::chain_walker::parse_param_types_from_signature_for_lang(
                            sig,
                            lang_id,
                        )
                    {
                        sym.param_types = params
                            .iter()
                            .map(|p| arena.intern_type_str(p))
                            .collect();
                    }
                }
            }
            SymbolKind::Field
            | SymbolKind::Property
            | SymbolKind::Variable
            | SymbolKind::Parameter => {
                if sym.declared_type.is_some() {
                    continue;
                }
                let Some(sig) = sym.signature.as_deref() else {
                    continue;
                };
                if let Some(ty) =
                    crate::indexer::resolve::engine::contract::chain_walker::parse_declared_type_from_signature_for_lang(
                        sig,
                        lang_id,
                    )
                {
                    if !ty.is_empty() {
                        sym.declared_type = Some(arena.intern_type_str(&ty));
                    }
                }
            }
            _ => {}
        }
    }
}

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
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
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
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
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
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
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
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.b().c()` — walk into the function child,
            // then mark the resolved segment as invoked so the walker yields the
            // function's return type rather than the function value itself.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
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
        "identifier" | "shorthand_property_identifier_pattern" => {
            node_text_bytes(node, src) == name
        }
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
    // A CALLED segment is a method, never a type — `A.CallTo(x).Invokes(y)`
    // has an uppercase called middle segment that the uppercase heuristic
    // would otherwise misread as the `Ns.Type.method()` shape.
    if type_seg.is_call {
        return;
    }
    if type_seg
        .name
        .chars()
        .next()
        .map_or(false, |ch| ch.is_uppercase())
    {
        refs.push(crate::types::ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
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

mod amd;
mod call_args;
mod handlebars;
mod html;
mod jquery;

pub use amd::append_amd_define_imports;
pub use call_args::{extract_call_args, replace_template_substitutions};
pub use handlebars::{
    append_ember_helper_default_export, append_handlebars_register_helper_globals,
};
pub use html::{
    extract_astro_frontmatter, extract_html_script_style_regions, extract_script_refs, ScriptRef,
};
pub use jquery::append_jquery_fn_plugin_globals;

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
