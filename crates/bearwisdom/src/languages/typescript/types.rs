use super::helpers::node_text;
use crate::types::{AliasTarget, EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Classify the right-hand side of a `type_alias_declaration` into a
/// structural [`AliasTarget`].
///
/// `value_node` is the node returned by `type_alias_declaration.value`
/// (i.e. the type expression on the right of `=`). The classifier
/// unwraps `parenthesized_type` / `readonly_type` wrappers so they
/// don't disguise the inner shape, then dispatches on node kind.
///
/// The shape is captured at extract time so the chain walker can avoid
/// re-parsing — and so unions / intersections can't be silently
/// mistaken for single-type applications when the engine flattens
/// `TypeRef`s into a positional list (which loses the union vs.
/// generic-args distinction).
pub(super) fn classify_alias_target(value_node: &Node, src: &[u8]) -> AliasTarget {
    let mut node = *value_node;
    // Unwrap transparent wrappers so they don't bury the real shape.
    loop {
        match node.kind() {
            "parenthesized_type" | "readonly_type" => {
                let mut found = None;
                for i in 0..node.child_count() {
                    let Some(child) = node.child(i) else { continue };
                    if matches!(child.kind(), "(" | ")" | "readonly") {
                        continue;
                    }
                    found = Some(child);
                    break;
                }
                match found {
                    Some(inner) => node = inner,
                    None => break,
                }
            }
            _ => break,
        }
    }

    match node.kind() {
        "type_identifier" | "identifier" => AliasTarget::Application {
            root: node_text(node, src),
            args: Vec::new(),
        },
        "nested_type_identifier" | "member_expression" => AliasTarget::Application {
            root: node_text(node, src),
            args: Vec::new(),
        },
        "generic_type" => {
            let root = node
                .child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            let mut args: Vec<String> = Vec::new();
            if let Some(type_args_node) = node.child_by_field_name("type_arguments") {
                for i in 0..type_args_node.child_count() {
                    let Some(arg) = type_args_node.child(i) else { continue };
                    if matches!(arg.kind(), "<" | ">" | ",") {
                        continue;
                    }
                    let arg_name = head_type_name(&arg, src);
                    if !arg_name.is_empty() {
                        args.push(arg_name);
                    }
                }
            }
            AliasTarget::Application { root, args }
        }
        // `User[]` is equivalent to `Array<User>` in TypeScript's type
        // system. Treating it as `Application { root: "Array", args: [User] }`
        // means the chain walker can dereference `arr.map(...)` /
        // `arr.filter(...)` to `Array.map` / `Array.filter` in lib.es5.d.ts
        // through the same alias-expansion path that handles the explicit
        // generic form.
        "array_type" => {
            let mut element = String::new();
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "[" | "]") {
                    continue;
                }
                element = head_type_name(&child, src);
                if !element.is_empty() {
                    break;
                }
            }
            AliasTarget::Application {
                root: "Array".to_string(),
                args: if element.is_empty() {
                    Vec::new()
                } else {
                    vec![element]
                },
            }
        }
        "union_type" => {
            let mut branches = Vec::new();
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if child.kind() == "|" {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    branches.push(name);
                }
            }
            AliasTarget::Union(branches)
        }
        "intersection_type" => {
            let mut branches = Vec::new();
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if child.kind() == "&" {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    branches.push(name);
                }
            }
            AliasTarget::Intersection(branches)
        }
        "object_type" => AliasTarget::Object,
        // `type X = typeof someValue` — the alias resolves to whatever
        // type the value reference has. Capture the value's name as
        // written; the chain walker later looks up its `field_type` /
        // `return_type` to continue.
        "type_query" => {
            // tree-sitter exposes the referenced name via the `name`
            // field on `type_query`, but synthetic test grammars and
            // some real-world parses don't always populate it — fall
            // back to the first non-keyword child the way the existing
            // TypeRef extractor does.
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, src);
                if !name.is_empty() {
                    return AliasTarget::Typeof(name);
                }
            }
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if child.kind() == "typeof" {
                    continue;
                }
                let name = node_text(child, src);
                if !name.is_empty() {
                    return AliasTarget::Typeof(name);
                }
            }
            AliasTarget::Other
        }
        // Everything else — `keyof T`, mapped, conditional,
        // indexed-access, template-literal, function types, tuples,
        // type predicates, infer, this, literals — is a non-application
        // shape we don't expand yet. Recorded as `Other` so callers
        // don't fall back to the field_type heuristic.
        _ => AliasTarget::Other,
    }
}

