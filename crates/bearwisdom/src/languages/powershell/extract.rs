// =============================================================================
// languages/powershell/extract.rs  —  PowerShell symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function   — `function_statement`
//   Class      — `class_statement`
//   Enum       — `enum_statement`
//   EnumMember — `enum_member` (child of enum_statement)
//   Method     — `class_method_definition` (child of class_statement)
//   Property   — `class_property_definition` (child of class_statement)
//   Variable   — `script_parameter` in `param_block`; top-level `assignment_expression`
//
// REFERENCES:
//   Imports    — `using_statement` (using namespace / using module)
//   Imports    — sentinel: .NET local-var type binding (target_name="dotnet-stdlib",
//                module=Some(var_name)); consumed by the resolver's build_file_context
//   Calls      — `command` nodes (every cmdlet/function invocation)
//   Calls      — `invokation_expression` (method calls)
//   TypeRef    — `member_access` (property/field reads)
//   Inherits   — `class_statement` with `:` base type
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

use super::commands::{extract_command, visit_for_calls};
use super::dotnet_bindings::emit_dotnet_binding_sentinels;
use super::node_helpers::{
    collect_type_identifiers, find_child_text, find_root_variable, first_simple_name,
    invokation_module, node_text,
};

// Re-export the public surface other modules in this language plugin
// consume (`resolve`, `resolve_tests`). The `try_parse_*` paths only have
// `#[cfg(test)]` consumers, hence the allow.
pub(crate) use super::dotnet_bindings::is_dotnet_type_name;

#[allow(unused_imports)]
pub(crate) use super::dotnet_bindings::{
    try_parse_cmdlet_result_chain, try_parse_new_object, try_parse_propagation,
    try_parse_type_new, try_parse_typed_param,
};

/// Sentinel target name for .NET variable-type binding refs.
/// The resolver's `build_file_context` looks for `Imports` refs with this
/// target name; `module` carries the variable name (stripped of `$`).
pub(crate) const DOTNET_BINDING_SENTINEL: &str = "dotnet-stdlib";

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_powershell::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult::empty();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, "");

    // .NET local-variable type binding scan.
    //
    // For each `$var = New-Object Windows.Controls.Border` (and similar) found
    // in the raw source, emit a sentinel Imports ref so the resolver can classify
    // subsequent member-access refs on `$var` as dotnet-stdlib external refs
    // rather than unresolved.  The sentinel is:
    //   kind=Imports, target_name="dotnet-stdlib", module=Some(var_name)
    //
    // Also covers three new patterns introduced in Pass 2:
    //   Part 1: $sync["Key"].Member  → binds "sync" (and other registry vars)
    //           to System.Windows.DependencyObject
    //   Part 2: $_.Member inside ForEach-Object/Where-Object → binds "_" to
    //           System.Windows.UIElement (WPF catch-all)
    //   Part 3: (Get-Xxx).Member    → binds "__cmdlet_get_xxx" synthetic tag
    //           to the cmdlet's .NET return type
    //
    // This is done on the raw source text (not via tree-sitter) because it is
    // simpler and fast enough; the three recognised patterns are straightforward
    // to scan line by line.
    emit_dotnet_binding_sentinels(source, &mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}
// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    class_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_statement" => {
                let idx = extract_function_indexed(&child, src, symbols, refs, parent_index);
                // Recurse into function body for nested functions/commands
                visit(child, src, symbols, refs, idx.or(parent_index), class_prefix);
            }
            "class_statement" => {
                extract_class(&child, src, symbols, refs, parent_index);
            }
            "enum_statement" => {
                extract_enum(&child, src, symbols, refs, parent_index);
            }
            "using_statement" => {
                extract_using(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "param_block" => {
                extract_param_block(&child, src, symbols, parent_index);
            }
            "assignment_expression" => {
                // Only extract top-level (script-scope) assignments, not those
                // buried inside function/class bodies (parent_index would be Some
                // for those). The visit caller sets parent_index = None at the
                // program root, so this correctly limits to script scope.
                if parent_index.is_none() {
                    extract_top_level_assignment(&child, src, symbols);
                }
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
            "command" => {
                extract_command(&child, src, parent_index.unwrap_or(0), refs);
                // Recurse into command children so that script-block arguments
                // (e.g. `ForEach-Object { $_.Method() }`) are also visited.
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
            "invokation_expression" => {
                let source_idx = parent_index.unwrap_or(0);
                let name = find_child_text(&child, "member_name", src)
                    .or_else(|| find_child_text(&child, "type_name", src))
                    .or_else(|| find_child_text(&child, "simple_name", src))
                    .unwrap_or_else(|| {
                        // Last resort: first named child text
                        (0..child.child_count())
                            .filter_map(|i| child.child(i))
                            .find(|c| c.is_named())
                            .map(|c| node_text(&c, src).to_string())
                            .unwrap_or_default()
                    });
                if !name.is_empty() {
                    let module = invokation_module(&child, src);
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
            "member_access" => {
                extract_member_access(&child, src, parent_index.unwrap_or(0), refs);
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index, class_prefix);
            }
        }
    }
}

/// Like extract_function but returns the symbol index for use as parent.
fn extract_function_indexed(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name = match find_child_text(node, "function_name", src) {
        Some(n) => n,
        None => return None,
    };

    let line = node.start_position().row as u32;
    let sig = format!("function {} {{ ... }}", name);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    visit_for_calls(node, src, idx, refs);
    Some(idx)
}

// ---------------------------------------------------------------------------
// Function extraction
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match find_child_text(node, "function_name", src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let sig = format!("function {} {{ ... }}", name);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract calls inside function body
    visit_for_calls(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Class extraction
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match first_simple_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let class_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("class {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Detect inheritance: `class Foo : Bar` — the grammar emits two `simple_name`
    // children separated by `:`. The first is the class name (already captured),
    // the second (if a `:` sibling precedes it) is the base class name.
    {
        let mut saw_colon = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                ":" => {
                    saw_colon = true;
                }
                "simple_name" if saw_colon => {
                    let base = node_text(&child, src).to_string();
                    if !base.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: class_idx,
                            target_name: base,
                            kind: EdgeKind::Inherits,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                    saw_colon = false; // only emit once per `:` separator
                }
                _ => {}
            }
        }
    }

    // Extract methods and properties inside class body
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_method_definition" => {
                extract_method(&child, src, symbols, refs, class_idx, &name);
            }
            "class_property_definition" => {
                extract_property(&child, src, symbols, refs, class_idx, &name);
            }
            _ => {}
        }
    }
}

fn extract_method(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
    class_name: &str,
) {
    let method_name = match find_child_text(node, "simple_name", src) {
        Some(n) => n,
        None => return,
    };

    let qualified = format!("{}.{}", class_name, method_name);
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: method_name.clone(),
        qualified_name: qualified,
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("{} ({} method)", method_name, class_name)),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });

    visit_for_calls(node, src, idx, refs);
}

fn extract_property(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
    class_name: &str,
) {
    // Property name is in `variable` child — strip leading `$`
    let raw_name = match find_child_text(node, "variable", src) {
        Some(n) => n,
        None => return,
    };
    let prop_name = raw_name.trim_start_matches('$').to_string();
    let qualified = format!("{}.{}", class_name, prop_name);
    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: prop_name.clone(),
        qualified_name: qualified,
        kind: SymbolKind::Property,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("${}", prop_name)),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });
}

// ---------------------------------------------------------------------------
// Enum extraction
// ---------------------------------------------------------------------------

