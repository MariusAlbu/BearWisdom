//! Strict RBI contract extraction.
//!
//! RBI is Ruby source, so its declarations are parsed with tree-sitter-ruby.
//! This accepts only the small `sig { params(...).returns(...) }` subset that
//! maps exactly to the resolver's structural function representation.

use std::collections::{HashMap, HashSet};

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

struct Scope {
    qualified_name: String,
    symbol_index: usize,
}

struct Method {
    name: String,
    qualified_name: String,
    scope_path: String,
    parent_index: usize,
    signature: String,
    start_line: u32,
    end_line: u32,
    start_col: u32,
    end_col: u32,
    byte_offset: u32,
}

/// Extract strict RBI contracts from an error-free Ruby CST.
pub(super) fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_ruby::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Ruby grammar");
    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult::new(Vec::new(), Vec::new(), true);
    };
    if tree.root_node().has_error() {
        return ExtractionResult::new(Vec::new(), Vec::new(), true);
    }

    let src = source.as_bytes();
    let mut symbols = Vec::new();
    let mut methods = Vec::new();
    let mut method_counts = HashMap::new();
    walk_scopes(
        tree.root_node(),
        src,
        None,
        "",
        &mut symbols,
        &mut methods,
        &mut method_counts,
    );

    for method in methods
        .into_iter()
        .filter(|method| method_counts[&method.qualified_name] == 1)
    {
        symbols.push(ExtractedSymbol {
            name: method.name,
            qualified_name: method.qualified_name,
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: method.start_line,
            end_line: method.end_line,
            start_col: method.start_col,
            end_col: method.end_col,
            byte_offset: method.byte_offset,
            signature: Some(method.signature),
            doc_comment: None,
            scope_path: Some(method.scope_path),
            parent_index: Some(method.parent_index),
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
    ExtractionResult::new(symbols, Vec::new(), false)
}

fn walk_scopes(
    container: Node,
    src: &[u8],
    scope: Option<&Scope>,
    prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    methods: &mut Vec<Method>,
    method_counts: &mut HashMap<String, usize>,
) {
    let children = named_children(container);
    for (index, child) in children.iter().copied().enumerate() {
        match child.kind() {
            "class" | "module" => {
                let Some(name_node) = child.child_by_field_name("name") else {
                    continue;
                };
                let name = text(name_node, src);
                if !is_constant_name(&name) {
                    continue;
                }
                let qualified_name = qualify_scope_name(prefix, &name);
                let symbol_index = symbols.len();
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name: qualified_name.clone(),
                    kind: if child.kind() == "class" {
                        SymbolKind::Class
                    } else {
                        SymbolKind::Interface
                    },
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    byte_offset: child.start_byte() as u32,
                    signature: source_first_line(child, src),
                    doc_comment: None,
                    scope_path: (!prefix.is_empty()).then(|| prefix.to_string()),
                    parent_index: scope.map(|parent| parent.symbol_index),
                    declared_type: None,
                    return_type: None,
                    param_types: Vec::new(),
                    generic_params: Vec::new(),
                });
                if let Some(body) = child.child_by_field_name("body") {
                    let nested_scope = Scope {
                        qualified_name,
                        symbol_index,
                    };
                    walk_scopes(
                        body,
                        src,
                        Some(&nested_scope),
                        &nested_scope.qualified_name,
                        symbols,
                        methods,
                        method_counts,
                    );
                }
            }
            "method" => {
                let Some(scope) = scope else {
                    continue;
                };
                let Some(name_node) = child.child_by_field_name("name") else {
                    continue;
                };
                let name = text(name_node, src);
                let qualified_name = super::helpers::qualify(&name, &scope.qualified_name);
                *method_counts.entry(qualified_name.clone()).or_default() += 1;
                let Some(previous) = index.checked_sub(1).and_then(|i| children.get(i)) else {
                    continue;
                };
                if index
                    .checked_sub(2)
                    .and_then(|i| children.get(i))
                    .is_some_and(|node| is_sig_call(*node, src))
                {
                    continue;
                }
                let Some(signature) = contract_signature(*previous, child, src) else {
                    continue;
                };
                methods.push(Method {
                    qualified_name,
                    name,
                    scope_path: scope.qualified_name.clone(),
                    parent_index: scope.symbol_index,
                    signature,
                    start_line: name_node.start_position().row as u32,
                    end_line: name_node.end_position().row as u32,
                    start_col: name_node.start_position().column as u32,
                    end_col: name_node.end_position().column as u32,
                    byte_offset: name_node.start_byte() as u32,
                });
            }
            _ => {}
        }
    }
}

