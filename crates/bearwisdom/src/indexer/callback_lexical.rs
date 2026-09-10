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
    /// `None` permits an enclosing capture. PHP anonymous functions instead
    /// retain only exact, by-value `use ($name)` spans; an empty list fences
    /// every outer name. PHP arrows capture automatically and use `None`.
    outer_captures: Option<Vec<SourceSpan>>,
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
    outer_captures: Option<Vec<String>>,
}

pub(crate) fn supports(prefix: &str) -> bool {
    matches!(
        prefix,
        "scala" | "java" | "csharp" | "kotlin" | "swift" | "dart" | "go" | "python" | "php"
    )
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
            let Some(source_name) = text(source, parameter) else {
                continue;
            };
            let Some(name) = callback_binding_name(prefix, source_name) else {
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
                text(source, barrier.name)
                    .and_then(|name| callback_binding_name(prefix, name))
                    .map(|name| (name.to_owned(), barrier.range))
            })
            .collect();
        scopes.push((callback.body, scope));
        bound.push(BoundCallback {
            body: callback.body,
            parameters,
            barriers,
            boundaries: callback.boundaries,
            outer_captures: callback.outer_captures.map(|captures| {
                captures
                    .into_iter()
                    .filter_map(|capture| text(source, capture))
                    .filter_map(|name| callback_binding_name(prefix, name))
                    .map(str::to_owned)
                    .collect()
            }),
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
        // Visit every containing callback from inner to outer. A callback
        // without a matching declaration may capture its parent parameter;
        // an unsupported matching declaration still fences that exact name.
        // PHP anonymous functions permit only exact by-value `use (...)`
        // names to capture outward; a zero-parameter PHP arrow remains
        // transparent because PHP captures it automatically.
        let mut containing: Vec<_> = bound
            .iter()
            .filter(|callback| contains(callback.body, byte))
            .collect();
        containing.sort_by_key(|callback| callback.body.end - callback.body.start);
        for callback in containing {
            if callback
                .boundaries
                .iter()
                .any(|boundary| contains(*boundary, byte))
                || callback
                    .barriers
                    .iter()
                    .any(|(barrier, range)| barrier == name && contains(*range, byte))
            {
                break;
            }
            if let Some((_, binding)) = callback
                .parameters
                .iter()
                .find(|(parameter, _)| parameter == name)
            {
                graph.references.insert(byte, *binding);
                break;
            }
            if callback
                .outer_captures
                .as_ref()
                .is_some_and(|captures| !captures.iter().any(|capture| capture == name))
            {
                break;
            }
        }
    }
    Some(graph)
}

/// PHP declaration tokens retain their `$` source spelling, while the PHP
/// call extractor stores the matching chain root without it. Keep the exact
/// span in `declarations`, but compare the shared bare variable name. Dart
/// and Go deliberately leave `_` callback slots as signature holes.
fn callback_binding_name<'a>(prefix: &str, source_name: &'a str) -> Option<&'a str> {
    let name = if prefix == "php" {
        source_name.strip_prefix('$').unwrap_or(source_name)
    } else {
        source_name
    };
    if name.is_empty() || matches!(prefix, "dart" | "go") && name == "_" {
        None
    } else {
        Some(name)
    }
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
        "kotlin" | "swift" => kind == "lambda_literal",
        "dart" => kind == "function_expression",
        "go" => kind == "func_literal",
        "python" => kind == "lambda",
        "php" => matches!(kind, "arrow_function" | "anonymous_function"),
        _ => false,
    }
}

fn callback(node: Node, prefix: &str) -> Option<Callback> {
    // PHP error recovery may wrap an unsupported parameter form while still
    // retaining a real arrow/anonymous body. Keep that syntactic callback as
    // a conservative fence instead of letting its reads leak to an outer
    // callback identity.
    if node.has_error() && prefix != "php" {
        return None;
    }
    match prefix {
        "kotlin" => kotlin_callback(node),
        "swift" => swift_callback(node),
        "dart" => dart_callback(node),
        "go" => go_callback(node),
        "python" => python_callback(node),
        "php" => php_callback(node),
        _ => callback_with_fields(node, prefix),
    }
}

fn callback_with_fields(node: Node, prefix: &str) -> Option<Callback> {
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
        outer_captures: None,
    })
}

/// Kotlin has no `body` field on `lambda_literal`: its direct children are the
/// optional `lambda_parameters` followed by statements. The absence of the
/// former means the implicit `it` binding, which has no declaration span and
/// therefore cannot participate in this identity graph.
fn kotlin_callback(node: Node) -> Option<Callback> {
    let parameters = named_children(node)
        .find(|child| child.kind() == "lambda_parameters")
        .map(kotlin_parameter_spans)?;
    callback_from_explicit_parameters(node, parameters, "kotlin")
}

