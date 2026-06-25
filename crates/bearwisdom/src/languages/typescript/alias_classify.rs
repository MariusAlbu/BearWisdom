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
                    let Some(arg) = type_args_node.child(i) else {
                        continue;
                    };
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
        // `[A, B]` / `[get: A, set: B]` — a tuple. Each element's head type is
        // recorded by position so an array-destructure `const [a, b] = x` selects
        // the right slot. A labeled element (`get: A`) parses as a parameter node;
        // a rest/optional element wraps its type. Labels are dropped.
        "tuple_type" => {
            let mut elements = Vec::new();
            let mut tc = node.walk();
            for child in node.children(&mut tc) {
                if matches!(child.kind(), "[" | "]" | ",") {
                    continue;
                }
                let head = tuple_element_head(&child, src);
                if !head.is_empty() {
                    elements.push(head);
                }
            }
            AliasTarget::Tuple(elements)
        }
        "union_type" => {
            let mut branches = Vec::new();
            let mut has_object_branch = false;
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if child.kind() == "|" {
                    continue;
                }
                if child.kind() == "object_type" {
                    has_object_branch = true;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    branches.push(name);
                }
            }
            // A union whose only branches are anonymous object types
            // (`{kind:"a"}|{kind:"b"}`) yields no nameable branch, but
            // `recurse_for_object_types` flattens those members under the alias.
            // Classify it as a structural Object so the flattened members
            // resolve, instead of an empty Union that resolves nothing. A
            // primitive/literal union (`string | number`) stays a Union — there
            // are no members to flatten.
            if branches.is_empty() && has_object_branch {
                AliasTarget::Object
            } else {
                AliasTarget::Union(branches)
            }
        }
        "intersection_type" => {
            let mut branches = Vec::new();
            // A mapped branch found among the anonymous (unnamed) children, kept
            // alongside any named branches so member lookup can try BOTH halves of
            // `Named & { [K in keyof T]: V }`.
            let mut mapped_fallback: Option<AliasTarget> = None;
            collect_intersection_branches(&node, src, &mut branches, &mut mapped_fallback);
            match (branches.is_empty(), mapped_fallback) {
                // No named branch, only a mapped one — surface the mapped source so
                // the chain walker follows through to the source type's members.
                (true, Some(mapped)) => mapped,
                // Named branch(es) AND a mapped branch — carry both so neither is
                // dropped: `lookup_member_on_intersection` climbs the named
                // branches and `mapped_source_type` follows the mapped source.
                (false, Some(AliasTarget::Mapped { source, value_template })) => {
                    AliasTarget::IntersectionMapped {
                        branches,
                        source,
                        value_template,
                    }
                }
                // Only named branches (or a non-mapped fallback) — plain intersection.
                (_, _) => AliasTarget::Intersection(branches),
            }
        }
        // A `{ [K in keyof T]: V }` mapped type parses as an `object_type`
        // wrapping a `mapped_type_clause` in this grammar — not a top-level
        // `mapped_type` node. A plain object (no clause) stays `Object`; its
        // named members are emitted by `recurse_for_object_types`.
        "object_type" => classify_mapped_object(&node, src).unwrap_or(AliasTarget::Object),
        "mapped_type" => classify_mapped_object(&node, src).unwrap_or(AliasTarget::Object),
        // `type Foo<T> = T extends U ? X : Y` — conditional type.
        // Read the four sub-expressions in source order. tree-sitter
        // exposes them as positional named children of
        // `conditional_type` separated by `extends`/`?`/`:` tokens.
        // The `extends` clause's raw node is kept alongside its head
        // name so an `infer` capture (`Array<infer U>`) can be recorded
        // as a `(var, slot)` binding the expander resolves; the four
        // head names drive `is_assignable_to` branch selection.
        "conditional_type" => {
            let mut parts: Vec<String> = Vec::new();
            let mut extends_node: Option<Node> = None;
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "extends" | "?" | ":" | "(" | ")") {
                    continue;
                }
                if !child.is_named() {
                    continue;
                }
                // The `extends` clause is the second named child.
                if parts.len() == 1 {
                    extends_node = Some(child);
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    parts.push(name);
                } else {
                    parts.push(node_text(child, src));
                }
            }
            if parts.len() >= 4 {
                let infer_binding = extends_node
                    .as_ref()
                    .and_then(|n| infer_binding_from_extends(n, src));
                return AliasTarget::Conditional {
                    check: parts[0].clone(),
                    extends: parts[1].clone(),
                    true_branch: parts[2].clone(),
                    false_branch: parts[3].clone(),
                    infer_binding,
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
                        .or_else(|| {
                            trimmed
                                .strip_prefix('\'')
                                .and_then(|s| s.strip_suffix('\''))
                        })
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
        // `type F<T> = (...) => T` — callable alias whose call result is a
        // single nominal or generic-param head. Capture that head as
        // `Application { root, args: [] }` so the generic alias-expansion
        // path can substitute the application's type arg for `root` and
        // resolve members on the call result (e.g. `Accessor<QueryClient>`
        // → `QueryClient`). Declines when the return type reduces to
        // multiple heads (union, intersection, etc.) — those have no
        // unique application root and must stay opaque.
        "function_type" => {
            let return_head = node
                .child_by_field_name("return_type")
                .map(|n| head_type_name(&n, src))
                .unwrap_or_default();
            if return_head.is_empty() {
                AliasTarget::Other
            } else {
                AliasTarget::Application {
                    root: return_head,
                    args: Vec::new(),
                }
            }
        }
        // Everything else — `keyof T`, mapped, conditional,
        // indexed-access, template-literal, tuples, type predicates,
        // infer, this, literals — is a non-application shape we don't
        // expand yet. Recorded as `Other` so callers don't fall back
        // to the field_type heuristic.
        _ => AliasTarget::Other,
    }
}