fn contract_signature(sig: Node, method: Node, src: &[u8]) -> Option<String> {
    let parsed = sig_expression(sig, src)?;
    let name = text(method.child_by_field_name("name")?, src);
    match method_parameters(method, src)? {
        MethodParameters::FinalBlock {
            ordinary,
            block_name,
        } => block_contract_signature(&name, ordinary, block_name, parsed, src),
        MethodParameters::PositionalProc { callback_name } => {
            positional_proc_contract_signature(&name, callback_name, parsed, src)
        }
    }
}

/// The established attached-block form. Its leading `&` is retained in the
/// canonical parameter name so callers can distinguish the Ruby calling
/// convention after this source CST is no longer available.
fn block_contract_signature(
    name: &str,
    method_names: Vec<String>,
    block_name: String,
    parsed: Params<'_>,
    src: &[u8],
) -> Option<String> {
    if parsed.entries.len() != method_names.len() + 1 {
        return None;
    }

    let mut signature_params = Vec::with_capacity(parsed.entries.len());
    for (expected_name, (actual_name, type_node)) in method_names.iter().zip(&parsed.entries) {
        let ty = simple_type(*type_node, src)?;
        if expected_name != actual_name {
            return None;
        }
        signature_params.push(format!("{expected_name}: {ty}"));
    }
    let (declared_block_name, callback) = parsed.entries.last()?;
    if declared_block_name != &block_name {
        return None;
    }
    let callback = parse_proc(*callback, src)?;
    signature_params.push(format!(
        "&{block_name}: ({}) -> {}",
        callback.params.join(", "),
        callback.return_type
    ));
    Some(format!(
        "{name}({}): {}",
        signature_params.join(", "),
        parsed.return_type
    ))
}

/// The only ordinary Proc parameter admitted by this strict RBI slice. It is
/// deliberately mutually exclusive with `&block`: one required identifier,
/// one same-named Sorbet entry, and no other method inputs. Its `^` canonical
/// marker records that the callback is an ordinary positional argument while
/// retaining the source parameter name.
fn positional_proc_contract_signature(
    name: &str,
    callback_name: String,
    parsed: Params<'_>,
    src: &[u8],
) -> Option<String> {
    let [(declared_name, callback)] = parsed.entries.as_slice() else {
        return None;
    };
    if declared_name != &callback_name {
        return None;
    }
    let callback = parse_proc(*callback, src)?;
    Some(format!(
        "{name}(^{callback_name}: ({}) -> {}): {}",
        callback.params.join(", "),
        callback.return_type,
        parsed.return_type
    ))
}

struct Params<'tree> {
    entries: Vec<(String, Node<'tree>)>,
    return_type: String,
}

struct ProcType {
    params: Vec<String>,
    return_type: String,
}

fn sig_expression<'tree>(sig: Node<'tree>, src: &[u8]) -> Option<Params<'tree>> {
    if !is_sig_call(sig, src) || sig.child_by_field_name("arguments").is_some() {
        return None;
    }
    let block = sig.child_by_field_name("block")?;
    if block.kind() != "block" || block.child_by_field_name("parameters").is_some() {
        return None;
    }
    let body = block.child_by_field_name("body")?;
    let expressions = named_children(body);
    if expressions.len() != 1 {
        return None;
    }
    let (return_type, receiver) = terminal_type(expressions[0], src)?;
    let params_call = exact_call(receiver, "params", src)?;
    let arguments = params_call.child_by_field_name("arguments")?;
    let entries = parse_named_arguments(arguments, src)?;
    Some(Params {
        entries,
        return_type,
    })
}

fn terminal_type<'tree>(node: Node<'tree>, src: &[u8]) -> Option<(String, Node<'tree>)> {
    if node.kind() != "call" || node.child_by_field_name("block").is_some() || !dot_call(node, src)
    {
        return None;
    }
    let receiver = node.child_by_field_name("receiver")?;
    match text(node.child_by_field_name("method")?, src).as_str() {
        "void" if node.child_by_field_name("arguments").is_none() => {
            Some(("void".into(), receiver))
        }
        "returns" => {
            let argument = one_argument(node.child_by_field_name("arguments")?)?;
            let ty = simple_type(argument, src)?;
            Some((ty, receiver))
        }
        _ => None,
    }
}

fn parse_proc(node: Node, src: &[u8]) -> Option<ProcType> {
    let (return_type, receiver) = terminal_type(node, src)?;
    let (params, base) = if is_dot_named_call(receiver, "params", src) {
        let arguments = receiver.child_by_field_name("arguments")?;
        let entries = parse_named_arguments(arguments, src)?;
        if entries.is_empty() {
            return None;
        }
        let mut parsed = Vec::with_capacity(entries.len());
        for (_, value) in entries {
            parsed.push(simple_type(value, src)?);
        }
        (parsed, receiver.child_by_field_name("receiver")?)
    } else {
        (Vec::new(), receiver)
    };
    if !is_t_proc(base, src) {
        return None;
    }
    Some(ProcType {
        params,
        return_type,
    })
}

