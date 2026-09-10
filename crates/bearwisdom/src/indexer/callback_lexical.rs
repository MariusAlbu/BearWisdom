//! Minimal source identity for callback parameters in languages without the
//! full TS/JS lexical graph. This deliberately captures neither locals nor
//! arbitrary identifier reads: only lambda declarations and extracted chain
//! roots in their bodies participate.
use crate::{
    indexer::lexical::{BindingId, LexicalBindings, ScopeId},
    types::{ExtractedRef, SourceSpan},
};
use tree_sitter::Node;

#[derive(Clone)]
struct Callback {
    body: SourceSpan,
    parameters: Vec<SourceSpan>,
    barriers: Vec<Barrier>,
    boundaries: Vec<SourceSpan>,
}

#[derive(Clone)]
struct Barrier {
    name: SourceSpan,
    range: SourceSpan,
}

struct BoundCallback {
    body: SourceSpan,
    parameters: Vec<(String, BindingId)>,
    barriers: Vec<(String, SourceSpan)>,
    boundaries: Vec<SourceSpan>,
}

pub(crate) fn supports(prefix: &str) -> bool {
    matches!(prefix, "scala" | "java" | "csharp")
}

/// Capture a graph whose only bindings are callback parameters. The returned
/// graph is intentionally separate from `FlowMeta::lexical`: opting into it
/// must not change the legacy local-inference or global-source semantics.
pub(crate) fn capture(
    root: Node,
    source: &[u8],
    prefix: &str,
    refs: &[ExtractedRef],
) -> Option<LexicalBindings> {
    if !supports(prefix) {
        return None;
    }
    let mut callbacks = Vec::new();
    collect_callbacks(root, prefix, &mut callbacks);
    if callbacks.is_empty() {
        return None;
    }

    // Parent callback bodies precede nested bodies. `LexicalBindings::scope_at`
    // depends on child scopes being ordered by their source starts.
    callbacks.sort_by_key(|callback| (callback.body.start, std::cmp::Reverse(callback.body.end)));
    let mut graph = LexicalBindings::default();
    let root_scope = graph.add_scope(None, root.start_byte() as u32, root.end_byte() as u32, true);
    let mut scopes: Vec<(SourceSpan, ScopeId)> = Vec::new();
    let mut bound = Vec::new();

    for callback in callbacks {
        let parent = scopes
            .iter()
            .filter(|(body, _)| contains(*body, callback.body.start))
            .min_by_key(|(body, _)| body.end - body.start)
            .map(|(_, scope)| *scope)
            .unwrap_or(root_scope);
        let scope = graph.add_scope(Some(parent), callback.body.start, callback.body.end, true);
        let mut parameters = Vec::new();
        for parameter in callback.parameters {
            let Some(name) = text(source, parameter) else {
                continue;
            };
            let name_id = graph.intern(name);
            let binding = graph.declare(scope, name_id, callback.body.start, None);
            graph.declarations.insert(parameter, binding);
            parameters.push((name.to_owned(), binding));
        }
        let barriers = callback
            .barriers
            .into_iter()
            .filter_map(|barrier| {
                text(source, barrier.name).map(|name| (name.to_owned(), barrier.range))
            })
            .collect();
        scopes.push((callback.body, scope));
        bound.push(BoundCallback {
            body: callback.body,
            parameters,
            barriers,
            boundaries: callback.boundaries,
        });
    }

    // `references` is an attested bridge for the later callback cache. The
    // extractor address is authoritative: Java stores a member selector here,
    // while its chain root is the receiver parameter.
    for reference in refs {
        let Some(name) = reference
            .chain
            .as_ref()
            .and_then(|chain| chain.segments.first())
            .map(|segment| segment.name.as_str())
        else {
            continue;
        };
        let byte = reference.byte_offset;
        if let Some(callback) = bound
            .iter()
            .filter(|callback| contains(callback.body, byte))
            // A nested callback that does not declare `x` must not hide an
            // outer captured `x`. Select among declarations, not bodies.
            .filter(|callback| callback.parameters.iter().any(|(parameter, _)| parameter == name))
            .min_by_key(|callback| callback.body.end - callback.body.start)
        {
            // A callback parameter can be captured through a nested callback,
            // but it is not visible through an ordinary callable or class
            // boundary. Those bodies have their own parameter/local identity
            // rules, which this intentionally callback-only graph does not
            // model.
            if callback
                .boundaries
                .iter()
                .any(|boundary| contains(*boundary, byte))
            {
                continue;
            }
            let shadowed = callback
                .barriers
                .iter()
                .any(|(barrier, range)| barrier == name && contains(*range, byte));
            if !shadowed {
                if let Some((_, binding)) = callback
                    .parameters
                    .iter()
                    .find(|(parameter, _)| parameter == name)
                {
                    graph.references.insert(byte, *binding);
                }
            }
        }
    }
    Some(graph)
}

fn collect_callbacks(node: Node, prefix: &str, callbacks: &mut Vec<Callback>) {
    if callback_kind(prefix, node.kind()) {
        if let Some(callback) = callback(node, prefix) {
            callbacks.push(callback);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_callbacks(child, prefix, callbacks);
    }
}

fn callback_kind(prefix: &str, kind: &str) -> bool {
    match prefix {
        "scala" | "java" => kind == "lambda_expression",
        "csharp" => matches!(kind, "lambda_expression" | "anonymous_method_expression"),
        _ => false,
    }
}

fn callback(node: Node, prefix: &str) -> Option<Callback> {
    if node.has_error() {
        return None;
    }
    let parameters_node = node.child_by_field_name("parameters").or_else(|| {
        let mut cursor = node.walk();
        let found = node
            .named_children(&mut cursor)
            .find(|child| matches!(child.kind(), "implicit_parameter" | "parameter_list"));
        found
    })?;
    let parameters = parameter_spans(parameters_node, prefix);
    let body = node.child_by_field_name("body").or_else(|| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|child| *child != parameters_node)
            .filter(|child| child.start_byte() >= parameters_node.end_byte())
            .last()
    })?;
    let body_span = span(body);
    Some(Callback {
        body: body_span,
        parameters,
        barriers: collect_barriers(body, prefix, body_span),
        boundaries: collect_boundaries(body, prefix),
    })
}

