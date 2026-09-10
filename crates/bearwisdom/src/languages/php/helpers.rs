// =============================================================================
// php/helpers.rs  —  Shared utilities for the PHP extractor
// =============================================================================

use crate::types::{SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn node_text(node: &Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

/// Dot-separated qualifier (used for qualified names within a namespace).
pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// Dot-separated qualifier for namespace symbols themselves — same scheme as
/// `qualify()`, so a namespace's qualified name composes with a class's the
/// same way a class's composes with a member's.
pub(super) fn qualify_ns(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Read the visibility modifier of a method or property declaration.
/// Defaults to Public if no modifier is present (interfaces, enum methods, etc.).
pub(super) fn extract_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(&child, src);
            return match text.as_str() {
                "public" => Some(Visibility::Public),
                "protected" => Some(Visibility::Protected),
                "private" => Some(Visibility::Private),
                _ => Some(Visibility::Public),
            };
        }
    }
    Some(Visibility::Public)
}

/// The immediately preceding PHPDoc block, if it is separated from `node` by
/// whitespace only. A regular comment or any declaration/attribute between
/// the block and the declaration intentionally breaks association.
pub(super) fn adjacent_phpdoc(node: &Node, src: &[u8]) -> Option<String> {
    let comment = node.prev_named_sibling()?;
    if comment.kind() != "comment"
        || !src[comment.end_byte()..node.start_byte()]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let comment = node_text(&comment, src);
    let trimmed = comment.trim();
    (trimmed.starts_with("/**") && trimmed.ends_with("*/")).then_some(comment)
}

pub(super) fn build_method_signature(
    node: &Node,
    src: &[u8],
    name: &str,
    phpdoc: Option<&str>,
) -> Option<String> {
    let params = node
        .child_by_field_name("parameters")
        .map(|p| enrich_phpdoc_callable_params(&p, src, phpdoc))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| format!(": {}", node_text(&r, src)))
        .unwrap_or_default();
    Some(format!("function {name}{params}{ret}"))
}

#[derive(Debug)]
struct PhpDocCallableParam {
    name: String,
    type_: String,
}

/// Enrich only plain, non-variadic slots with one unambiguous supported PHPDoc
/// callable tag. Replacements run right-to-left over the raw formal parameter
/// text, preserving the variable and spelling of every other slot.
fn enrich_phpdoc_callable_params(node: &Node, src: &[u8], phpdoc: Option<&str>) -> String {
    let raw = node_text(node, src);
    let Some(phpdoc) = phpdoc else {
        return raw;
    };
    let documented = parse_phpdoc_callable_params(phpdoc);
    if documented.is_empty() {
        return raw;
    }

    let mut replacements = Vec::new();
    let mut cursor = node.walk();
    for parameter in node.named_children(&mut cursor) {
        if parameter.kind() != "simple_parameter" {
            continue;
        }
        let Some(name) = parameter.child_by_field_name("name") else {
            continue;
        };
        let variable = node_text(&name, src);
        let Some(variable) = variable.strip_prefix('$') else {
            continue;
        };
        let Some(documented) = documented.iter().find(|entry| entry.name == variable) else {
            continue;
        };
        let Some(replacement) = phpdoc_callable_slot_replacement(
            &parameter,
            &name,
            src,
            &documented.type_,
            node.start_byte(),
        ) else {
            continue;
        };
        replacements.push(replacement);
    }

    let mut enriched = raw;
    for (start, end, replacement) in replacements.into_iter().rev() {
        if start <= end && end <= enriched.len() {
            enriched.replace_range(start..end, &replacement);
        }
    }
    enriched
}

