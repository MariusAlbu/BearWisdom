// =============================================================================
// javascript/calls.rs — call-site analysis for the JavaScript extractor
//
// Owns the `Calls`-edge pipeline for `call_expression`, `new_expression`,
// `tagged_template_expression`, and JSX component invocations, plus the
// helpers that decide whether a callee is real (parameter-shadow filter,
// keyword filter, callee-name unwrap).
// =============================================================================

use super::helpers::node_text;
use super::imports::{extract_first_string_arg, extract_require_path};
use crate::languages::common::build_member_chain;
use crate::types::{EdgeKind, ExtractedRef as Ref};
use tree_sitter::Node;

/// Recursively scan `node` for invocation forms and emit `Calls` / `Imports`
/// refs attributed to `source_symbol_index`.
pub(super) fn extract_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(func_node) = child.child_by_field_name("function") {
                    // Build a structured chain; fall back to plain callee name.
                    let chain = build_member_chain(func_node, src);
                    let callee = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name(func_node, src));

                    // require('foo') → Imports edge instead of Calls
                    if callee == "require" {
                        if let Some(module) = extract_require_path(&child, src) {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: module.clone(),
                                kind: EdgeKind::Imports,
                                line: child.start_position().row as u32,
                                module: Some(module),
                                chain: None,
                                byte_offset: child.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                                    col: 0,
                                });
                        }
                    }
                    // import('foo') — dynamic import → Imports edge
                    else if callee == "import" {
                        if let Some(module) = extract_first_string_arg(&child, src) {
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: module.clone(),
                                kind: EdgeKind::Imports,
                                line: child.start_position().row as u32,
                                module: Some(module),
                                chain: None,
                                byte_offset: child.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                                    col: 0,
                                });
                        }
                    }
                    // Regular call — emit with chain for member access resolution.
                    else if !callee.is_empty() {
                        // Parameter-shadow filter — see `emit_call_ref_js`
                        // comment for rationale. Receiver-first on chains
                        // of ≥2 segments; callee-only for bare calls.
                        let receiver_name = chain
                            .as_ref()
                            .filter(|c| c.segments.len() >= 2)
                            .and_then(|c| c.segments.first())
                            .map(|s| s.name.as_str());
                        let shadowed = match receiver_name {
                            Some(recv) => is_enclosing_function_parameter(func_node, src, recv),
                            None => is_enclosing_function_parameter(func_node, src, &callee),
                        };
                        if !shadowed {
                            crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
                            refs.push(Ref {
                                source_symbol_index,
                                target_name: callee,
                                kind: EdgeKind::Calls,
                                line: func_node.start_position().row as u32,
                                module: None,
                                chain,
                                byte_offset: func_node.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                                    col: 0,
                                });
                        }
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            // `new Foo(args)` → Calls edge to the constructor
            "new_expression" => {
                if let Some(constructor) = child.child_by_field_name("constructor") {
                    let name = callee_name(constructor, src);
                    if !name.is_empty() {
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: constructor.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: constructor.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                                col: 0,
                            });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            // `` html`<div>` `` / `` gql`query {}` `` — tag is the called function.
            "tagged_template_expression" => {
                if let Some(tag) = child.child_by_field_name("tag") {
                    let name = callee_name(tag, src);
                    if !name.is_empty() {
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: tag.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: tag.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                                col: 0,
                            });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            // JSX: `<Component />`, `<Component>...</Component>`, or the
            // member-expression form `<Foo.Bar />` that React Context uses
            // for `<MyContext.Provider value={x}>`. Emit a Calls edge only
            // for user-defined components (PascalCase first char). Lowercase
            // tags like `<div>` / `<span>` are HTML intrinsics — not graph-
            // resolvable symbols — and were previously emitted as `type_ref`
            // refs that polluted `unresolved_refs`.
            //
            // For the member-expression form we emit the TAIL segment name
            // (`Provider`) as `target_name` with a structured MemberChain
            // `[MyContext, Provider]` so the chain walker can resolve the
            // receiver's inferred React.Context<T> type to the Provider
            // member. Previously we stuffed the full `"MyContext.Provider"`
            // string into `target_name` — which never resolved against any
            // symbol and polluted unresolved_refs on every .jsx file that
            // used the Context pattern. Matches the TypeScript extractor.
            "jsx_self_closing_element" | "jsx_opening_element" => {
                let tag = child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0));
                if let Some(tag_node) = tag {
                    let tag_name = node_text(tag_node, src);
                    let is_component = tag_name.chars().next().map_or(false, |c| c.is_uppercase());
                    if !tag_name.is_empty() && is_component {
                        let chain = build_member_chain(tag_node, src);
                        let target = chain
                            .as_ref()
                            .and_then(|c| c.segments.last())
                            .map(|s| s.name.clone())
                            .unwrap_or(tag_name);
                        crate::languages::emit_chain_type_ref(
                            &chain,
                            source_symbol_index,
                            &tag_node,
                            refs,
                        );
                        refs.push(Ref {
                            source_symbol_index,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line: tag_node.start_position().row as u32,
                            module: None,
                            chain,
                            byte_offset: tag_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                                col: 0,
                            });
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Resolve a callee node to its bare function/method name.
///
/// For `member_expression` returns only the property (last) segment — the
/// receiver must come from chain walking, not from concatenation. Returns
/// `""` for IIFEs and other dynamic callees so callers can skip emission.
pub(super) fn callee_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_expression" => {
            // Return ONLY the property name (the last segment), matching the
            // TypeScript extractor convention and the `ExtractedRef.target_name`
            // contract ("For chain-bearing refs, this is the LAST segment name").
            //
            // The prior implementation concatenated `obj.prop` using the raw
            // text of the object sub-tree. That was catastrophic for any
            // chain whose receiver was itself a call expression (Chai / Jasmine
            // assertions):
            //   expect(scratch.innerHTML).to.equal
            // was stored as a single target_name of literally that whole
            // string, inflating `unresolved_refs` by thousands of rows per
            // test-heavy project (javascript-preact alone: ~2,500 such refs).
            //
            // The resolver key is the method name; receiver context should
            // come from chain walking, not from stuffing it into target_name.
            node.child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(node, src))
        }
        // Unwrap parens and recurse: `(foo.bar)()` → `bar`.
        "parenthesized_expression" => node
            .named_child(0)
            .map(|inner| callee_name(inner, src))
            .unwrap_or_default(),
        // Keyword-shaped callees: `import('mod')` (dynamic import) and
        // `super(...)` both parse with a keyword node as the call's
        // function. The call sites downstream branch on the string value
        // (`"import"` → Imports edge; `"super"` → filtered as a keyword).
        "import" | "super" => node_text(node, src),
        // IIFEs (`(function(){})()`, `(() => {})()`) and other dynamic
        // callees have no named target — emitting a ref for them used to
        // dump the whole function body source into `target_name` via the
        // `rsplit('.')` fallback, producing garbage like:
        //     "data;\n            });\n        }\n\n        init();\n    }\n})"
        // that polluted `unresolved_refs` (SimplCommerce alone: ~100 such
        // rows from AngularJS module IIFEs). No target name ⇒ no ref.
        _ => String::new(),
    }
}

