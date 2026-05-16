use super::helpers::node_text;
use crate::types::AliasTarget;
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
        // `type Foo<T> = { [K in keyof T]: U }` — mapped type. Walk
        // the `mapped_type_clause` child to find the keyof source,
        // then read the value template from the mapped_type's
        // remaining type child. Both pieces are needed so PR 15's
        // expander can detect the transparent `T[K]` pattern.
        "mapped_type" => {
            let mut source = String::new();
            let mut value_template = String::new();
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                match child.kind() {
                    "mapped_type_clause" => {
                        // The clause's `type` field is the iteration
                        // source — either a `keyof_type` /
                        // `index_type_query` or a type expression to
                        // iterate over (`"a" | "b"`).
                        if let Some(type_node) = child.child_by_field_name("type") {
                            if matches!(
                                type_node.kind(),
                                "keyof_type" | "index_type_query"
                            ) {
                                for j in 0..type_node.child_count() {
                                    let Some(op) = type_node.child(j) else { continue };
                                    if op.kind() == "keyof" {
                                        continue;
                                    }
                                    let name = head_type_name(&op, src);
                                    if !name.is_empty() {
                                        source = name;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // Skip syntactic noise; everything else is the
                    // value template (the type after `:`).
                    "{" | "}" | ":" | "?" | "+" | "-" | "readonly" => {}
                    _ => {
                        if child.is_named() && value_template.is_empty() {
                            // Take the raw text — PR 15's expander
                            // checks for the `T[K]` pattern via a
                            // simple syntactic match, which is the
                            // dominant case for utility types
                            // (Partial / Required / Readonly).
                            value_template = node_text(child, src)
                                .trim()
                                .to_string();
                        }
                    }
                }
            }
            return AliasTarget::Mapped {
                source,
                value_template,
            };
        }
        // `type Foo<T> = T extends U ? X : Y` — conditional type.
        // Read the four sub-expressions in source order. tree-sitter
        // exposes them as positional named children of
        // `conditional_type` separated by `extends`/`?`/`:` tokens.
        // Branch selection is deferred (no subtype checker yet) but
        // the captured shape lets a future PR wire it without
        // re-touching extract.
        "conditional_type" => {
            let mut parts: Vec<String> = Vec::new();
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "extends" | "?" | ":" | "(" | ")") {
                    continue;
                }
                if !child.is_named() {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    parts.push(name);
                } else {
                    parts.push(node_text(child, src));
                }
            }
            if parts.len() >= 4 {
                return AliasTarget::Conditional {
                    check: parts[0].clone(),
                    extends: parts[1].clone(),
                    true_branch: parts[2].clone(),
                    false_branch: parts[3].clone(),
                };
            }
            return AliasTarget::Other;
        }
        // `type X = T[K]` — indexed access. Capture the object's
        // head type and the key as written. The key may be a literal
        // string ("foo"), a generic param (`K`), or another type
        // expression like `keyof T` — the chain walker decides what
        // to do with each shape at expansion time. Anything where the
        // object isn't reducible to a single head bails to `Other`.
        "indexed_access_type" => {
            let object_node = node.child_by_field_name("object");
            let index_node = node.child_by_field_name("index");
            let object_name = match object_node {
                Some(n) => head_type_name(&n, src),
                None => String::new(),
            };
            let key_text = match index_node {
                Some(n) => {
                    // For literal types ("foo"), strip quotes so the
                    // chain walker can look up `T.foo` directly.
                    // tree-sitter wraps literals in `literal_type` →
                    // `string` / `number` / etc.
                    let raw = node_text(n, src);
                    let trimmed = raw.trim();
                    let stripped = trimmed
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .or_else(|| trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                        .unwrap_or(trimmed);
                    stripped.to_string()
                }
                None => String::new(),
            };
            if object_name.is_empty() || key_text.is_empty() {
                return AliasTarget::Other;
            }
            return AliasTarget::IndexedAccess {
                object: object_name,
                key: key_text,
            };
        }
        // `type X = keyof T` — tree-sitter exposes this as either
        // `keyof_type` or `index_type_query` depending on grammar
        // version. The operand is the target type whose members will
        // be enumerated; the chain walker can't expand this to a head
        // (it's a string union) but downstream PRs (indexed access,
        // mapped types) consume the captured target name.
        "keyof_type" | "index_type_query" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "keyof") {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    return AliasTarget::Keyof(name);
                }
            }
            AliasTarget::Other
        }
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