fn kotlin_parameter_spans(parameters: Node) -> Vec<SourceSpan> {
    named_children(parameters)
        .flat_map(|parameter| match parameter.kind() {
            "variable_declaration" => declaration_name(parameter).into_iter().collect(),
            "multi_variable_declaration" => named_children(parameter)
                .filter(|inner| inner.kind() == "variable_declaration")
                .filter_map(declaration_name)
                .collect(),
            _ => Vec::new(),
        })
        .map(span)
        .collect()
}

/// Swift's named closure parameters are declaration tokens under
/// `lambda_literal.type → lambda_function_type_parameters`. Shorthand `$0` /
/// `$1` parameters have no declaration nodes and are intentionally omitted.
fn swift_callback(node: Node) -> Option<Callback> {
    let ty = node.child_by_field_name("type")?;
    let parameters =
        named_children(ty).find(|child| child.kind() == "lambda_function_type_parameters")?;
    let spans = named_children(parameters)
        .filter(|parameter| parameter.kind() == "lambda_parameter")
        .filter_map(|parameter| {
            parameter
                .child_by_field_name("name")
                .filter(|name| name.kind() == "simple_identifier")
        })
        .map(span)
        .collect();
    callback_from_explicit_parameters(node, spans, "swift")
}

/// Dart function expressions keep their source declaration identifiers in the
/// `formal_parameter` nodes under the `parameters` field.  Do not fall back to
/// arbitrary descendants: a type identifier or nested function parameter is
/// not evidence for this callback binding.
fn dart_callback(node: Node) -> Option<Callback> {
    // The field is repeated when a closure has type parameters; select the
    // actual formal list rather than assuming its field order.
    let parameters = named_children(node).find(|child| child.kind() == "formal_parameter_list")?;
    let spans = named_children(parameters)
        .filter(|parameter| parameter.kind() == "formal_parameter")
        .filter_map(|parameter| {
            named_children(parameter).find(|child| child.kind() == "identifier")
        })
        .map(span)
        .collect();
    let body = node.child_by_field_name("body")?;
    callback_from_explicit_parameters(body, spans, "dart")
}

/// Go's `parameter_declaration.name` field is repeated for grouped names
/// (`func(a, b T)`). The grammar exposes every declaration name as a direct
/// identifier child; the type is a distinct node kind, so retain precisely
/// those direct declaration tokens.
fn go_callback(node: Node) -> Option<Callback> {
    let parameters = node.child_by_field_name("parameters")?;
    let spans = named_children(parameters)
        .filter(|parameter| parameter.kind() == "parameter_declaration")
        .flat_map(named_children)
        .filter(|name| name.kind() == "identifier")
        .map(span)
        .collect();
    let body = node.child_by_field_name("body")?;
    callback_from_explicit_parameters(body, spans, "go")
}

/// Python lambda parameters have a direct identifier form only. Defaults,
/// typed/default wrappers, splats, separators, and destructuring are emitted
/// as callback-signature holes, so this graph must not manufacture a source
/// declaration for any of them. `_` is an ordinary Python identifier and is
/// therefore retained.
fn python_callback(node: Node) -> Option<Callback> {
    let parameters = node.child_by_field_name("parameters")?;
    let spans = named_children(parameters)
        .filter(|parameter| parameter.kind() == "identifier")
        .map(span)
        .collect();
    let body = node.child_by_field_name("body")?;
    callback_from_explicit_parameters(body, spans, "python")
}

/// PHP callback parameter names are `variable_name` tokens (for example
/// `$item`) nested in `simple_parameter` nodes. Variadics are deliberate
/// signature holes in the fixed-arity callback contract, so they cannot gain
/// identity here. The declaration span stays exact; comparison normalization
/// happens only when binding to the extractor's bare chain root (`item`).
fn php_callback(node: Node) -> Option<Callback> {
    let parameters = node.child_by_field_name("parameters")?;
    let body = node.child_by_field_name("body")?;
    let body_span = span(body);
    let mut spans = Vec::new();
    let mut barriers = collect_barriers(body, "php", body_span);
    for parameter in named_children(parameters) {
        if parameter.kind() == "simple_parameter" {
            if let Some(name) = parameter
                .child_by_field_name("name")
                .filter(|name| name.kind() == "variable_name")
            {
                spans.push(span(name));
            }
        } else {
            // Variadic and future unsupported parameter nodes have a real
            // local name even though CallArg::LambdaAt carries a positional
            // hole. Fence that exact name so it cannot fall through to an
            // enclosing callback parameter.
            for name in php_parameter_variable_names(parameter) {
                barriers.push(Barrier {
                    name: span(name),
                    range: body_span,
                });
            }
        }
    }
    Some(Callback {
        body: body_span,
        parameters: spans,
        barriers,
        boundaries: collect_boundaries(body, "php"),
        outer_captures: (node.kind() == "anonymous_function")
            .then(|| php_anonymous_capture_names(node)),
    })
}

