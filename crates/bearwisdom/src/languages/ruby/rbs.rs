//! Strict, source-independent RBS contract extraction.
//!
//! This deliberately recognizes a tiny contract subset.  An RBS declaration
//! can describe calling conventions that the resolver's compact
//! `Type::Function` cannot model, so every shape outside the exact grammar
//! below is omitted rather than approximated.

use std::collections::HashMap;

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

#[derive(Clone)]
struct Scope {
    qualified_name: Option<String>,
    symbol_index: Option<usize>,
}

struct Method {
    name: String,
    qualified_name: String,
    scope_path: String,
    parent_index: usize,
    signature: String,
    line: u32,
    column: u32,
    byte_offset: u32,
}

/// Extract only a non-overloaded RBS method with required positional inputs
/// and one required block.  The generated signature is an internal,
/// colon-shaped representation consumed by the common type-id population
/// path; its final parameter is the structural `(A, B) -> R` callback type.
pub(super) fn extract(source: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut methods = Vec::new();
    let mut byte_offset = 0u32;
    let mut malformed_scope_structure = false;

    for (line_index, raw_with_ending) in source.split_inclusive('\n').enumerate() {
        let raw_without_newline = raw_with_ending
            .strip_suffix('\n')
            .unwrap_or(raw_with_ending);
        let raw_line = raw_without_newline
            .strip_suffix('\r')
            .unwrap_or(raw_without_newline);
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            byte_offset = byte_offset.saturating_add(raw_with_ending.len() as u32);
            continue;
        }

        if line == "end" {
            if scopes.pop().is_none() {
                malformed_scope_structure = true;
            }
            byte_offset = byte_offset.saturating_add(raw_with_ending.len() as u32);
            continue;
        }

        if let Some((kind, name)) = scope_declaration(line) {
            let parent = scopes.last().and_then(|scope| scope.symbol_index);
            let parent_qname = scopes
                .last()
                .and_then(|scope| scope.qualified_name.as_deref());
            let qualified_name = parent_qname
                .map(|parent| format!("{parent}::{name}"))
                .unwrap_or_else(|| name.to_string());
            let index = result.symbols.len();
            result.symbols.push(ExtractedSymbol {
                name: name.to_string(),
                qualified_name: qualified_name.clone(),
                kind,
                visibility: Some(Visibility::Public),
                start_line: line_index as u32,
                end_line: line_index as u32,
                start_col: raw_line.find(name).unwrap_or(0) as u32,
                end_col: raw_line.len() as u32,
                byte_offset: byte_offset.saturating_add(raw_line.find(name).unwrap_or(0) as u32),
                signature: Some(format!("{line}")),
                doc_comment: None,
                scope_path: parent_qname.map(str::to_string),
                parent_index: parent,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
            scopes.push(Scope {
                qualified_name: Some(qualified_name),
                symbol_index: Some(index),
            });
        } else if opens_unsupported_scope(line) {
            // Keep stack balance while ensuring descendants cannot inherit a
            // trusted qualification from an unsupported declaration shape.
            scopes.push(Scope {
                qualified_name: None,
                symbol_index: None,
            });
        } else if let Some(scope) = scopes.last() {
            if let (Some(qualified_name), Some(parent_index)) =
                (scope.qualified_name.as_deref(), scope.symbol_index)
            {
                if let Some((name, signature)) = parse_method(line) {
                    let name_col = raw_line.find(&name).unwrap_or(0) as u32;
                    methods.push(Method {
                        name: name.clone(),
                        qualified_name: format!("{qualified_name}::{name}"),
                        scope_path: qualified_name.to_string(),
                        parent_index,
                        signature,
                        line: line_index as u32,
                        column: name_col,
                        byte_offset: byte_offset.saturating_add(name_col),
                    });
                }
            }
        }

        byte_offset = byte_offset.saturating_add(raw_with_ending.len() as u32);
    }

    // This deliberately raw parser cannot recover a trustworthy declaration
    // nesting after an unmatched terminator or an unclosed supported/unknown
    // scope. Keep the scope symbols for discovery, but do not surface any
    // callable contract from a globally malformed file.
    if malformed_scope_structure || !scopes.is_empty() {
        return result;
    }

    let mut counts = HashMap::<String, usize>::new();
    for method in &methods {
        *counts.entry(method.qualified_name.clone()).or_default() += 1;
    }
    for method in methods
        .into_iter()
        .filter(|method| counts[&method.qualified_name] == 1)
    {
        result.symbols.push(ExtractedSymbol {
            name: method.name,
            qualified_name: method.qualified_name,
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: method.line,
            end_line: method.line,
            start_col: method.column,
            end_col: method.column,
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
    result
}

fn scope_declaration(line: &str) -> Option<(SymbolKind, &str)> {
    let (keyword, kind) = if let Some(rest) = line.strip_prefix("class ") {
        (rest, SymbolKind::Class)
    } else if let Some(rest) = line.strip_prefix("module ") {
        (rest, SymbolKind::Interface)
    } else {
        return None;
    };
    let name = keyword.trim();
    is_scope_name(name).then_some((kind, name))
}

fn opens_unsupported_scope(line: &str) -> bool {
    matches!(
        line.split_whitespace().next(),
        Some("class" | "module" | "interface")
    )
}

fn parse_method(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("def ")?;
    let (name, rest) = rest.split_once(':')?;
    let name = name.trim();
    if !is_method_name(name) {
        return None;
    }
    let rest = rest.trim();
    let (ordinary, rest) = take_group(rest, '(', ')')?;
    let ordinary = parse_required_positional_types(ordinary)?;
    let rest = rest.trim_start();
    let (block, rest) = take_group(rest, '{', '}')?;
    let (callback_params, callback_return) = parse_required_block(block.trim())?;
    let return_type = rest.trim_start().strip_prefix("->")?.trim();
    if !is_simple_type(return_type) {
        return None;
    }

    let mut params: Vec<String> = ordinary
        .into_iter()
        .enumerate()
        .map(|(index, ty)| format!("arg{index}: {ty}"))
        .collect();
    params.push(format!(
        "callback: ({}) -> {callback_return}",
        callback_params.join(", ")
    ));
    Some((
        name.to_string(),
        format!("{name}({}): {return_type}", params.join(", ")),
    ))
}

fn parse_required_positional_types(group: &str) -> Option<Vec<String>> {
    if group.trim().is_empty() {
        return Some(Vec::new());
    }
    let types: Vec<_> = group.split(',').map(str::trim).collect();
    (!types.is_empty() && types.iter().all(|ty| is_simple_type(ty)))
        .then(|| types.into_iter().map(str::to_string).collect())
}

fn parse_required_block(group: &str) -> Option<(Vec<String>, String)> {
    let (params, rest) = take_group(group, '(', ')')?;
    let return_type = rest.trim_start().strip_prefix("->")?.trim();
    let params = parse_required_positional_types(params)?;
    is_simple_type(return_type).then(|| (params, return_type.to_string()))
}

/// Consume one fully balanced group and return its inner text plus the suffix.
/// RBS types accepted by this strict subset cannot nest a group, but balanced
/// scanning keeps malformed delimiters from being reinterpreted as contracts.
fn take_group(input: &str, open: char, close: char) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if !input.starts_with(open) {
        return None;
    }
    let mut depth = 0i32;
    for (index, ch) in input.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some((
                    &input[open.len_utf8()..index],
                    &input[index + close.len_utf8()..],
                ));
            }
            if depth < 0 {
                return None;
            }
        }
    }
    None
}

fn is_method_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '!' | '?'))
}

fn is_path(value: &str) -> bool {
    let value = value.strip_prefix("::").unwrap_or(value);
    !value.is_empty()
        && value.split("::").all(|part| {
            !part.is_empty()
                && is_method_name(part)
                && part
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_uppercase())
        })
}

fn is_scope_name(value: &str) -> bool {
    !value.contains("::")
        && is_method_name(value)
        && value
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
}

fn is_simple_type(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty()
        || value
            .contains(|c: char| matches!(c, '[' | ']' | '<' | '>' | '|' | '&' | '?' | '*' | ':'))
        || matches!(
            value,
            "untyped" | "top" | "bot" | "self" | "instance" | "class"
        )
    {
        return false;
    }
    if value.len() == 1 && value.as_bytes()[0].is_ascii_uppercase() {
        return false;
    }
    value == "void" || is_path(value)
}