/// Ordinary nested callables and class bodies establish declarations that the
/// callback-only graph cannot identify. They fence an outer callback parameter
/// out of attribution. Nested callbacks are traversed without becoming a
/// boundary themselves: each gets its own callback scope and can either
/// declare its own parameter or capture the outer one, while an ordinary
/// boundary inside it must still fence the outer binding.
fn collect_boundaries(body: Node, prefix: &str) -> Vec<SourceSpan> {
    let mut boundaries = Vec::new();
    collect_boundaries_inner(body, prefix, body, &mut boundaries);
    boundaries
}

fn collect_boundaries_inner(
    node: Node,
    prefix: &str,
    callback_body: Node,
    boundaries: &mut Vec<SourceSpan>,
) {
    if node != callback_body && ordinary_boundary_kind(prefix, node.kind()) {
        boundaries.push(span(node));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_boundaries_inner(child, prefix, callback_body, boundaries);
    }
}

fn ordinary_boundary_kind(prefix: &str, kind: &str) -> bool {
    match prefix {
        "scala" => matches!(
            kind,
            "function_definition"
                | "function_declaration"
                | "class_definition"
                | "object_definition"
                | "trait_definition"
                | "enum_definition"
        ),
        "java" => matches!(
            kind,
            "method_declaration"
                | "constructor_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "annotation_type_declaration"
                | "class_body"
                | "anonymous_class_body"
        ),
        "csharp" => matches!(
            kind,
            "local_function_statement"
                | "method_declaration"
                | "constructor_declaration"
                | "destructor_declaration"
                | "class_declaration"
                | "struct_declaration"
                | "record_declaration"
                | "interface_declaration"
        ),
        _ => false,
    }
}

/// The callback graph does not model arbitrary locals. A declaration or write
/// that could shadow/rebind a callback parameter instead fences affected reads
/// out of contextual attribution. Nested bodies are scanned too because an
/// outer parameter can be captured through a nested callback.
fn collect_barriers(body: Node, prefix: &str, callback_body: SourceSpan) -> Vec<Barrier> {
    let mut barriers = Vec::new();
    collect_barriers_inner(body, prefix, callback_body, &mut barriers);
    barriers
}

fn collect_barriers_inner(
    node: Node,
    prefix: &str,
    callback_body: SourceSpan,
    barriers: &mut Vec<Barrier>,
) {
    match (prefix, node.kind()) {
        ("scala", "val_definition" | "var_definition") => {
            if let Some(name) = node
                .child_by_field_name("pattern")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body),
                });
            }
        }
        ("java", "variable_declarator") | ("csharp", "variable_declarator") => {
            if let Some(name) = node
                .child_by_field_name("name")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body),
                });
            }
        }
        (_, "assignment_expression") => {
            if let Some(name) = node
                .child_by_field_name("left")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body),
                });
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_barriers_inner(child, prefix, callback_body, barriers);
    }
}

fn barrier_range(node: Node, callback_body: SourceSpan) -> SourceSpan {
    let start = node.start_byte() as u32;
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "block" {
            return SourceSpan {
                start,
                end: parent.end_byte() as u32,
            };
        }
        let parent_span = span(parent);
        if parent_span == callback_body {
            break;
        }
        current = parent.parent();
    }
    SourceSpan {
        start,
        end: callback_body.end,
    }
}

fn parameter_spans(parameters: Node, prefix: &str) -> Vec<SourceSpan> {
    match prefix {
        "scala" => match parameters.kind() {
            "identifier" => vec![span(parameters)],
            "bindings" => named_children(parameters)
                .filter(|binding| binding.kind() == "binding")
                .filter_map(|binding| binding.child_by_field_name("name"))
                .filter(|name| name.kind() == "identifier")
                .map(span)
                .collect(),
            _ => Vec::new(),
        },
        "java" => match parameters.kind() {
            "identifier" => vec![span(parameters)],
            "inferred_parameters" => named_children(parameters)
                .filter(|parameter| parameter.kind() == "identifier")
                .map(span)
                .collect(),
            "formal_parameters" => named_children(parameters)
                .filter(|parameter| parameter.kind() == "formal_parameter")
                .filter_map(|parameter| parameter.child_by_field_name("name"))
                .filter(|name| name.kind() == "identifier")
                .map(span)
                .collect(),
            _ => Vec::new(),
        },
        "csharp" => match parameters.kind() {
            "implicit_parameter" => vec![span(parameters)],
            "parameter_list" => named_children(parameters)
                .filter(|parameter| parameter.kind() == "parameter")
                .filter_map(|parameter| parameter.child_by_field_name("name"))
                .filter(|name| name.kind() == "identifier")
                .map(span)
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn named_children(node: Node) -> impl Iterator<Item = Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .collect::<Vec<_>>()
        .into_iter()
}

fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

fn contains(range: SourceSpan, byte: u32) -> bool {
    range.start <= byte && byte < range.end
}

fn text<'a>(source: &'a [u8], span: SourceSpan) -> Option<&'a str> {
    std::str::from_utf8(source.get(span.start as usize..span.end as usize)?).ok()
}

#[cfg(test)]
#[path = "callback_lexical_tests.rs"]
mod tests;