/// An anonymous PHP closure captures outer locals only when its `use` clause
/// names them by value. The grammar represents `&$item` as a `by_ref` child;
/// leave it out because mutation identity is not modeled by this graph.
fn php_anonymous_capture_names(node: Node) -> Vec<SourceSpan> {
    named_children(node)
        .find(|child| child.kind() == "anonymous_function_use_clause")
        .map(|clause| {
            named_children(clause)
                .filter(|capture| capture.kind() == "variable_name")
                .map(span)
                .collect()
        })
        .unwrap_or_default()
}

fn php_parameter_variable_names(node: Node) -> Vec<Node> {
    if node.kind() == "variable_name" {
        return vec![node];
    }
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        names.extend(php_parameter_variable_names(child));
    }
    names
}

fn callback_from_explicit_parameters(
    body: Node,
    parameters: Vec<SourceSpan>,
    prefix: &str,
) -> Option<Callback> {
    if parameters.is_empty() {
        return None;
    }
    let body_span = span(body);
    Some(Callback {
        body: body_span,
        parameters,
        barriers: collect_barriers(body, prefix, body_span),
        boundaries: collect_boundaries(body, prefix),
        outer_captures: None,
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
        "kotlin" => matches!(
            kind,
            "function_declaration"
                | "class_declaration"
                | "object_declaration"
                | "interface_declaration"
                | "enum_class_body"
        ),
        "swift" => matches!(
            kind,
            "function_declaration"
                | "init_declaration"
                | "deinit_declaration"
                | "class_declaration"
                | "struct_declaration"
                | "protocol_declaration"
                | "enum_declaration"
        ),
        "dart" => matches!(
            kind,
            "local_function_declaration"
                | "function_declaration"
                | "class_declaration"
                | "enum_declaration"
                | "extension_declaration"
                | "extension_type_declaration"
                | "mixin_declaration"
        ),
        "go" => matches!(
            kind,
            "function_declaration"
                | "method_declaration"
                | "type_declaration"
                | "struct_type"
                | "interface_type"
        ),
        "python" => matches!(kind, "function_definition" | "class_definition"),
        "php" => matches!(
            kind,
            "function_definition"
                | "class_declaration"
                | "interface_declaration"
                | "trait_declaration"
                | "enum_declaration"
                | "anonymous_class"
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
    // A PHP arrow has its own function scope, and an anonymous function has
    // its own body plus explicit-capture rules. Its writes must not extend a
    // barrier into the enclosing callback; the nested callback gets its own
    // barrier pass when `php_callback` constructs it.
    if node.start_byte() != callback_body.start as usize
        && prefix == "php"
        && callback_kind(prefix, node.kind())
    {
        return;
    }
    match (prefix, node.kind()) {
        ("scala", "val_definition" | "var_definition") => {
            if let Some(name) = node
                .child_by_field_name("pattern")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
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
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("kotlin", "property_declaration")
        | ("swift", "property_declaration" | "variable_declaration") => {
            if let Some(name) = declaration_name(node) {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("kotlin", "assignment") => {
            if let Some(name) = node
                .child_by_field_name("left")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("swift", "assignment") => {
            if let Some(name) = swift_assignment_name(node) {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("dart", "assignment_expression") => {
            if let Some(name) = dart_assignment_name(node) {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("python", "assignment" | "augmented_assignment") => {
            if let Some(name) = node
                .child_by_field_name("left")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("python", "named_expression") => {
            if let Some(name) = node
                .child_by_field_name("name")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("python", "for_in_clause") => {
            if let Some(scope) = python_comprehension_scope(node) {
                if let Some(left) = node.child_by_field_name("left") {
                    for name in python_pattern_identifiers(left) {
                        barriers.push(Barrier {
                            name: span(name),
                            range: span(scope),
                        });
                    }
                }
            }
        }
        ("php", "assignment_expression" | "augmented_assignment_expression") => {
            if let Some(name) = node
                .child_by_field_name("left")
                .filter(|name| name.kind() == "variable_name")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("php", "foreach_statement") => {
            if let Some(body) = node.child_by_field_name("body") {
                for name in php_foreach_target_names(node, body) {
                    barriers.push(Barrier {
                        name: span(name),
                        range: barrier_range(node, callback_body, prefix),
                    });
                }
            }
        }
        ("php", "catch_clause") => {
            if let (Some(name), Some(_body)) = (
                node.child_by_field_name("name")
                    .filter(|name| name.kind() == "variable_name"),
                node.child_by_field_name("body"),
            ) {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("dart", "initialized_variable_definition") => {
            if let Some(name) = node
                .child_by_field_name("name")
                .filter(|name| name.kind() == "identifier")
            {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
                });
            }
        }
        ("go", "short_var_declaration" | "assignment_statement") => {
            if let Some(left) = node.child_by_field_name("left") {
                for name in named_children(left).filter(|name| name.kind() == "identifier") {
                    barriers.push(Barrier {
                        name: span(name),
                        range: barrier_range(node, callback_body, prefix),
                    });
                }
            }
        }
        ("go", "var_spec") => {
            for name in named_children(node).filter(|name| name.kind() == "identifier") {
                barriers.push(Barrier {
                    name: span(name),
                    range: barrier_range(node, callback_body, prefix),
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
                    range: barrier_range(node, callback_body, prefix),
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

fn swift_assignment_name(node: Node) -> Option<Node> {
    let target = node.child_by_field_name("target")?;
    if target.kind() == "simple_identifier" {
        return Some(target);
    }
    named_children(target).find(|child| child.kind() == "simple_identifier")
}

fn dart_assignment_name(node: Node) -> Option<Node> {
    let left = node.child_by_field_name("left")?;
    if left.kind() == "identifier" {
        return Some(left);
    }
    // Dart wraps a bare assignment target in `assignable_expression`. Only a
    // sole direct identifier rebinds the callback parameter; selectors such
    // as `x.field = value` mutate a member and must not fence `x` itself.
    if left.kind() == "assignable_expression" {
        let mut children = named_children(left);
        let identifier = children
            .next()
            .filter(|child| child.kind() == "identifier")?;
        if children.next().is_none() {
            return Some(identifier);
        }
    }
    None
}

/// `foreach` has no field for its targets in the PHP grammar. The target is
/// everything after the `as` token and before the body; collect only direct
/// variable/pattern leaves in that region, never the iterated expression.
fn php_foreach_target_names<'tree>(node: Node<'tree>, body: Node<'tree>) -> Vec<Node<'tree>> {
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
            php_binding_pattern_names(child, &mut names);
        }
    }
    names
}

fn php_binding_pattern_names<'tree>(node: Node<'tree>, names: &mut Vec<Node<'tree>>) {
    if node.kind() == "variable_name" {
        names.push(node);
        return;
    }
    if !matches!(node.kind(), "by_ref" | "pair" | "list_literal") {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        php_binding_pattern_names(child, names);
    }
}

fn python_comprehension_scope(node: Node) -> Option<Node> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "list_comprehension"
                | "set_comprehension"
                | "dictionary_comprehension"
                | "generator_expression"
        ) {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Comprehension targets are binding patterns. Keeping only identifier leaves
/// covers bare and tuple/list targets without treating attributes or arbitrary
/// expressions as newly scoped names.
fn python_pattern_identifiers(node: Node) -> Vec<Node> {
    if node.kind() == "identifier" {
        return vec![node];
    }
    if !matches!(
        node.kind(),
        "tuple_pattern" | "list_pattern" | "pattern_list"
    ) {
        return Vec::new();
    }
    named_children(node)
        .flat_map(python_pattern_identifiers)
        .collect()
}

fn barrier_range(node: Node, callback_body: SourceSpan, prefix: &str) -> SourceSpan {
    let start = node.start_byte() as u32;
    // PHP locals, foreach targets, and catch variables are function-scoped.
    // A write inside a nested compound statement therefore fences the same
    // name through the rest of the callback function, not only that block.
    if prefix == "php" {
        return SourceSpan {
            start,
            end: callback_body.end,
        };
    }
    let mut current = node.parent();
    while let Some(parent) = current {
        if lexical_scope_kind(prefix, parent.kind()) {
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

fn lexical_scope_kind(prefix: &str, kind: &str) -> bool {
    kind == "block" || matches!(prefix, "swift") && kind == "statements"
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

/// The declaration identifier nested directly in a variable declaration or a
/// wrapper such as Kotlin's `property_declaration` / Swift's
/// `property_declaration`. Do not walk arbitrary descendants: a type or
/// initializer identifier is not a declaration.
fn declaration_name(node: Node) -> Option<Node> {
    if node.kind() == "property_declaration" {
        if let Some(pattern) = node.child_by_field_name("name") {
            if matches!(pattern.kind(), "simple_identifier" | "identifier") {
                return Some(pattern);
            }
            if let Some(name) = pattern.child_by_field_name("bound_identifier").or_else(|| {
                named_children(pattern)
                    .find(|child| matches!(child.kind(), "simple_identifier" | "identifier"))
            }) {
                return Some(name);
            }
        }
    }
    if node.kind() == "variable_declaration" {
        return named_children(node)
            .find(|child| matches!(child.kind(), "simple_identifier" | "identifier"));
    }
    named_children(node)
        .find(|child| child.kind() == "variable_declaration")
        .and_then(declaration_name)
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
