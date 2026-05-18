use super::helpers::node_text;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Extract the positional arguments from a `call_expression`'s `arguments` node.
///
/// Walks named children of the `arguments` list, converting each to a `CallArg`.
/// Handles string literals, template literals with/without interpolation, tagged
/// template bodies, bare identifiers, and numeric/boolean literals.
pub(super) fn extract_call_args(call_node: &Node, src: &[u8]) -> Vec<CallArg> {
    let Some(args_node) = call_node.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        let arg = match child.kind() {
            "string" => {
                // `"text"` or `'text'` — strip surrounding quotes.
                let raw = node_text(child, src);
                let inner = raw
                    .trim_start_matches(['"', '\'', '`'])
                    .trim_end_matches(['"', '\'', '`'])
                    .to_string();
                CallArg::StringLit(inner)
            }
            "template_string" => {
                // `` `text ${expr} more` `` — check for substitution children.
                let has_substitution = (0..child.child_count()).any(|i| {
                    child.child(i)
                        .map(|c| c.kind() == "template_substitution")
                        .unwrap_or(false)
                });
                if has_substitution {
                    // Replace each `${...}` span with `{}` placeholder.
                    let raw = node_text(child, src);
                    let replaced = replace_template_substitutions(&raw);
                    CallArg::TemplateLit(replaced)
                } else {
                    // No interpolation — treat as a plain string literal.
                    let raw = node_text(child, src);
                    let inner = raw.trim_matches('`').to_string();
                    CallArg::StringLit(inner)
                }
            }
            "tagged_template_expression" => {
                // `` gql`query Foo { ... }` `` — capture tag + body.
                let tag_node = child.child_by_field_name("tag");
                let tmpl_node = child.child_by_field_name("template");
                let tag = tag_node.map(|n| node_text(n, src)).unwrap_or_default();
                let body = tmpl_node
                    .map(|n| {
                        let raw = node_text(n, src);
                        raw.trim_matches('`').to_string()
                    })
                    .unwrap_or_default();
                CallArg::TaggedTemplate { tag, body }
            }
            "identifier" => CallArg::Ident(node_text(child, src)),
            "number" => CallArg::Literal(node_text(child, src)),
            "true" | "false" | "null" | "undefined" => {
                CallArg::Literal(child.kind().to_string())
            }
            "object" => {
                let pairs = extract_object_property_pairs(&child, src);
                if pairs.is_empty() {
                    CallArg::Other
                } else {
                    CallArg::ObjectKeys(pairs)
                }
            }
            _ => CallArg::Other,
        };
        result.push(arg);
    }
    result
}

/// Replace `${...}` spans in a raw template literal text with `{}` placeholders.
#[cfg_attr(test, allow(dead_code))]
pub(super) fn replace_template_substitutions(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut depth = 1usize;
            while let Some(inner) = chars.next() {
                match inner {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 { break; }
                    }
                    _ => {}
                }
            }
            out.push_str("{}");
        } else {
            out.push(c);
        }
    }
    out
}

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
            let call_args = extract_call_args(call_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name,
                kind: EdgeKind::Calls,
                line: func_node.start_position().row as u32,
                col: 0,
                module: None,
                chain,
                byte_offset: func_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
});
        }
    }
}

/// Emit an Instantiates ref for a single `new_expression` node.
///
/// Populates `call_args` and a single-segment `chain` with the constructor
/// name so flow-emission detectors can recognise constructor-keyed patterns
/// (`new Worker('queue-name', processor)`) using the same chain + call-args
/// surface as method calls.
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
            let call_args = extract_call_args(new_node, src);
            let chain = Some(MemberChain {
                segments: vec![ChainSegment {
                    name: name.clone(),
                    node_kind: constructor.kind().to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                    byte_offset: 0,
                                    declared_type_id: None,
                    type_arg_ids: Vec::new(),
}],
            });
            // Side-channel synthetic ref: when this `new X(...)` is the
            // initializer of `const/let/var bound = new X(constructor_arg)`,
            // emit an EdgeKind::Imports ref keyed `__ts_bgjob_queue_binding__:<bound>`
            // with `module` set to the first string constructor argument.
            // The TS resolver skips Imports-kind refs and the file-context
            // import loop picks the entry up, exposing the binding to
            // file-scoped detectors without polluting any resolution path.
            if let (Some(bound), Some(queue_name)) = (
                binding_name_of_new(new_node, src),
                first_string_arg_text(new_node, src),
            ) {
                if matches!(name.as_str(), "Queue" | "Worker") {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: format!("__ts_bgjob_queue_binding__:{}", bound),
                        kind: EdgeKind::Imports,
                        line: constructor.start_position().row as u32,
                        col: 0,
                        module: Some(queue_name),
                        chain: None,
                        byte_offset: constructor.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Instantiates,
                line: constructor.start_position().row as u32,
                col: 0,
                module: None,
                chain,
                byte_offset: constructor.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
});
        }
    }
}