/// Emit a Calls ref for a single `call_expression` node.
///
/// Mirrors the TypeScript `calls::emit_call_ref` but uses the JS-local
/// `callee_name` helper and handles `require`/`import` as Imports edges.
pub(super) fn emit_call_ref_js(
    call_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    let Some(func_node) = call_node.child_by_field_name("function") else {
        return;
    };
    // Build a structured chain first; fall back to plain callee name when the
    // node isn't chainable (e.g. an anonymous arrow-function callee).
    let chain = build_member_chain(func_node, src);
    let callee = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| callee_name(func_node, src));

    if callee == "require" {
        if let Some(module) = extract_require_path(call_node, src) {
            refs.push(Ref {
                source_symbol_index,
                target_name: module.clone(),
                kind: EdgeKind::Imports,
                line: call_node.start_position().row as u32,
                module: Some(module),
                chain: None,
                byte_offset: call_node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
                    col: 0,
                });
        }
    } else if callee == "import" {
        if let Some(module) = extract_first_string_arg(call_node, src) {
            refs.push(Ref {
                source_symbol_index,
                target_name: module.clone(),
                kind: EdgeKind::Imports,
                line: call_node.start_position().row as u32,
                module: Some(module),
                chain: None,
                byte_offset: call_node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
                    col: 0,
                });
        }
    } else if !callee.is_empty() && !is_js_keyword(&callee) {
        // Parameter-shadow filter: `(setter) => setter(e.target.value)`
        // emits a Calls ref for `setter` that can never resolve because
        // the binding is a local parameter, not a declared function. Same
        // for chain receivers — `(function ($, currentSearchOption) { …
        // currentSearchOption.hasOwnProperty(k) … })` puts
        // `currentSearchOption` in `unresolved_refs` as a TypeRef. Drop
        // both forms when the target matches an enclosing parameter.
        let receiver_name = chain
            .as_ref()
            .filter(|c| c.segments.len() >= 2)
            .and_then(|c| c.segments.first())
            .map(|s| s.name.as_str());
        if let Some(recv) = receiver_name {
            if is_enclosing_function_parameter(func_node, src, recv) {
                return;
            }
        } else if is_enclosing_function_parameter(func_node, src, &callee) {
            return;
        }
        // Emit a TypeRef for the chain receiver when it looks like a type name.
        crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
        refs.push(Ref {
            source_symbol_index,
            target_name: callee,
            kind: EdgeKind::Calls,
            line: func_node.start_position().row as u32,
            module: None,
            chain,
            byte_offset: func_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
                col: 0,
            });
    }
}