/// Replace just a compatible native type, or insert a structural type before
/// an untyped variable. Defaults, references, and attributes can change the
/// declaration grammar around that insertion, so their contracts abstain.
fn phpdoc_callable_slot_replacement(
    parameter: &Node,
    variable: &Node,
    src: &[u8],
    callable_type: &str,
    parameters_start: usize,
) -> Option<(usize, usize, String)> {
    if parameter
        .child_by_field_name("reference_modifier")
        .is_some()
        || parameter.child_by_field_name("default_value").is_some()
        || parameter.child_by_field_name("attributes").is_some()
    {
        return None;
    }

    if let Some(type_) = parameter.child_by_field_name("type") {
        if !matches!(
            node_text(&type_, src).trim(),
            "callable" | "Closure" | "\\Closure"
        ) {
            return None;
        }
        return Some((
            type_.start_byte().saturating_sub(parameters_start),
            type_.end_byte().saturating_sub(parameters_start),
            callable_type.to_owned(),
        ));
    }

    let start = variable.start_byte().saturating_sub(parameters_start);
    Some((start, start, format!("{callable_type} ")))
}

/// Parse exactly `@param callable(A, B): R $name` tags. Variadic,
/// union/intersection, generic, nullable, and descriptive forms abstain: the
/// stored signature must contain only a function type whose slots are known to
/// align with the native declaration.
fn parse_phpdoc_callable_params(phpdoc: &str) -> Vec<PhpDocCallableParam> {
    let Some(body) = phpdoc
        .trim()
        .strip_prefix("/**")
        .and_then(|doc| doc.strip_suffix("*/"))
    else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    let mut duplicates = Vec::new();
    for raw_line in body.lines() {
        let line = raw_line.trim().trim_start_matches('*').trim_start();
        let Some(entry) = parse_phpdoc_callable_tag(line) else {
            continue;
        };
        if duplicates.iter().any(|name| name == &entry.name) {
            continue;
        }
        if let Some(index) = entries
            .iter()
            .position(|known: &PhpDocCallableParam| known.name == entry.name)
        {
            // Repeated tags make the declaration-to-contract mapping ambiguous.
            duplicates.push(entries.remove(index).name);
        } else {
            entries.push(entry);
        }
    }
    entries
}

fn parse_phpdoc_callable_tag(line: &str) -> Option<PhpDocCallableParam> {
    let rest = line.strip_prefix("@param")?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let callable = rest.strip_prefix("callable(")?;
    let close = callable.find(')')?;
    let params = &callable[..close];
    let after_params = callable[close + 1..].trim_start();
    let after_colon = after_params.strip_prefix(':')?.trim_start();
    let return_end = after_colon.find(char::is_whitespace)?;
    let return_ = after_colon[..return_end].trim();
    let variable = after_colon[return_end..].trim();
    let name = variable.strip_prefix('$')?;
    if !is_phpdoc_callable_type(return_) || !is_php_identifier(name) {
        return None;
    }

    let mut types = Vec::new();
    if !params.trim().is_empty() {
        for param in params.split(',') {
            let param = param.trim();
            if !is_phpdoc_callable_type(param) {
                return None;
            }
            types.push(param);
        }
    }
    Some(PhpDocCallableParam {
        name: name.to_string(),
        type_: format!("({}) -> {return_}", types.join(", ")),
    })
}

fn is_phpdoc_callable_type(type_: &str) -> bool {
    !type_.is_empty()
        && type_
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'\\'))
}

fn is_php_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'_' | b'a'..=b'z' | b'A'..=b'Z'))
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

pub(super) fn build_class_signature(
    node: &Node,
    src: &[u8],
    name: &str,
    kind: SymbolKind,
) -> Option<String> {
    let keyword = match kind {
        SymbolKind::Interface => "interface",
        _ => "class",
    };

    let base = node
        .child_by_field_name("base_clause")
        .map(|b| {
            format!(
                " extends {}",
                node_text(&b, src).trim_start_matches("extends ").trim()
            )
        })
        .unwrap_or_default();

    let impls = node
        .child_by_field_name("class_implements")
        .map(|i| {
            format!(
                " implements {}",
                node_text(&i, src).trim_start_matches("implements ").trim()
            )
        })
        .unwrap_or_default();

    Some(format!("{keyword} {name}{base}{impls}"))
}
