// =============================================================================
// php/calls.rs  —  Call extraction and import ref helpers for PHP
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Each arm that recurses explicitly MUST `continue` — otherwise the
    // unconditional recursion at the bottom of the loop visits the same
    // subtree again, doubling work at every nesting level. A method chain
    // N deep (common in Laravel fluent builders) otherwise costs O(2^N)
    // recursions and ref-pushes, blowing memory into hundreds of MiB.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "member_call_expression" | "nullsafe_member_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    let chain = build_chain(&child, src);
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &name_node, refs);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        module: None,
                        chain,
                        byte_offset: name_node.start_byte() as u32,
                    });
                }
                // Recurse into the object expression and arguments to find nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
                continue;
            }

            "static_call_expression" | "scoped_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    let chain = build_chain(&child, src);
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &name_node, refs);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        module: None,
                        chain,
                        byte_offset: name_node.start_byte() as u32,
                    });
                }
                // Recurse into arguments to find nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
                continue;
            }

            "object_creation_expression" => {
                let cls_node_opt = if let Some(n) = child.child_by_field_name("class_type") {
                    Some(n)
                } else {
                    let mut c = child.walk();
                    let mut found = None;
                    for n in child.children(&mut c) {
                        if n.kind() == "name"
                            || n.kind() == "qualified_name"
                            || n.kind() == "identifier"
                            || n.kind() == "variable_name"
                        {
                            found = Some(n);
                            break;
                        }
                    }
                    found
                };
                if let Some(cls_node) = cls_node_opt {
                    let cls_name = node_text(&cls_node, src);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: cls_name,
                        kind: EdgeKind::Instantiates,
                        line: cls_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }

            "function_call_expression" => {
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let callee = node_text(&fn_node, src);
                    let simple = callee.rsplit('\\').next().unwrap_or(&callee).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::Calls,
                        line: fn_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }

            // `match($x) { 1 => 'one', default => 'other' }` — recurse into arms.
            "match_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
                continue;
            }

            // `fn($x) => $x->name` — recurse into body.
            "arrow_function" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                }
                continue;
            }

            // `function() use ($x) { ... }` — anonymous function.
            // Extract calls from the body; `use` clause variables are already in scope.
            "anonymous_function_creation_expression" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                }
                continue;
            }

            // `include 'file.php'` / `require_once 'config.php'` — emit Imports edge.
            "include_expression" | "include_once_expression"
            | "require_expression" | "require_once_expression" => {
                extract_include_require(&child, src, refs, source_symbol_index);
            }

            // `"Hello $name and {$obj->method()}"` — interpolated string with embedded expressions.
            "encapsed_string" => {
                extract_encapsed_string_calls(&child, src, source_symbol_index, refs);
                continue;
            }

            _ => {}
        }
        extract_calls_from_body(&child, src, source_symbol_index, refs);
    }
}

/// Extract an Imports edge from an `include`/`require`/`include_once`/`require_once` expression.
pub(super) fn extract_include_require(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    // The path expression is the only named child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "encapsed_string" {
            let raw = node_text(&child, src);
            // Strip surrounding quotes.
            let path = raw
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('\'')
                .to_string();
            if path.is_empty() {
                continue;
            }
            let parts: Vec<&str> = path.split('/').collect();
            let target = parts
                .last()
                .unwrap_or(&path.as_str())
                .trim_end_matches(".php")
                .to_string();
            let module = if parts.len() > 1 {
                Some(parts[..parts.len() - 1].join("/"))
            } else {
                None
            };
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: target,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
                byte_offset: 0,
            });
        }
    }
}

/// Extract calls from interpolated expressions inside a PHP double-quoted string.
///
/// `"Hello {$obj->greet()}"` — the `{$obj->greet()}` part is an embedded expression.
fn extract_encapsed_string_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `{$expr}` — variable_name or expression inside braces.
            "variable_name" => {} // simple var, no call
            // Expression-level interpolation (e.g. method call inside `{...}`).
            _ if child.is_named() => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Variable extraction helpers for PHP
// ---------------------------------------------------------------------------