/// JS reserved words that should never be emitted as call targets.
/// Tree-sitter-javascript produces `super(...)` as a `call_expression` whose
/// `function` field is literally the identifier `super`. That leaks into
/// `unresolved_refs` as a `super` target (450+ refs in javascript-preact
/// alone). Similarly for `import(...)`, `new.target`, etc. — all keywords
/// the resolver has no business looking up against the symbol index.
fn is_js_keyword(name: &str) -> bool {
    matches!(
        name,
        "super" | "this" | "new" | "typeof" | "instanceof" | "void"
            | "yield" | "await" | "delete" | "in" | "of" | "return"
            | "throw" | "try" | "catch" | "finally" | "debugger"
            | "if" | "else" | "switch" | "case" | "default" | "break"
            | "continue" | "for" | "while" | "do" | "function" | "class"
            | "extends" | "const" | "let" | "var" | "static" | "async"
            | "true" | "false" | "null" | "undefined"
    )
}

/// True when `name` is bound as a parameter of any enclosing function in
/// the AST (function declaration, function expression, arrow function,
/// method, generator). Used to filter unresolved-ref emission for chain
/// walkers whose receiver / callee / iterable is a local parameter rather
/// than a global type or function — the resolver has nothing to point it
/// at and `unresolved_refs` would only carry noise.
///
/// Walks the parent chain from `at` up to the program root. Handles
/// destructured (`{ a, b }`), array (`[x, y]`), rest (`...rest`), and
/// default (`x = 1`) parameter forms.
pub(super) fn is_enclosing_function_parameter(at: Node, src: &[u8], name: &str) -> bool {
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
                if parameter_list_binds(params, src, name) {
                    return true;
                }
            }
        }
        cur = parent;
    }
    false
}

/// True when `name` appears as a binding identifier in a function
/// parameter list node (`formal_parameters`, or a single `identifier` in
/// the arrow-short form `x => …`). Recurses through patterns to handle
/// destructuring, rest, and defaults.
fn parameter_list_binds(params: Node, src: &[u8], name: &str) -> bool {
    if pattern_binds_name(params, src, name) {
        return true;
    }
    let mut cursor = params.walk();
    for child in params.named_children(&mut cursor) {
        if pattern_binds_name(child, src, name) {
            return true;
        }
    }
    false
}

/// Recursive walk of a binding pattern to check whether `name` appears
/// as one of the introduced identifiers. Covers:
///   - plain identifier:     `x`
///   - object pattern:       `{ a, b: c, ...rest }`
///   - array pattern:        `[x, y, ...z]`
///   - rest pattern:         `...rest`
///   - default-value:        `x = 1`
///   - assignment pattern:   `{ a = 1 }`
///   - typed params:         tree-sitter-typescript wraps these but the
///                           JS grammar also accepts them in .tsx files.
fn pattern_binds_name(node: Node, src: &[u8], name: &str) -> bool {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            node_text(node, src) == name
        }
        "rest_pattern" | "spread_element" => {
            // `...rest` — the identifier lives under the rest marker.
            node.named_child(0)
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        "assignment_pattern" => {
            // `x = default` — check the left (the bound name).
            node.child_by_field_name("left")
                .or_else(|| node.named_child(0))
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        "object_pattern" | "array_pattern" | "object_assignment_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if pattern_binds_name(child, src, name) {
                    return true;
                }
            }
            false
        }
        "pair_pattern" => {
            // `{ a: b }` — the bound name is the VALUE, not the key.
            node.child_by_field_name("value")
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        "required_parameter" | "optional_parameter" | "formal_parameters" => {
            let inner = node
                .child_by_field_name("pattern")
                .or_else(|| node.named_child(0));
            inner
                .map(|c| pattern_binds_name(c, src, name))
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Emit a Calls ref for a single `new_expression` node.
pub(super) fn emit_new_ref_js(
    new_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    let Some(constructor) = new_node.child_by_field_name("constructor") else {
        return;
    };
    let name = callee_name(constructor, src);
    if !name.is_empty() {
        refs.push(Ref {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::Calls,
            line: constructor.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: constructor.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
                col: 0,
            });
    }
}