/// If `new_node` sits directly as the initializer of a `variable_declarator`
/// (`const name = new X(...)`), return the bound identifier text. Walks up
/// only one level — chained initializers (`const x = (new X()).y`) intentionally
/// don't qualify.
fn binding_name_of_new(new_node: &Node, src: &[u8]) -> Option<String> {
    let parent = new_node.parent()?;
    if parent.kind() != "variable_declarator" {
        return None;
    }
    let name_node = parent.child_by_field_name("name")?;
    if name_node.kind() != "identifier" {
        return None;
    }
    Some(node_text(name_node, src))
}

/// Collect statically-determinable `(key, optional string-literal value)`
/// pairs from an `object` (object literal) node. Recognises shorthand
/// identifiers (`{ foo }`), explicit pairs (`{ foo: handler }` /
/// `{ foo: 'bar' }`), method shorthand (`{ getUser(...) { ... } }`), and
/// string-keyed entries (`{ "foo": ... }`). The value slot is populated
/// only when the value is a plain string literal or a flat template literal
/// without interpolation; identifiers, function expressions, member
/// accesses, and computed values all leave the slot `None`. Computed keys
/// (`{ [dyn]: v }`) and spread elements (`{ ...rest }`) are skipped.
fn extract_object_property_pairs(
    object_node: &Node,
    src: &[u8],
) -> Vec<(String, Option<String>)> {
    let mut pairs = Vec::new();
    let mut cursor = object_node.walk();
    for child in object_node.named_children(&mut cursor) {
        match child.kind() {
            "pair" => {
                let Some(key_node) = child.child_by_field_name("key") else { continue; };
                let name = match key_node.kind() {
                    "property_identifier" | "identifier" => node_text(key_node, src),
                    "string" => node_text(key_node, src)
                        .trim_start_matches(['"', '\'', '`'])
                        .trim_end_matches(['"', '\'', '`'])
                        .to_string(),
                    _ => continue,
                };
                if name.is_empty() {
                    continue;
                }
                let value = child.child_by_field_name("value").and_then(|v| {
                    match v.kind() {
                        "string" => Some(
                            node_text(v, src)
                                .trim_start_matches(['"', '\'', '`'])
                                .trim_end_matches(['"', '\'', '`'])
                                .to_string(),
                        ),
                        "template_string" => {
                            let has_subst = (0..v.child_count()).any(|i| {
                                v.child(i)
                                    .map(|c| c.kind() == "template_substitution")
                                    .unwrap_or(false)
                            });
                            if has_subst {
                                None
                            } else {
                                Some(node_text(v, src).trim_matches('`').to_string())
                            }
                        }
                        _ => None,
                    }
                });
                pairs.push((name, value));
            }
            "shorthand_property_identifier" | "property_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    pairs.push((name, None));
                }
            }
            "method_definition" => {
                if let Some(key_node) = child.child_by_field_name("name") {
                    let name = node_text(key_node, src);
                    if !name.is_empty() {
                        pairs.push((name, None));
                    }
                }
            }
            _ => {}
        }
    }
    pairs
}

/// Return the first positional argument of `new_node` when it is a plain
/// string literal (`new X("name")`). Returns `None` for template literals
/// with substitutions, identifiers, or non-literal argument shapes.
fn first_string_arg_text(new_node: &Node, src: &[u8]) -> Option<String> {
    let args_node = new_node.child_by_field_name("arguments")?;
    let mut cursor = args_node.walk();
    let first = args_node.named_children(&mut cursor).next()?;
    match first.kind() {
        "string" => {
            let raw = node_text(first, src);
            Some(
                raw.trim_start_matches(['"', '\'', '`'])
                    .trim_end_matches(['"', '\'', '`'])
                    .to_string(),
            )
        }
        "template_string" => {
            // Only flat templates (no substitution) qualify.
            let has_subst = (0..first.child_count()).any(|i| {
                first.child(i)
                    .map(|c| c.kind() == "template_substitution")
                    .unwrap_or(false)
            });
            if has_subst {
                None
            } else {
                let raw = node_text(first, src);
                Some(raw.trim_matches('`').to_string())
            }
        }
        _ => None,
    }
}