/// Extract Variable symbols from a PHP `foreach` statement.
///
/// ```text
/// foreach ($items as $key => $value) { ... }
/// ```
pub(super) fn extract_foreach_vars(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<crate::types::ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    source_symbol_index: usize,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    // Value field: `$value` (or `$key => $value` — the value is the last binding).
    if let Some(value_node) = node.child_by_field_name("value") {
        push_php_foreach_var(&value_node, src, symbols, parent_index, qualified_prefix);
    }
    // Key field (optional): `$key`.
    if let Some(key_node) = node.child_by_field_name("key") {
        push_php_foreach_var(&key_node, src, symbols, parent_index, qualified_prefix);
    }

    // Recurse into the body.
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, source_symbol_index, refs);
    }

    // Fallback: walk children for variable_name nodes when fields are absent.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "body" || child.kind() == "foreach" || child.kind() == "as" {
            continue;
        }
        if child.kind() == "variable_name" {
            use super::helpers::{qualify, scope_from_prefix};
            let raw = node_text(&child, src);
            let name = raw.trim_start_matches('$').to_string();
            if !name.is_empty() && name != "this" {
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: qualify(&name, qualified_prefix),
                    kind: SymbolKind::Variable,
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
    }
}

fn push_php_foreach_var(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    // Resolve the effective variable_name node.
    let var_found: Option<tree_sitter::Node>;
    if node.kind() == "by_ref" {
        // `foreach ($items as &$item)` — walk children to find variable_name.
        let mut found = None;
        for i in 0..node.child_count() {
            if let Some(ch) = node.child(i) {
                if ch.kind() == "variable_name" {
                    found = Some(ch);
                    break;
                }
            }
        }
        var_found = found;
    } else if node.kind() == "variable_name" {
        var_found = Some(*node);
    } else {
        var_found = None;
    }

    if let Some(var_node) = var_found {
        let raw = node_text(&var_node, src);
        let name = raw.trim_start_matches('$').to_string();
        if !name.is_empty() && name != "this" {
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: qualify(&name, qualified_prefix),
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: var_node.start_position().row as u32,
                end_line: var_node.end_position().row as u32,
                start_col: var_node.start_position().column as u32,
                end_col: var_node.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
        }
    }
}