/// Collect the named-type branches of an intersection into `branches`, recursing
/// into nested `intersection_type` children. `A & B & C` parses left-
/// associatively as `(A & B) & C`, so the outer node's first child is itself an
/// intersection whose named branches (`A`, `B`) would be lost without recursion.
/// The first anonymous mapped branch is recorded in `mapped_fallback`.
fn collect_intersection_branches(
    node: &Node,
    src: &[u8],
    branches: &mut Vec<String>,
    mapped_fallback: &mut Option<AliasTarget>,
) {
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        if child.kind() == "&" {
            continue;
        }
        if child.kind() == "intersection_type" {
            collect_intersection_branches(&child, src, branches, mapped_fallback);
            continue;
        }
        let name = head_type_name(&child, src);
        if !name.is_empty() {
            branches.push(name);
        } else if matches!(child.kind(), "object_type" | "mapped_type")
            && mapped_fallback.is_none()
        {
            if let Some(AliasTarget::Mapped {
                source,
                value_template,
            }) = classify_mapped_object(&child, src)
            {
                if !source.is_empty() {
                    *mapped_fallback = Some(AliasTarget::Mapped {
                        source,
                        value_template,
                    });
                }
            }
        }
    }
}

/// Classify a `{ [K in keyof Src]: V }` mapped type from a node that wraps a
/// `mapped_type_clause` — either a top-level `mapped_type` or the `object_type`
/// the TS grammar wraps it in. `source` is the keyof target (the `Src`);
/// `value_template` is the index value as written. Returns `None` when the node
/// holds no mapped clause, so a plain object type stays `Object`.
///
/// The TS grammar wraps mapped types as:
///   `object_type → index_signature → mapped_type_clause`
/// The `mapped_type_clause` is never a direct child of `object_type`; it always
/// sits one level deeper inside an `index_signature`. This function descends
/// through `index_signature` to find the clause, and reads the value template
/// from the `index_signature`'s `type_annotation` child (the `: V` after `]`).
fn classify_mapped_object(node: &Node, src: &[u8]) -> Option<AliasTarget> {
    let mut has_clause = false;
    let mut source = String::new();
    let mut value_template = String::new();
    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        // The grammar nests mapped_type_clause under index_signature, not
        // directly under object_type. Descend one level when we see it.
        if child.kind() == "index_signature" {
            for j in 0..child.child_count() {
                let Some(sig_child) = child.child(j) else { continue };
                if sig_child.kind() == "mapped_type_clause" {
                    has_clause = true;
                    extract_mapped_source(&sig_child, src, &mut source);
                } else if sig_child.kind() == "type_annotation" && value_template.is_empty() {
                    // `type_annotation` holds the `:` plus the value type; skip
                    // the colon token and grab the first named type child.
                    for k in 0..sig_child.child_count() {
                        let Some(ann_child) = sig_child.child(k) else { continue };
                        if ann_child.kind() == ":" {
                            continue;
                        }
                        if ann_child.is_named() {
                            value_template = node_text(ann_child, src).trim().to_string();
                            break;
                        }
                    }
                }
            }
        } else if child.kind() == "mapped_type_clause" {
            // Direct child — top-level `mapped_type` node (grammar variant).
            has_clause = true;
            extract_mapped_source(&child, src, &mut source);
        } else if child.is_named()
            && value_template.is_empty()
            && !matches!(child.kind(), "{" | "}" | ":" | "?" | "+" | "-" | "readonly" | ";" | ",")
        {
            // Fallback for direct-child value template in top-level `mapped_type`.
            value_template = node_text(child, src).trim().to_string();
        }
    }
    has_clause.then_some(AliasTarget::Mapped {
        source,
        value_template,
    })
}

