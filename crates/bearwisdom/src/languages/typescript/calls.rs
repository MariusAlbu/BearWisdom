use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Emit a Calls ref for a single `call_expression` node.
///
/// This is used from `extract_node` to capture calls at any AST level that the
/// recursive visitor traverses (top-level statements, field initializers, etc.)
/// without re-walking the entire subtree (the caller handles recursion).
pub(super) fn emit_call_ref(
    call_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(func_node) = call_node.child_by_field_name("function") {
        let chain = build_chain(func_node, src);
        let target_name = chain
            .as_ref()
            .and_then(|c| c.segments.last())
            .map(|s| s.name.clone())
            .unwrap_or_else(|| callee_name_fallback(func_node, src));

        crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
        if !target_name.is_empty() && target_name != "undefined" {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name,
                kind: EdgeKind::Calls,
                line: func_node.start_position().row as u32,
                module: None,
                chain,
                byte_offset: func_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
});
        }
    }
}

/// Emit an Instantiates ref for a single `new_expression` node.
pub(super) fn emit_new_ref(
    new_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(constructor) = new_node.child_by_field_name("constructor") {
        let name = match constructor.kind() {
            "identifier" | "type_identifier" => node_text(constructor, src),
            "member_expression" => callee_name_fallback(constructor, src),
            _ => return,
        };
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Instantiates,
                line: constructor.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
    }
}

