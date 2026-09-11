//! PHP callback syntax and capture policy.
use crate::{
    indexer::callback_lexical::{
        CallbackBarrier, CallbackDescriptor, CallbackLexicalAdapter, CallbackParameter,
        OuterCapture,
    },
    types::SourceSpan,
};
use tree_sitter::Node;
pub(crate) static ADAPTER: CallbackLexicalAdapter = CallbackLexicalAdapter { describe };
fn sp(n: Node) -> SourceSpan {
    SourceSpan {
        start: n.start_byte() as u32,
        end: n.end_byte() as u32,
    }
}
fn kids(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn txt<'a>(s: &'a [u8], n: Node) -> Option<&'a str> {
    std::str::from_utf8(&s[n.start_byte()..n.end_byte()]).ok()
}
fn var(s: &[u8], n: Node) -> Option<String> {
    let source_name = txt(s, n)?;
    let name = source_name.strip_prefix('$').unwrap_or(source_name);
    (!name.is_empty()).then(|| name.to_owned())
}
fn describe(n: Node, s: &[u8]) -> Option<CallbackDescriptor> {
    if !matches!(n.kind(), "arrow_function" | "anonymous_function") {
        return None;
    }
    let ps = n.child_by_field_name("parameters")?;
    let body = n.child_by_field_name("body")?;
    let mut parameters = vec![];
    let mut barriers = barriers(body, s);
    for p in kids(ps) {
        if p.kind() == "simple_parameter" {
            if let Some(x) = p
                .child_by_field_name("name")
                .filter(|x| x.kind() == "variable_name")
            {
                if let Some(name) = var(s, x) {
                    let annotation = p
                        .child_by_field_name("type")
                        .and_then(|t| txt(s, t))
                        .map(str::to_owned);
                    parameters.push(CallbackParameter {
                        declaration: sp(x),
                        name,
                        annotation,
                    });
                }
            }
        } else {
            for x in vars(p) {
                if let Some(name) = var(s, x) {
                    barriers.push(CallbackBarrier {
                        name,
                        range: sp(body),
                    })
                }
            }
        }
    }
    let outer_capture = if n.kind() == "anonymous_function" {
        OuterCapture::Explicit(
            kids(n)
                .into_iter()
                .find(|x| x.kind() == "anonymous_function_use_clause")
                .into_iter()
                .flat_map(kids)
                .filter(|x| x.kind() == "variable_name")
                .filter_map(|x| var(s, x))
                .collect(),
        )
    } else {
        OuterCapture::Transparent
    };
    Some(CallbackDescriptor {
        body: sp(body),
        parameters,
        barriers,
        boundaries: boundaries(body).into_iter().map(sp).collect(),
        outer_capture,
    })
}
fn vars(n: Node) -> Vec<Node> {
    if n.kind() == "variable_name" {
        vec![n]
    } else {
        kids(n).into_iter().flat_map(vars).collect()
    }
}
fn boundaries(n: Node) -> Vec<Node> {
    fn v<'tree>(n: Node<'tree>, root: Node<'tree>, o: &mut Vec<Node<'tree>>) {
        if n != root
            && matches!(
                n.kind(),
                "function_definition"
                    | "class_declaration"
                    | "interface_declaration"
                    | "trait_declaration"
                    | "enum_declaration"
                    | "anonymous_class"
            )
        {
            o.push(n);
            return;
        }
        for c in kids(n) {
            v(c, root, o)
        }
    }
    let mut o = vec![];
    v(n, n, &mut o);
    o
}
fn barriers(body: Node, s: &[u8]) -> Vec<CallbackBarrier> {
    fn v(n: Node, root: Node, s: &[u8], o: &mut Vec<CallbackBarrier>) {
        if n != root && matches!(n.kind(), "arrow_function" | "anonymous_function") {
            return;
        }
        let mut add = |x: Node| {
            if let Some(name) = var(s, x) {
                o.push(CallbackBarrier {
                    name,
                    range: SourceSpan {
                        start: n.start_byte() as u32,
                        end: root.end_byte() as u32,
                    },
                })
            }
        };
        match n.kind() {
            "assignment_expression" | "augmented_assignment_expression" => {
                if let Some(x) = n
                    .child_by_field_name("left")
                    .filter(|x| x.kind() == "variable_name")
                {
                    add(x)
                }
            }
            "catch_clause" => {
                if n.child_by_field_name("body").is_some() {
                    if let Some(x) = n
                        .child_by_field_name("name")
                        .filter(|x| x.kind() == "variable_name")
                    {
                        add(x)
                    }
                }
            }
            "foreach_statement" => {
                if let Some(foreach_body) = n.child_by_field_name("body") {
                    for x in foreach_target_names(n, foreach_body) {
                        add(x)
                    }
                }
            }
            _ => {}
        }
        for c in kids(n) {
            v(c, root, s, o)
        }
    }
    let mut o = vec![];
    v(body, body, s, &mut o);
    o
}

/// `foreach` has no field for its targets. Inspect only the source region
/// after `as` and before the body so variables in the iterable or body do not
/// become callback-local barriers.
fn foreach_target_names<'tree>(node: Node<'tree>, body: Node<'tree>) -> Vec<Node<'tree>> {
    let mut after_as = false;
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child == body {
            break;
        }
        if child.kind() == "as" {
            after_as = true;
            continue;
        }
        if after_as {
            binding_pattern_names(child, &mut names);
        }
    }
    names
}

fn binding_pattern_names<'tree>(node: Node<'tree>, names: &mut Vec<Node<'tree>>) {
    if node.kind() == "variable_name" {
        names.push(node);
        return;
    }
    if !matches!(node.kind(), "by_ref" | "pair" | "list_literal") {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        binding_pattern_names(child, names);
    }
}

#[cfg(test)]
#[path = "callback_lexical_tests.rs"]
mod tests;