enum MethodParameters {
    FinalBlock {
        ordinary: Vec<String>,
        block_name: String,
    },
    PositionalProc {
        callback_name: String,
    },
}

fn method_parameters(method: Node, src: &[u8]) -> Option<MethodParameters> {
    let parameters = method.child_by_field_name("parameters")?;
    let parameters = named_children(parameters);
    if let [parameter] = parameters.as_slice() {
        if parameter.kind() == "identifier" {
            let callback_name = text(*parameter, src);
            return is_identifier(&callback_name)
                .then_some(MethodParameters::PositionalProc { callback_name });
        }
    }
    let (last, ordinary) = parameters.split_last()?;
    if last.kind() != "block_parameter" {
        return None;
    }
    let block_name = text(last.child_by_field_name("name")?, src);
    if !is_identifier(&block_name) {
        return None;
    }
    let mut names = Vec::with_capacity(ordinary.len());
    for parameter in ordinary {
        if parameter.kind() != "identifier" {
            return None;
        }
        let name = text(*parameter, src);
        if !is_identifier(&name) {
            return None;
        }
        names.push(name);
    }
    let mut seen = HashSet::new();
    if !names
        .iter()
        .chain(std::iter::once(&block_name))
        .all(|name| seen.insert(name.as_str()))
    {
        return None;
    }
    Some(MethodParameters::FinalBlock {
        ordinary: names,
        block_name,
    })
}

fn exact_call<'tree>(node: Node<'tree>, name: &str, src: &[u8]) -> Option<Node<'tree>> {
    (node.kind() == "call"
        && node.child_by_field_name("receiver").is_none()
        && node.child_by_field_name("block").is_none()
        && text(node.child_by_field_name("method")?, src) == name)
        .then_some(node)
}

fn is_sig_call(node: Node, src: &[u8]) -> bool {
    node.kind() == "call"
        && node.child_by_field_name("receiver").is_none()
        && text(node.child_by_field_name("method").unwrap_or(node), src) == "sig"
}

fn is_dot_named_call(node: Node, name: &str, src: &[u8]) -> bool {
    node.kind() == "call"
        && node.child_by_field_name("block").is_none()
        && node.child_by_field_name("receiver").is_some()
        && text(node.child_by_field_name("method").unwrap_or(node), src) == name
        && dot_call(node, src)
}

fn is_t_proc(node: Node, src: &[u8]) -> bool {
    node.kind() == "call"
        && node.child_by_field_name("block").is_none()
        && node.child_by_field_name("arguments").is_none()
        && text(node.child_by_field_name("method").unwrap_or(node), src) == "proc"
        && node
            .child_by_field_name("receiver")
            .is_some_and(|receiver| receiver.kind() == "constant" && text(receiver, src) == "T")
        && dot_call(node, src)
}

fn dot_call(node: Node, src: &[u8]) -> bool {
    node.child_by_field_name("operator")
        .is_some_and(|operator| text(operator, src) == ".")
}

fn parse_named_arguments<'tree>(
    arguments: Node<'tree>,
    src: &[u8],
) -> Option<Vec<(String, Node<'tree>)>> {
    if arguments.kind() != "argument_list" {
        return None;
    }
    let pairs = named_children(arguments);
    let mut result = Vec::with_capacity(pairs.len());
    let mut seen = HashSet::new();
    for pair in pairs {
        if pair.kind() != "pair" {
            return None;
        }
        let key = text(pair.child_by_field_name("key")?, src);
        if !is_identifier(&key) {
            return None;
        }
        if !seen.insert(key.clone()) {
            return None;
        }
        result.push((key, pair.child_by_field_name("value")?));
    }
    Some(result)
}

fn one_argument<'tree>(arguments: Node<'tree>) -> Option<Node<'tree>> {
    let arguments = named_children(arguments);
    (arguments.len() == 1).then_some(arguments[0])
}

fn simple_type(node: Node, src: &[u8]) -> Option<String> {
    matches!(node.kind(), "constant" | "scope_resolution")
        .then(|| text(node, src))
        .filter(|value| is_simple_path(value))
}

fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or_default().to_string()
}

fn source_first_line(node: Node, src: &[u8]) -> Option<String> {
    text(node, src)
        .lines()
        .next()
        .map(|line| line.trim().to_string())
}

fn qualify_scope_name(prefix: &str, name: &str) -> String {
    if name.starts_with("::") || prefix.is_empty() {
        super::helpers::qualify(name.trim_start_matches("::"), "")
    } else {
        super::helpers::qualify(name, prefix)
    }
}

fn is_simple_path(value: &str) -> bool {
    let value = value.strip_prefix("::").unwrap_or(value);
    !value.is_empty() && value.split("::").all(is_constant_name)
}

fn is_constant_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