/// Fill `source` with the keyof operand from a `mapped_type_clause`.
///
/// Reads the clause's `type` field (a `keyof_type` / `index_type_query`) and
/// extracts the type identifier that follows `keyof`.
fn extract_mapped_source(clause: &Node, src: &[u8], source: &mut String) {
    if let Some(type_node) = clause.child_by_field_name("type") {
        if matches!(type_node.kind(), "keyof_type" | "index_type_query") {
            for j in 0..type_node.child_count() {
                let Some(op) = type_node.child(j) else { continue };
                if op.kind() == "keyof" {
                    continue;
                }
                let name = head_type_name(&op, src);
                if !name.is_empty() {
                    *source = name;
                    break;
                }
            }
        }
    }
}

/// Best-effort head name of a type expression. Returns the simple name
/// for `type_identifier` / `identifier` / `generic_type` (just the
/// `name` field, not its args), the dotted text for
/// `nested_type_identifier` / `member_expression`, the element-type
/// head for `array_type`, and an empty string for shapes whose head
/// can't be reduced to a single name (unions, intersections, mapped,
/// conditional, etc.).
/// The head type name of one tuple element node. A labeled element parses as a
/// `required_parameter`/`optional_parameter` whose `type` field is a
/// `type_annotation` (`: T`); a `rest_type`/`optional_type` wraps the type as a
/// child; an unlabeled element IS the type node.
fn tuple_element_head(child: &Node, src: &[u8]) -> String {
    match child.kind() {
        "required_parameter" | "optional_parameter" => child
            .child_by_field_name("type")
            .map(|ta| type_annotation_head(&ta, src))
            .unwrap_or_default(),
        "optional_type" | "rest_type" => {
            for i in 0..child.child_count() {
                if let Some(n) = child.child(i) {
                    if n.is_named() {
                        return head_type_name(&n, src);
                    }
                }
            }
            String::new()
        }
        _ => head_type_name(child, src),
    }
}

/// The head type name inside a `type_annotation` (`: T` → `T`'s head).
fn type_annotation_head(ta: &Node, src: &[u8]) -> String {
    for i in 0..ta.child_count() {
        if let Some(c) = ta.child(i) {
            if c.kind() != ":" {
                return head_type_name(&c, src);
            }
        }
    }
    String::new()
}

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
        // `typeof value` — the value's name, so `ReturnType<typeof v>` carries `v`
        // as its single argument for the ReturnType intrinsic to resolve.
        "type_query" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if child.kind() == "typeof" {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Record a single `infer` capture in a conditional's `extends` clause.
///
/// Recognizes `extends Head<infer Var>` (a `generic_type`) and returns
/// `Some((Var, slot))` where `slot` is the 0-based position of the
/// `infer_type` among the type arguments. Used so the expander can bind
/// `Var` to the checked type's `Apply` arg at `slot`.
///
/// Declines (returns `None`) when the extends clause is not a generic
/// application, carries no `infer`, or carries more than one `infer`
/// (multi-capture needs unification the expander does not perform).
fn infer_binding_from_extends(extends: &Node, src: &[u8]) -> Option<(String, usize)> {
    if extends.kind() != "generic_type" {
        return None;
    }
    let type_args = extends.child_by_field_name("type_arguments")?;
    let mut binding: Option<(String, usize)> = None;
    let mut slot = 0usize;
    for i in 0..type_args.child_count() {
        let Some(arg) = type_args.child(i) else {
            continue;
        };
        if matches!(arg.kind(), "<" | ">" | ",") {
            continue;
        }
        if arg.kind() == "infer_type" {
            if binding.is_some() {
                // More than one `infer` in the same clause — decline.
                return None;
            }
            let var = infer_var_name(&arg, src)?;
            binding = Some((var, slot));
        }
        slot += 1;
    }
    binding
}

/// Extract the variable name introduced by an `infer_type` node
/// (`infer U` → `"U"`). Reads the `name` field, falling back to the
/// first `type_identifier` / `identifier` child.
fn infer_var_name(infer_node: &Node, src: &[u8]) -> Option<String> {
    if let Some(name_node) = infer_node.child_by_field_name("name") {
        let name = node_text(name_node, src);
        if !name.is_empty() {
            return Some(name);
        }
    }
    for i in 0..infer_node.child_count() {
        let Some(child) = infer_node.child(i) else {
            continue;
        };
        if matches!(child.kind(), "type_identifier" | "identifier") {
            let name = node_text(child, src);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "alias_classify_tests.rs"]
mod tests;