/// Best-effort head name of a type expression. Returns the simple name
/// for `type_identifier` / `identifier` / `generic_type` (just the
/// `name` field, not its args), the dotted text for
/// `nested_type_identifier` / `member_expression`, the element-type
/// head for `array_type`, and an empty string for shapes whose head
/// can't be reduced to a single name (unions, intersections, mapped,
/// conditional, etc.).
fn head_type_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" | "identifier" => node_text(*node, src),
        "nested_type_identifier" | "member_expression" => node_text(*node, src),
        "generic_type" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, src))
            .unwrap_or_default(),
        "array_type" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "[" | "]") {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "parenthesized_type" | "readonly_type" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "(" | ")" | "readonly") {
                    continue;
                }
                return head_type_name(&child, src);
            }
            String::new()
        }
        _ => String::new(),
    }
}

pub(super) fn extract_type_ref_from_annotation(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk into type_annotation → the actual type node.
    // type_annotation children: ":" + the type itself
    let type_node = if node.kind() == "type_annotation" {
        let count = node.child_count();
        let mut found = None;
        for i in 0..count {
            if let Some(child) = node.child(i) {
                if child.kind() != ":" {
                    found = Some(child);
                    break;
                }
            }
        }
        found
    } else {
        Some(*node)
    };
    let Some(type_node) = type_node else { return };

    match type_node.kind() {
        "type_identifier" | "identifier" => {
            let type_name = node_text(type_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        "generic_type" => {
            // Repository<User> → extract "Repository" as the ref target,
            // but also emit a second ref with the full generic text for
            // the field_type map to capture type arguments.
            if let Some(name) = type_node.child_by_field_name("name") {
                let base_name = node_text(name, src);
                // Emit base type ref (for edge resolution to the type itself).
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: base_name.clone(),
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
                // Also extract type arguments for generic parameter resolution.
                if let Some(type_args_node) = type_node.child_by_field_name("type_arguments") {
                    for i in 0..type_args_node.child_count() {
                        if let Some(arg) = type_args_node.child(i) {
                            if matches!(
                                arg.kind(),
                                "type_identifier" | "identifier" | "generic_type" | "array_type"
                            ) {
                                let arg_name = match arg.kind() {
                                    "generic_type" => arg
                                        .child_by_field_name("name")
                                        .map(|n| node_text(n, src))
                                        .unwrap_or_default(),
                                    "array_type" => {
                                        // User[] → extract "User"
                                        let mut found_name = String::new();
                                        for j in 0..arg.child_count() {
                                            if let Some(child) = arg.child(j) {
                                                if child.kind() == "type_identifier"
                                                    || child.kind() == "identifier"
                                                {
                                                    found_name = node_text(child, src);
                                                    break;
                                                }
                                            }
                                        }
                                        found_name
                                    }
                                    _ => node_text(arg, src),
                                };
                                if !arg_name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index,
                                        target_name: arg_name,
                                        kind: EdgeKind::TypeRef,
                                        line: arg.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                        byte_offset: 0,
                                                                            namespace_segments: Vec::new(),
});
                                }
                            }
                        }
                    }
                }
            }
        }
        "nested_type_identifier" | "member_expression" => {
            // db.Kysely → extract the full dotted name
            let type_name = node_text(type_node, src);
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        "function_type" => {
            // (req: Request, res: Response) => void
            // Extract parameter types.
            if let Some(params) = type_node.child_by_field_name("parameters") {
                for i in 0..params.child_count() {
                    if let Some(param) = params.child(i) {
                        if param.kind() == "required_parameter"
                            || param.kind() == "optional_parameter"
                        {
                            if let Some(type_ann) = param.child_by_field_name("type") {
                                extract_type_ref_from_annotation(
                                    &type_ann,
                                    src,
                                    source_symbol_index,
                                    refs,
                                );
                            }
                        }
                    }
                }
            }
            // Return type: the child after "=>".
            let child_count = type_node.child_count();
            for i in 0..child_count {
                if let Some(child) = type_node.child(i) {
                    if child.kind() == "=>" {
                        if let Some(ret) = type_node.child(i + 1) {
                            extract_type_ref_from_annotation(
                                &ret,
                                src,
                                source_symbol_index,
                                refs,
                            );
                        }
                        break;
                    }
                }
            }
        }
        "union_type" => {
            // User | null  /  string | number
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() != "|" {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        "intersection_type" => {
            // Foo & Bar
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() != "&" {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        "array_type" => {
            // User[]  — element type is the child before "["
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() != "[" && child.kind() != "]" {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        "parenthesized_type" => {
            // (User | null)
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() != "(" && child.kind() != ")" {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        // `T extends U ? A : B` — extract all four type positions.
        "conditional_type" => {
            // Fields: left (check type), right (extends type), consequence, alternative.
            // tree-sitter uses: "left", "right" for extends pair, "consequence", "alternative".
            for field in &["left", "right", "consequence", "alternative"] {
                if let Some(child) = type_node.child_by_field_name(field) {
                    extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                }
            }
            // Also walk all children to catch any not covered by named fields.
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if !matches!(
                        child.kind(),
                        "extends" | "?" | ":" | "conditional_type"
                            | "type_identifier" | "identifier"
                    ) {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        // `{ [K in keyof T]: T[K] }` — extract constraint and value types.
        "mapped_type" => {
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    match child.kind() {
                        "{" | "}" | "[" | "]" | "in" | ":" | "readonly" | "?" | "+" | "-"
                        | "property_identifier" | "type_identifier" | "identifier" => {}
                        _ => {
                            extract_type_ref_from_annotation(
                                &child,
                                src,
                                source_symbol_index,
                                refs,
                            );
                        }
                    }
                }
            }
        }
        // `T[K]` — extract the object type.
        "indexed_access_type" => {
            if let Some(obj) = type_node.child_by_field_name("object") {
                extract_type_ref_from_annotation(&obj, src, source_symbol_index, refs);
            }
            if let Some(index) = type_node.child_by_field_name("index") {
                extract_type_ref_from_annotation(&index, src, source_symbol_index, refs);
            }
        }
        // `` `prefix_${string}` `` — recurse for any type refs inside.
        "template_literal_type" => {
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if !matches!(child.kind(), "`" | "${" | "}") {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        // `typeof Foo` — the referenced name is a value-space ref; emit as TypeRef.
        "type_query" => {
            // The expression after `typeof` is the referenced name.
            if let Some(expr) = type_node.child_by_field_name("name") {
                let name = node_text(expr, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: expr.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            } else {
                // Fallback: first named non-keyword child.
                for i in 0..type_node.child_count() {
                    if let Some(child) = type_node.child(i) {
                        if child.kind() != "typeof" {
                            let name = node_text(child, src);
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: child.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                                break;
                            }
                        }
                    }
                }
            }
        }
        // `keyof T` — tree-sitter uses `index_type_query` for this node.
        "keyof_type" | "index_type_query" => {
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() != "keyof" {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        // `readonly T[]` — unwrap and recurse.
        "readonly_type" => {
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() != "readonly" {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        // `x is Foo` — tree-sitter uses `type_predicate` (inside `type_predicate_annotation`).
        // Also handle the outer `type_predicate_annotation` wrapper.
        "predicate_type" | "type_predicate" | "type_predicate_annotation" => {
            // Walk children: skip the subject identifier and `is` keyword,
            // extract the asserted type (type_identifier or generic_type after `is`).
            let mut after_is = false;
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    match child.kind() {
                        "is" => {
                            after_is = true;
                        }
                        ":" => {}  // colon in type_predicate_annotation
                        _ => {
                            if after_is {
                                extract_type_ref_from_annotation(
                                    &child,
                                    src,
                                    source_symbol_index,
                                    refs,
                                );
                                break;
                            } else if child.kind() == "type_predicate" {
                                // Recurse into the inner type_predicate node.
                                extract_type_ref_from_annotation(
                                    &child,
                                    src,
                                    source_symbol_index,
                                    refs,
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }
        // `infer T` — T is a local binding, not a reference to an existing type.
        "infer_type" => {}
        // `this` — self-reference, nothing to extract.
        "this_type" => {}
        // Literal types (`"foo"`, `42`, `true`) — not type references.
        "literal_type" | "string" | "number" | "true" | "false" | "null" => {}
        // Object type literal `{ field: T; method(arg: U): V }`. Members are
        // emitted as Property/Method symbols via `extract_node` (driven by
        // `recurse_for_object_types` from the type_alias_declaration / interface
        // handlers). Recursing through the body here would emit the parameter
        // NAMES (`arg`, `args`, etc.) and property NAMES (`field`) as TypeRefs,
        // which is wrong — they're identifier *positions*, not type references.
        // The proper TypeRefs for member types come from the symbol-emission
        // path on the same nodes.
        "object_type" => {}
        // Tuple type: `[string, number]` — recurse into element types.
        "tuple_type" => {
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if !matches!(child.kind(), "[" | "]" | ",") {
                        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                    }
                }
            }
        }
        // Catch-all: walk all children recursively so any type node structure we
        // don't explicitly handle (e.g. new type constructs, nested wrappers) still
        // yields all type_identifier leaves.
        _ => {
            extract_type_refs_recursive(&type_node, src, source_symbol_index, refs);
        }
    }
}

/// Walk any AST subtree and emit a TypeRef for every `type_identifier` leaf,
/// skipping TypeScript primitives.
///
/// This is the escape hatch used by `extract_type_ref_from_annotation`'s catch-all
/// arm and can be called directly for contexts where the exact type node structure
/// is unknown (e.g. `as_expression` type arguments, `satisfies_expression` types).
pub(super) fn extract_type_refs_recursive(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        // Leaf: emit TypeRef if not a primitive.
        "type_identifier" | "identifier" => {
            let name = node_text(*node, src);
            if !name.is_empty() && !is_ts_primitive(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        // Skip inert tokens and binding-only nodes — including the
        // `string` / `number` / boolean nodes that can appear directly
        // (without a `literal_type` wrapper) when used as type members.
        "infer_type" | "this_type" | "literal_type"
        | "string" | "number" | "true" | "false" | "null" | "undefined"
        | "string_fragment" | "regex" => {}
        // Skip punctuation keywords that appear as unnamed children.
        "extends" | "keyof" | "readonly" | "typeof" | "infer" | "is"
        | "?" | ":" | "|" | "&" | "[" | "]" | "(" | ")" | "{" | "}" | "," | "=>" => {}
        // Object type literal — members are emitted as Property/Method symbols
        // by `extract_node` (driven by `recurse_for_object_types`). Walking
        // children here would emit property NAMES (like `action` in
        // `{ action: string }`) as TypeRefs because property_signature
        // contains an identifier leaf for the name.
        "object_type" => {}
        // Member nodes inside an object_type — same reasoning. extract_node
        // handles these as symbols; the deep-walk fallback shouldn't visit
        // them. Without this, the property/parameter NAME identifier leaks
        // out as a spurious TypeRef.
        "property_signature" | "method_signature" | "call_signature"
        | "construct_signature" | "index_signature"
        | "abstract_method_signature" => {}
        // Function-type parameter list members. Walking these would emit
        // parameter NAMES (`req`, `res`, `args`) as TypeRefs. The proper
        // parameter TYPES are extracted via `extract_param_and_return_types`.
        "required_parameter" | "optional_parameter" | "rest_parameter" => {}
        // For all structural type nodes, recurse into children.
        // extract_type_ref_from_annotation handles specific nodes with named field
        // lookups for precision; this helper is the deep-walk fallback that ensures
        // nothing is silently skipped.
        _ => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    // Delegate back to the main handler for recognised type constructs
                    // (union_type, intersection_type, generic_type, etc.) so that their
                    // named-field extraction logic runs; fall through to recursive walk
                    // for anything else.
                    match child.kind() {
                        "type_identifier" | "identifier" => {
                            let name = node_text(child, src);
                            if !name.is_empty() && !is_ts_primitive(&name) {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: child.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                        "infer_type" | "this_type" | "literal_type" => {}
                        // See top-level match above — these contain identifier
                        // leaves that aren't type references (property names,
                        // parameter names) and should be processed by
                        // extract_node, not the type-ref deep-walk.
                        "object_type"
                        | "property_signature" | "method_signature" | "call_signature"
                        | "construct_signature" | "index_signature"
                        | "abstract_method_signature"
                        | "required_parameter" | "optional_parameter" | "rest_parameter" => {}
                        _ => {
                            extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
                        }
                    }
                }
            }
        }
    }
}

/// Return true if `name` is a TypeScript primitive type keyword that should not
/// be emitted as a TypeRef edge.
#[inline]
fn is_ts_primitive(name: &str) -> bool {
    matches!(
        name,
        "string" | "number" | "boolean" | "void" | "any" | "unknown" | "never"
            | "undefined" | "null" | "object" | "symbol" | "bigint"
    )
}

/// Extract TypeRef edges for function/method parameter types and return type.
///
/// For `findAll(id: string, filter: FilterDto): Promise<Album[]>`, emits:
/// - TypeRef from findAll → FilterDto (parameter type)
/// - TypeRef from findAll → Promise (return type)
///
/// Skips primitive types (string, number, boolean, void, any, etc.) since they
/// don't reference user-defined symbols.
pub(super) fn extract_param_and_return_types(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Parameter types.
    if let Some(params) = node.child_by_field_name("parameters") {
        for i in 0..params.child_count() {
            if let Some(param) = params.child(i) {
                if param.kind() == "required_parameter" || param.kind() == "optional_parameter" {
                    if let Some(type_ann) = param.child_by_field_name("type") {
                        extract_type_ref_from_annotation(&type_ann, src, source_symbol_index, refs);
                    }
                }
            }
        }
    }

    // Return type.
    if let Some(ret_type) = node.child_by_field_name("return_type") {
        extract_type_ref_from_annotation(&ret_type, src, source_symbol_index, refs);
    }
}

/// Extract typed function/method parameters as Property symbols scoped to the method.
///
/// For `function getUser(repo: UserRepository)`, creates:
///   Symbol: `getUser.repo` (kind=Property, scope_path=Some("getUser"))
///   TypeRef: `getUser.repo → UserRepository`
///
/// This enables chain resolution inside the function body:
/// `repo.findOne()` resolves because `getUser.repo` is in `field_type` as `UserRepository`.
pub(super) fn extract_typed_params_as_symbols(
    func_node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;
    use crate::types::SymbolKind;

    let params = match func_node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return,
    };

    // The method scope — parameters should be qualified under the method name.
    // find_scope_at at the param position should give us the method scope
    // (since method_definition is now in TS_SCOPE_KINDS).
    let method_scope = if func_node.start_byte() > 0 {
        // Use the byte inside the parameters to find the method scope.
        scope_tree::find_scope_at(scope_tree, params.start_byte())
    } else {
        None
    };

    for i in 0..params.child_count() {
        let Some(param) = params.child(i) else { continue };
        if param.kind() != "required_parameter" && param.kind() != "optional_parameter" {
            continue;
        }

        // Skip constructor parameter properties (handled by extract_constructor_params).
        let has_modifier = (0..param.child_count()).any(|j| {
            param
                .child(j)
                .map(|c| c.kind() == "accessibility_modifier" || c.kind() == "readonly")
                .unwrap_or(false)
        });
        if has_modifier {
            continue;
        }

        // Get the parameter name.
        let name_node = match param
            .child_by_field_name("pattern")
            .or_else(|| param.child_by_field_name("name"))
        {
            Some(n) if n.kind() == "identifier" => n,
            _ => continue,
        };

        // Must have a type annotation — skip untyped parameters.
        let type_ann = match param.child_by_field_name("type") {
            Some(t) => t,
            None => continue,
        };

        let name = node_text(name_node, src);
        let qualified_name = scope_tree::qualify(&name, method_scope);
        let scope_path = scope_tree::scope_path(method_scope);

        let idx = symbols.len();
        symbols.push(crate::types::ExtractedSymbol {
            name,
            qualified_name,
            kind: SymbolKind::Property,
            visibility: None,
            start_line: param.start_position().row as u32,
            end_line: param.end_position().row as u32,
            start_col: param.start_position().column as u32,
            end_col: param.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Emit TypeRef from the parameter symbol to its type.
        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
    }
}