/// Extract TypeRef edges from catch clauses in a `try_statement`.
pub(super) fn extract_try_catch_types(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<crate::types::ExtractedRef>,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    source_symbol_index: usize,
) {
    use crate::types::{EdgeKind, ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `try { ... }` body.
            "compound_statement" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `catch (ExceptionType $e) { ... }`.
            "catch_clause" => {
                // Collect exception type(s).
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_catch_type_refs(&type_node, src, refs, source_symbol_index);
                }
                // Catch variable.
                if let Some(var_node) = child.child_by_field_name("variable") {
                    let raw = node_text(&var_node, src);
                    let name = raw.trim_start_matches('$').to_string();
                    if !name.is_empty() {
                        symbols.push(ExtractedSymbol {
                            name: name.clone(),
                            qualified_name: qualify(&name, qualified_prefix),
                            kind: SymbolKind::Variable,
                            visibility: Some(Visibility::Public),
                            start_line: var_node.start_position().row as u32,
                            end_line: var_node.end_position().row as u32,
                            start_col: var_node.start_position().column as u32,
                            end_col: var_node.end_position().column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_from_prefix(qualified_prefix),
                            parent_index,
                        });
                    }
                }
                // Recurse into catch body.
                let mut cc = child.walk();
                for cb in child.children(&mut cc) {
                    if cb.kind() == "compound_statement" {
                        extract_calls_from_body(&cb, src, source_symbol_index, refs);
                    }
                }
            }
            // `finally { ... }`.
            "finally_clause" => {
                let mut fc = child.walk();
                for fb in child.children(&mut fc) {
                    if fb.kind() == "compound_statement" {
                        extract_calls_from_body(&fb, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_catch_type_refs(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<crate::types::ExtractedRef>,
    source_symbol_index: usize,
) {
    use crate::types::EdgeKind;
    match node.kind() {
        "named_type" | "name" | "qualified_name" => {
            let name = node_text(node, src);
            let simple = name.rsplit('\\').next().unwrap_or(&name).to_string();
            if !simple.is_empty() {
                refs.push(crate::types::ExtractedRef {
                    source_symbol_index,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
        // `ExceptionA|ExceptionB` — union of exception types.
        "union_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_catch_type_refs(&child, src, refs, source_symbol_index);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_catch_type_refs(&child, src, refs, source_symbol_index);
                }
            }
        }
    }
}

/// Extract Variable symbols from `list($a, $b) = ...` or `[$a, $b] = ...`.
pub(super) fn extract_list_destructuring(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
    use super::helpers::{qualify, scope_from_prefix};

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_name" => {
                let raw = node_text(&child, src);
                let name = raw.trim_start_matches('$').to_string();
                if !name.is_empty() && name != "this" {
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name: qualify(&name, qualified_prefix),
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                    });
                }
            }
            // Nested array destructure element.
            "array_element" | "list_literal" | "array_creation_expression" => {
                extract_list_destructuring(&child, src, symbols, parent_index, qualified_prefix);
            }
            _ => {}
        }
    }
}

/// Extract TypeRef edges from a PHP type node, handling nullable, union, and intersection types.
pub(super) fn extract_type_refs_from_php_type(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<crate::types::ExtractedRef>,
    source_symbol_index: usize,
) {
    use crate::types::EdgeKind;
    match node.kind() {
        // `?string` — nullable type: unwrap to inner type.
        "nullable_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        // `string|int` — union type.
        "union_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        // `Foo&Bar` — intersection type.
        "intersection_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        // `(A&B)|C` — disjunctive normal form type (PHP 8.2+).
        "disjunctive_normal_form_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_php_type(&child, src, refs, source_symbol_index);
                }
            }
        }
        "named_type" | "name" | "qualified_name" | "identifier" => {
            let name = node_text(node, src);
            let simple = name.rsplit('\\').next().unwrap_or(&name).to_string();
            // Skip PHP built-in scalar types.
            if !simple.is_empty()
                && !matches!(
                    simple.as_str(),
                    "string" | "int" | "float" | "bool" | "array" | "object" | "null"
                        | "void" | "never" | "mixed" | "callable" | "iterable"
                        | "self" | "static" | "parent"
                )
            {
                refs.push(crate::types::ExtractedRef {
                    source_symbol_index,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

pub(super) fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "variable_name" => {
            let raw = node_text(node, src);
            let name = raw.trim_start_matches('$').to_string();
            let kind = if name == "this" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "variable_name".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "name" | "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_access_expression" => {
            let object = node.child_by_field_name("object")?;
            let name_node = node.child_by_field_name("name")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "member_access_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_call_expression" => {
            let object = node.child_by_field_name("object")?;
            let name_node = node.child_by_field_name("name")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "member_call_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "static_call_expression" | "scoped_call_expression" => {
            // `scoped_call_expression` (PHP 8 grammar) uses field "scope" for the
            // class and field "name" for the method.  Older grammars may use "class".
            // Try both so the chain is built regardless of grammar version.
            let class_node = node
                .child_by_field_name("scope")
                .or_else(|| node.child_by_field_name("class"))?;
            let name_node = node.child_by_field_name("name")?;
            segments.push(ChainSegment {
                name: node_text(&class_node, src),
                node_kind: "class".to_string(),
                kind: SegmentKind::TypeAccess,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "static_call_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_use_declaration(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => {
                push_use_ref_for_name(&child, src, refs, current_symbol_count);
            }
            "qualified_name" | "name" => {
                let full = node_text(&child, src);
                push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
            }
            _ => {}
        }
    }
}

fn push_use_ref_for_name(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
            return;
        }
    }
}

/// Push an Imports edge for a fully-qualified PHP name like `Foo\Bar\Baz`.
fn push_fq_import(
    full: String,
    line: u32,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let parts: Vec<&str> = full.split('\\').collect();
    let target = parts.last().unwrap_or(&full.as_str()).to_string();
    let module = if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("\\"))
    } else {
        None
    };
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line,
        module,
        chain: None,
        byte_offset: 0,
    });
}

pub(super) fn extract_trait_use(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            let parts: Vec<&str> = full.split('\\').collect();
            let target = parts.last().unwrap_or(&full.as_str()).to_string();
            let module = if parts.len() > 1 {
                Some(parts[..parts.len() - 1].join("\\"))
            } else {
                None
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count.saturating_sub(1),
                target_name: target,
                kind: EdgeKind::Implements,
                line: child.start_position().row as u32,
                module,
                chain: None,
                byte_offset: 0,
            });
        }
    }
}