fn extract_enum(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = match first_simple_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let enum_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("enum {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract individual enum members
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_member" {
            if let Some(member_name) = find_child_text(&child, "simple_name", src) {
                if !member_name.is_empty() {
                    let qualified = format!("{}.{}", name, member_name);
                    symbols.push(ExtractedSymbol {
                        name: member_name.clone(),
                        qualified_name: qualified,
                        kind: SymbolKind::EnumMember,
                        visibility: Some(Visibility::Public),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: 0,
                        signature: Some(member_name),
                        doc_comment: None,
                        scope_path: None,
                        parent_index: Some(enum_idx),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Using / Import-Module
// ---------------------------------------------------------------------------

fn extract_using(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `using namespace Foo.Bar` or `using module MyModule`
    let text = node_text(node, src);
    let line = node.start_position().row as u32;

    // Extract the module/namespace name from `using module Foo` or `using namespace Foo`
    let target = if let Some(rest) = text.strip_prefix("using module ") {
        rest.trim().to_string()
    } else if let Some(rest) = text.strip_prefix("using namespace ") {
        rest.trim().to_string()
    } else {
        return;
    };

    if !target.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target.clone(),
            kind: EdgeKind::Imports,
            line,
            module: Some(target),
            chain: None,
            byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}
fn extract_param_block(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // param_block → parameter_list → script_parameter
    // Walk descendants recursively to handle the nesting.
    extract_script_parameters_recursive(node, src, symbols, parent_index);
}

fn extract_script_parameters_recursive(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "script_parameter" {
            if let Some(raw) = find_child_text(&child, "variable", src) {
                let param_name = raw.trim_start_matches('$').to_string();
                if !param_name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        name: param_name.clone(),
                        qualified_name: param_name.clone(),
                        kind: SymbolKind::Variable,
                        visibility: Some(Visibility::Public),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: 0,
                        signature: Some(format!("${}", param_name)),
                        doc_comment: None,
                        scope_path: None,
                        parent_index,
                    });
                }
            }
        } else {
            // Recurse to handle parameter_list and other wrapper nodes
            extract_script_parameters_recursive(&child, src, symbols, parent_index);
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level assignment `$Var = <expr>`  →  Variable symbol
// ---------------------------------------------------------------------------

/// Walk the `left_assignment_expression` subtree to find the deepest variable node.
fn find_variable_in_subtree(node: &Node, src: &str) -> Option<String> {
    if node.kind() == "variable" {
        let raw = node_text(node, src);
        let name = raw.trim_start_matches('$');
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = find_variable_in_subtree(&child, src) {
            return Some(name);
        }
    }
    None
}

fn extract_top_level_assignment(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // assignment_expression children: left_assignment_expression, assignement_operator, pipeline
    let lhs = match (0..node.child_count())
        .filter_map(|i| node.child(i))
        .find(|c| c.kind() == "left_assignment_expression")
    {
        Some(n) => n,
        None => return,
    };

    if let Some(var_name) = find_variable_in_subtree(&lhs, src) {
        // Strip scope qualifiers: $global:Name → Name, $script:Name → Name
        let clean = if let Some(pos) = var_name.find(':') {
            var_name[pos + 1..].to_string()
        } else {
            var_name
        };
        if clean.is_empty() {
            return;
        }
        symbols.push(ExtractedSymbol {
            name: clean.clone(),
            qualified_name: clean.clone(),
            kind: SymbolKind::Variable,
            visibility: Some(Visibility::Public),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("${}", clean)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Member access `$obj.Property`  →  TypeRef edge
// ---------------------------------------------------------------------------

fn extract_member_access(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // member_access: _primary_expression  .  member_name
    // The _primary_expression may be:
    //   - a `variable`      — direct: $obj.Prop
    //   - an `element_access`  — Part 1: $sync["Key"].Prop
    //   - a `member_access`    — chain: $sync.Form.Prop
    //   - other               — no module extracted
    let member = find_child_text(node, "member_name", src)
        .or_else(|| find_child_text(node, "simple_name", src));

    if let Some(name) = member {
        if name.is_empty() {
            return;
        }

        // Distinguish two member-access shapes:
        //   `[Type]::Member`  — static access on a type literal. The receiver
        //                        is a real type name and the member is a
        //                        type-system reference; emit TypeRef.
        //   `$obj.Member`     — runtime property/field access on a variable
        //                        or expression. The member is hashtable /
        //                        property data, not a type — skip rather
        //                        than flooding unresolved_refs with hash
        //                        keys (`$settings.Rules.PSUseConsistentIndentation.PipelineIndentation`)
        //                        and AST property names (`$ast.EndBlock.Statements`).
        let mut module: Option<String> = None;
        let mut from_type_literal = false;

        // First named child that is not member_name / simple_name.
        let obj_idx = (0..node.child_count()).find(|&i| {
            node.child(i).map_or(false, |c| {
                c.is_named() && c.kind() != "member_name" && c.kind() != "simple_name"
            })
        });
        if let Some(child) = obj_idx.and_then(|i| node.child(i)) {
            match child.kind() {
                "type_literal" => {
                    let mut parts: Vec<String> = Vec::new();
                    collect_type_identifiers(child, src, &mut parts);
                    if !parts.is_empty() {
                        module = Some(parts.join("."));
                        from_type_literal = true;
                    }
                }
                "variable" => {
                    let v = node_text(&child, src).trim_start_matches('$');
                    if !v.is_empty() {
                        module = Some(v.to_string());
                    }
                }
                "element_access" | "member_access" => {
                    if let Some(root) = find_root_variable(&child, src) {
                        module = Some(root);
                    }
                }
                _ => {}
            }
        }

        if !from_type_literal {
            return;
        }

        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}