/// Recognise `process.env.<KEY>`, `process.env['KEY']`,
/// `import.meta.env.<KEY>`, and feature-flag-shaped member access
/// (`featureFlags.<flag>`, `<...FeatureFlag(s|Manager)?>.<flag>`,
/// `<features>.<flag>`) and emit a synthetic TypeRef ref carrying the chain
/// so the flow-emission layer can pull out a `ConfigLookup` or
/// `FeatureFlag` key. Returns silently for any other member/subscript
/// expression so unrelated shapes (`obj.field`, `arr[i]`) emit no extra refs.
fn emit_config_lookup_ref(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let key = match node.kind() {
        // `process.env.NODE_ENV` — object is `process.env`, property is the key.
        // `featureFlags.someFlag` — object is `featureFlags`, property is the flag.
        "member_expression" => {
            let object = match node.child_by_field_name("object") { Some(o) => o, None => return };
            let property = match node.child_by_field_name("property") { Some(p) => p, None => return };
            if property.kind() != "property_identifier" { return; }
            let env_match = is_env_object(&object, src);
            let ff_match = !env_match && is_feature_flag_root(&object, src);
            if !env_match && !ff_match { return; }
            node_text(property, src)
        }
        // `process.env['NODE_ENV']` — subscript with object and a string index.
        "subscript_expression" => {
            let object = match node.child_by_field_name("object") { Some(o) => o, None => return };
            let index = match node.child_by_field_name("index") { Some(i) => i, None => return };
            if !is_env_object(&object, src) { return; }
            if index.kind() != "string" { return; }
            node_text(index, src)
                .trim_start_matches(['"', '\'', '`'])
                .trim_end_matches(['"', '\'', '`'])
                .to_string()
        }
        _ => return,
    };
    if key.is_empty() {
        return;
    }
    let chain = build_chain(*node, src);
    // Use `EdgeKind::Imports`: the resolver classifies Imports refs as
    // external rather than unresolved, so the synthetic refs don't inflate
    // the `unresolved_refs` table even though they have no actual import
    // target. The flow detector still receives them via the dispatcher's
    // Imports-kind chain branch.
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: key,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// Recognise a feature-flag-shaped chain receiver. Matches when the root
/// identifier is `featureFlags`, `features`, `flags`, or contains
/// `featureFlag` / `featureflag` as a substring (case-insensitive). Also
/// peers through one intermediate `value` accessor (Svelte runes /
/// `Manager` singleton pattern) so `featureFlagsManager.value.<flag>`
/// works.
fn is_feature_flag_root(node: &Node, src: &[u8]) -> bool {
    match node.kind() {
        "identifier" => is_ff_identifier_name(&node_text(*node, src)),
        "member_expression" => {
            // `<ff>.value` — peer through.
            let inner_obj = match node.child_by_field_name("object") { Some(o) => o, None => return false };
            let inner_prop = match node.child_by_field_name("property") { Some(p) => p, None => return false };
            if inner_obj.kind() != "identifier" { return false; }
            if inner_prop.kind() != "property_identifier" { return false; }
            if node_text(inner_prop, src) != "value" { return false; }
            is_ff_identifier_name(&node_text(inner_obj, src))
        }
        _ => false,
    }
}

fn is_ff_identifier_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "featureflags" | "features" | "flags")
        || lower.contains("featureflag")
}

/// Return true when `node` represents the `process.env` or `import.meta.env`
/// receiver of an env-var lookup. Anything else (a custom `env` object, a
/// type expression, etc.) is rejected.
fn is_env_object(node: &Node, src: &[u8]) -> bool {
    if node.kind() != "member_expression" {
        return false;
    }
    let object = match node.child_by_field_name("object") { Some(o) => o, None => return false };
    let property = match node.child_by_field_name("property") { Some(p) => p, None => return false };
    if property.kind() != "property_identifier" || node_text(property, src) != "env" {
        return false;
    }
    match object.kind() {
        "identifier" => node_text(object, src) == "process",
        "member_expression" => {
            // `import.meta` — `import` keyword as the root, `meta` as the prop.
            let inner_obj = match object.child_by_field_name("object") { Some(o) => o, None => return false };
            let inner_prop = match object.child_by_field_name("property") { Some(p) => p, None => return false };
            inner_obj.kind() == "import"
                && inner_prop.kind() == "property_identifier"
                && node_text(inner_prop, src) == "meta"
        }
        _ => false,
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
                        let call_args = extract_call_args(&child, src);
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: func_node.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: func_node.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args,
});
                    }
                }
                extract_calls(&child, src, source_symbol_index, refs);
            }
            "new_expression" => {
                emit_new_ref(&child, src, source_symbol_index, refs);
                extract_calls(&child, src, source_symbol_index, refs);
            }
            // `process.env.X` / `process.env['X']` / `import.meta.env.X` —
            // emit a synthetic TypeRef ref with a chain so the resolver's
            // flow-emission detector can recognise the ConfigLookup shape.
            // Only fires for these specific top-level identifier roots so
            // unrelated member-access expressions don't get extra refs.
            "member_expression" | "subscript_expression" => {
                emit_config_lookup_ref(&child, src, source_symbol_index, refs);
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
                        // For tagged templates the body is the sole "argument".
                        let call_args = child.child_by_field_name("template")
                            .map(|tmpl| {
                                let raw = node_text(tmpl, src);
                                let body = raw.trim_matches('`').to_string();
                                vec![CallArg::TaggedTemplate { tag: target_name.clone(), body }]
                            })
                            .unwrap_or_default();
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: tag.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: tag.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args,
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
        col: 0,
        module: None,
        chain,
        byte_offset: tag_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
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