pub(super) fn extract_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(func_node) = child.child_by_field_name("function") {
                    let chain = build_chain(func_node, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name_fallback(func_node, src));

                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
                    if !target_name.is_empty() && target_name != "undefined" {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: func_node.start_position().row as u32,
                            module: None,
                            chain,
                            byte_offset: func_node.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }
            "new_expression" => {
                emit_new_ref(&child, src, source_symbol_index, refs);
                extract_calls(&child, src, source_symbol_index, refs);
            }
            // `sql\`SELECT ...\`` / `gql\`query { ... }\`` — tagged template expression.
            // The first child (the tag) is the function being called.
            "tagged_template_expression" => {
                // tree-sitter field: "tag" is the function, "template" is the literal.
                let tag_node = child.child_by_field_name("tag");
                if let Some(tag) = tag_node {
                    let chain = build_chain(tag, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name_fallback(tag, src));
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &tag, refs);
                    if !target_name.is_empty() && target_name != "undefined" {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: tag.start_position().row as u32,
                            module: None,
                            chain,
                            byte_offset: tag.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
                // Recurse for any nested calls inside the template.
                extract_calls(&child, src, source_symbol_index, refs);
            }
            // JSX: `<Component />` or `<Component>...</Component>` is a call
            // to the component function/class.  Emit a Calls edge for user-
            // defined components (PascalCase) — lowercase tags are HTML intrinsics.
            "jsx_self_closing_element" | "jsx_opening_element" => {
                emit_jsx_component_ref(&child, src, source_symbol_index, refs);
                extract_calls(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a `Calls` ref + receiver `TypeRef` for a JSX component tag.
///
/// Handles both the bare form (`<Component …/>`) and member-expression
/// form (`<Foo.Bar …/>`) — the latter produces a structured MemberChain
/// so the resolver's chain walker can follow the receiver's inferred
/// type (e.g. `PollContext` → `React.Context<T>`) to the tail member
/// (`Provider` / `Consumer`).
///
/// Skips lowercase tags (HTML intrinsics — not graph-resolvable symbols).
pub(super) fn emit_jsx_component_ref(
    element: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let Some(tag_node) = element
        .child_by_field_name("name")
        .or_else(|| element.named_child(0))
    else {
        return;
    };
    let tag_name = node_text(tag_node, src);
    if tag_name.is_empty()
        || !tag_name.chars().next().map_or(false, |c| c.is_uppercase())
    {
        return;
    }
    let chain = build_chain(tag_node, src);
    let target = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or(tag_name);
    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &tag_node, refs);
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::Calls,
        line: tag_node.start_position().row as u32,
        module: None,
        chain,
        byte_offset: tag_node.start_byte() as u32,
            namespace_segments: Vec::new(),
});
}

/// Build a structured member access chain from tree-sitter AST nodes.
///
/// Recursively walks nested `member_expression` nodes (left-recursive) to
/// produce a `Vec<ChainSegment>` from inside-out.
///
/// `this.repo.findOne()` tree structure:
/// ```text
/// member_expression @function
///   member_expression @object
///     this @object
///     property_identifier "repo"
///   property_identifier "findOne"
/// ```
/// produces: `[this, repo, findOne]`
pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

pub(super) fn build_chain_inner(
    node: Node,
    src: &[u8],
    segments: &mut Vec<ChainSegment>,
) -> Option<()> {
    match node.kind() {
        "this" | "super" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        // `import("module").then(...)` — dynamic import. Tree-sitter
        // typescript emits the `import` keyword as the function of the
        // call_expression. Without a chain root the walker bails and
        // `then` / `catch` / `finally` end up unresolved. Inject a
        // synthetic root segment whose `declared_type` is `Promise` so
        // Phase 1 picks Promise as the root and Phase 3 resolves the
        // method against `Promise.then` etc.
        "import" => {
            segments.push(ChainSegment {
                name: "import".to_string(),
                node_kind: "import".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some("Promise".to_string()),
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        // `[1,2,3].map(...)` / `[].push(...)` — array literal as chain
        // root. Without a synthetic root the walker bails on
        // `Array.map` / `.filter` / `.forEach` etc.
        "array" => {
            segments.push(ChainSegment {
                name: "Array".to_string(),
                node_kind: "array".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some("Array".to_string()),
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        // `{a: 1}.hasOwnProperty(...)` / `{...}.method()` — object
        // literal as chain root.
        "object" => {
            segments.push(ChainSegment {
                name: "Object".to_string(),
                node_kind: "object".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some("Object".to_string()),
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        // String literal `"foo".charAt(0)`, `'bar'.split(',')` — chain
        // root resolves to the String type (lib.es5.d.ts).
        "string" | "template_string" => {
            segments.push(ChainSegment {
                name: "String".to_string(),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some("String".to_string()),
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        // Numeric literal `(42).toString()` — chain root resolves to
        // the Number type. Rare but cheap to handle alongside the
        // others.
        "number" => {
            segments.push(ChainSegment {
                name: "Number".to_string(),
                node_kind: "number".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some("Number".to_string()),
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        // Regex literal `/foo/.test(s)` — RegExp type.
        "regex" => {
            segments.push(ChainSegment {
                name: "RegExp".to_string(),
                node_kind: "regex".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: Some("RegExp".to_string()),
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;

            // Check for optional chaining: `?.` between object and property.
            let is_optional = (0..node.child_count()).any(|i| {
                node.child(i)
                    .map(|c| c.kind() == "optional_chain")
                    .unwrap_or(false)
            });

            // Recurse into the object to build the prefix chain.
            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text(property, src),
                node_kind: property.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: is_optional,
            });
            Some(())
        }

        "subscript_expression" => {
            // `this.handlers['click']`
            let object = node.child_by_field_name("object")?;
            let index = node.child_by_field_name("index")?;

            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text(index, src),
                node_kind: "subscript_expression".to_string(),
                kind: SegmentKind::ComputedAccess,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.b().c()` — the object is a call_expression.
            // Walk into its function child to continue the chain.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // `await foo()`, `await x.method()` — peel the `await` so the chain
        // walker sees the underlying call. Without this, `target_name`
        // captures the entire `await jsonlStreamConsumer` text and
        // resolution always misses.
        "await_expression" => {
            // The wrapped expression is the only non-keyword child. Field-name
            // access is grammar-version-dependent, so iterate children.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "await" {
                    return build_chain_inner(child, src, segments);
                }
            }
            None
        }

        // `obj.foo!()` — TS non-null assertion wraps the callee. Same shape
        // as await: peel one level and recurse on the inner expression.
        "non_null_expression" => {
            let inner = node.child(0)?;
            build_chain_inner(inner, src, segments)
        }

        // `f<T>()` — generic instantiation. Tree-sitter wraps the callee in
        // an `instantiation_expression { function, type_arguments }`. The
        // chain root is the inner function expression.
        "instantiation_expression" => {
            let func = node
                .child_by_field_name("function")
                .or_else(|| node.child(0))?;
            build_chain_inner(func, src, segments)
        }

        // `(expr).foo()` — parenthesized expression around the callee.
        "parenthesized_expression" => {
            if let Some(expr) = node.child_by_field_name("expression") {
                return build_chain_inner(expr, src, segments);
            }
            let mut cursor = node.walk();
            let mut inner = None;
            for child in node.children(&mut cursor) {
                if !matches!(child.kind(), "(" | ")") {
                    inner = Some(child);
                    break;
                }
            }
            build_chain_inner(inner?, src, segments)
        }

        // `new Foo().bar()` — the chain root is the constructed type. Recurse
        // into the `constructor` field so identifier (`Foo`) and member-
        // expression (`pkg.Sub.Class`) constructors both produce the
        // appropriate root segment, then keep walking up. Without this branch
        // the chain builder bails on every fluent-builder pattern
        // (NestJS DocumentBuilder, JS-class instances, Angular Forms
        // builders) and the call-site ref loses its receiver context.
        "new_expression" => {
            let constructor = node
                .child_by_field_name("constructor")
                .or_else(|| node.child(1))?;
            // Strip generic-type-arguments wrapper from the constructor —
            // `new Map<string, User>()` parses with a generic_type
            // wrapping the bare class identifier.
            let target = if constructor.kind() == "generic_type" {
                constructor
                    .child_by_field_name("name")
                    .unwrap_or(constructor)
            } else {
                constructor
            };
            build_chain_inner(target, src, segments)
        }

        // `(x as Foo).bar()` / `(x satisfies Foo).bar()` — peel the cast.
        "as_expression" | "satisfies_expression" | "type_assertion" => {
            // First non-type child is the underlying expression.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let k = child.kind();
                if !matches!(k, "as" | "satisfies" | "<" | ">" | "type_identifier"
                    | "predefined_type" | "generic_type" | "union_type"
                    | "intersection_type" | "literal_type" | "tuple_type"
                    | "array_type" | "object_type" | "type_predicate"
                    | "function_type" | "constructor_type" | "conditional_type"
                    | "indexed_access_type" | "lookup_type" | "mapped_type"
                    | "template_literal_type" | "type_query" | "this_type"
                    | "readonly")
                {
                    return build_chain_inner(child, src, segments);
                }
            }
            None
        }

        // Unknown node — can't build a chain from this.
        _ => None,
    }
}

/// Fallback for when `build_chain()` returns `None`.
pub(super) fn callee_name_fallback(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_expression" => {
            // Fall back to just the property name (last segment).
            node.child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(node, src))
        }
        _ => {
            let t = node_text(node, src);
            sanitize_callee_text(&t)
        }
    }
}

/// Last-resort sanitisation when neither the chain walker nor the typed
/// fallbacks could narrow the callee to a single identifier — strip the
/// surface-syntax wrappers that would otherwise pollute `target_name`.
///
/// Without this, multi-token texts like `await jsonlStreamConsumer` or
/// `getInitialProps!` flow through verbatim and never match a real symbol
/// in the index. The chain walker handles the same forms structurally
/// (see the new `await_expression` / `non_null_expression` arms in
/// `build_chain_inner`); this helper is the safety net for AST shapes
/// that don't reach the structured path.
fn sanitize_callee_text(raw: &str) -> String {
    let mut s = raw.trim();
    // Peel any number of leading `await ` prefixes.
    while let Some(rest) = s.strip_prefix("await ") {
        s = rest.trim_start();
    }
    // After a non-null assertion, drop the trailing `!`.
    let s = s.trim_end_matches('!');
    // If a generic instantiation leaked through (`fn<T>`), keep only the
    // identifier preceding the angle-bracket.
    let s = s.split('<').next().unwrap_or(s);
    // Member expression — keep the last segment.
    let s = s.rsplit('.').next().unwrap_or(s);
    let s = s.trim();
    // Final guard: if the survivor still contains characters that can't
    // appear in a JS identifier, the source AST shape was something we
    // don't understand (ternary callee `(cond ? a : b)(...)`, dynamic
    // `import(...)` as callee, IIFE bodies, etc.). Returning the literal
    // text leaks garbage like `skip : describe)` into target_name. Reject
    // and let the caller drop the ref.
    if s.is_empty() || !is_js_identifier(s) {
        return String::new();
    }
    s.to_string()
}

/// `true` when `s` is a valid JavaScript identifier — first char is letter,
/// `_` or `$`; rest are alphanumeric, `_`, or `$`. Conservative: rejects
/// anything with whitespace, parens, colons, backticks, generics, or
/// any punctuation that would never resolve to a real symbol.
fn is_js_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    if !(first.is_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}
