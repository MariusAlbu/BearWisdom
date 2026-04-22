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
    // This is done on the raw source text (not via tree-sitter) because it is
    // simpler and fast enough; the three recognised patterns are straightforward
    // to scan line by line.
    emit_dotnet_binding_sentinels(source, &mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}

/// Scan `source` for .NET object-creation assignments and emit one sentinel
/// `Imports` ref per binding found.
fn emit_dotnet_binding_sentinels(source: &str, refs: &mut Vec<ExtractedRef>) {
    for (line_no, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let binding = try_parse_new_object(line)
            .or_else(|| try_parse_type_new(line))
            .or_else(|| try_parse_typed_param(line));

        if let Some((var_name, dotnet_type)) = binding {
            if is_dotnet_type_name(&dotnet_type) {
                refs.push(ExtractedRef {
                    source_symbol_index: 0, // sentinel — not tied to a specific symbol
                    target_name: DOTNET_BINDING_SENTINEL.to_string(),
                    kind: EdgeKind::Imports,
                    line: line_no as u32,
                    module: Some(var_name),
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// .NET pattern parsers (shared with resolve.rs via pub(crate))
// ---------------------------------------------------------------------------

/// Try to parse `$var = New-Object [-TypeName] Type.Name [args...]`.
/// Returns `(var_name_without_dollar, type_name)` on success.
pub(crate) fn try_parse_new_object(line: &str) -> Option<(String, String)> {
    if !line.starts_with('$') {
        return None;
    }
    let eq_pos = line.find('=')?;
    let lhs = line[..eq_pos].trim();
    let rhs = line[eq_pos + 1..].trim();

    let var_raw = lhs.trim_start_matches('$');
    let var_name = if let Some(pos) = var_raw.find(':') {
        var_raw[pos + 1..].to_string()
    } else {
        var_raw.to_string()
    };
    if var_name.is_empty() {
        return None;
    }

    let rhs_lower = rhs.to_ascii_lowercase();
    if !rhs_lower.starts_with("new-object") {
        return None;
    }

    let after_cmd = rhs[10..].trim();
    let type_part = if after_cmd.to_ascii_lowercase().starts_with("-typename") {
        after_cmd[9..].trim()
    } else {
        after_cmd
    };

    // Type name is first whitespace-delimited token; strip trailing `(` for
    // patterns like `New-Object Windows.CornerRadius(10)`.
    let raw_token = type_part.split_ascii_whitespace().next()?;
    let type_name = raw_token
        .find('(')
        .map(|p| &raw_token[..p])
        .unwrap_or(raw_token)
        .to_string();
    if type_name.is_empty() {
        return None;
    }

    Some((var_name, type_name))
}

/// Try to parse `$var = [Type.Name]::new(...)`.
/// Returns `(var_name_without_dollar, type_name)` on success.
pub(crate) fn try_parse_type_new(line: &str) -> Option<(String, String)> {
    if !line.starts_with('$') {
        return None;
    }
    let eq_pos = line.find('=')?;
    let lhs = line[..eq_pos].trim();
    let rhs = line[eq_pos + 1..].trim();

    let var_raw = lhs.trim_start_matches('$');
    let var_name = if let Some(pos) = var_raw.find(':') {
        var_raw[pos + 1..].to_string()
    } else {
        var_raw.to_string()
    };
    if var_name.is_empty() {
        return None;
    }

    if !rhs.starts_with('[') {
        return None;
    }

    // Depth-counting scan to handle nested brackets in generic types:
    //   [System.Collections.Generic.List[string]]::new()
    //   [System.Collections.Hashtable]::new()
    // We count `[` depth to find the matching outer `]`.
    let close_bracket = find_matching_close_bracket(&rhs[1..])?;
    // close_bracket is relative to rhs[1..], so absolute index = close_bracket + 1
    let abs_close = close_bracket + 1;
    let raw_type = rhs[1..abs_close].trim();
    if raw_type.is_empty() {
        return None;
    }
    // Strip generic type arguments for the stored type name:
    //   "System.Collections.Generic.List[string]" → "System.Collections.Generic.List"
    let type_name = strip_type_args(raw_type);
    if type_name.is_empty() {
        return None;
    }

    let after_bracket = rhs[abs_close + 1..].trim_start();
    if !after_bracket.to_ascii_lowercase().starts_with("::new") {
        return None;
    }

    Some((var_name, type_name))
}

/// Try to parse `[Type.Name]$var` typed parameter / variable.
/// Returns `(var_name_without_dollar, type_name)` on success.
pub(crate) fn try_parse_typed_param(line: &str) -> Option<(String, String)> {
    if !line.starts_with('[') {
        return None;
    }

    let close_bracket = line.find(']')?;
    let type_name = line[1..close_bracket].trim().to_string();
    if type_name.is_empty() {
        return None;
    }

    let after_bracket = line[close_bracket + 1..].trim_start();
    if !after_bracket.starts_with('$') {
        return None;
    }

    let rest = &after_bracket[1..];
    let name_end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    let var_name = rest[..name_end].to_string();
    if var_name.is_empty() {
        return None;
    }

    Some((var_name, type_name))
}

/// Returns `true` if `type_name` looks like a .NET framework namespace path.
/// Must contain a `.` and start with a recognised top-level namespace segment.
/// Generic type args (e.g. `[string]`, `<int>`) are stripped before checking.
pub(crate) fn is_dotnet_type_name(type_name: &str) -> bool {
    let base = strip_type_args(type_name);
    if !base.contains('.') {
        return false;
    }
    let root = base.split('.').next().unwrap_or("");
    matches!(
        root,
        "System"
            | "Microsoft"
            | "Windows"
            | "WPF"
            | "PresentationFramework"
            | "PresentationCore"
    )
}

/// Strip PowerShell-style generic type arguments from a type name.
///
/// Examples:
///   "System.Collections.Generic.List[string]"  → "System.Collections.Generic.List"
///   "System.Collections.Hashtable"             → "System.Collections.Hashtable"
///   "System.Action[string,int]"                → "System.Action"
fn strip_type_args(s: &str) -> String {
    // Strip everything from the first `[` or `<` that follows an identifier char.
    let bracket_pos = s
        .char_indices()
        .find(|&(_, c)| c == '[' || c == '<')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s[..bracket_pos].trim_end().to_string()
}

/// Depth-counting scan for the closing `]` that matches the opening `[`
/// which is assumed to appear just before `s` (i.e. `s` starts immediately
/// after the opening `[`).
///
/// Returns the index within `s` of the matching `]`, or `None` if not found.
fn find_matching_close_bracket(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
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
                        byte_offset: 0,
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
                            byte_offset: 0,
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
            byte_offset: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// Command (cmdlet invocation) → Calls edge
// ---------------------------------------------------------------------------

fn extract_command(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The command name is in the `command_name` child
    let cmd_name = match find_child_text(node, "command_name", src) {
        Some(n) => n,
        None => return,
    };

    // For `Import-Module`, try to extract the module name as an Imports edge;
    // fall back to emitting a Calls edge so the command node is always covered.
    if cmd_name.eq_ignore_ascii_case("Import-Module") {
        let mut emitted = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let text = node_text(&child, src);
            if child.kind() == "command_elements"
                || child.kind() == "string_literal"
                || child.kind() == "bare_string_literal"
            {
                let module = text.trim_matches(|c| c == '"' || c == '\'').to_string();
                if !module.is_empty() && module != cmd_name {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: module.clone(),
                        kind: EdgeKind::Imports,
                        line: node.start_position().row as u32,
                        module: Some(module),
                        chain: None,
                        byte_offset: 0,
                    });
                    emitted = true;
                    break;
                }
            }
        }
        if !emitted {
            // Couldn't resolve module name — still emit so the node is covered
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: cmd_name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
            });
        }
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: cmd_name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
    });
}

// ---------------------------------------------------------------------------
// Walk subtree collecting command/call nodes
// ---------------------------------------------------------------------------

fn visit_for_calls(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "command" {
            extract_command(&child, src, source_idx, refs);
            // Recurse so script-block args (ForEach-Object { ... }) are visited.
            visit_for_calls(&child, src, source_idx, refs);
        } else if child.kind() == "invokation_expression" {
            // Method call: extract method name with fallbacks
            let name = find_child_text(&child, "member_name", src)
                .or_else(|| find_child_text(&child, "type_name", src))
                .or_else(|| find_child_text(&child, "simple_name", src))
                .unwrap_or_else(|| {
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
                    byte_offset: 0,
                });
            }
            visit_for_calls(&child, src, source_idx, refs);
        } else {
            visit_for_calls(&child, src, source_idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Script parameter block — `param([type]$Name, ...)`
// ---------------------------------------------------------------------------

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
    // member_access: variable  .  member_name(simple_name)
    let member = find_child_text(node, "member_name", src)
        .or_else(|| find_child_text(node, "simple_name", src));

    if let Some(name) = member {
        if name.is_empty() {
            return;
        }
        // Qualifier: strip `$` from the variable child
        let module = find_child_text(node, "variable", src)
            .map(|v| v.trim_start_matches('$').to_string())
            .filter(|v| !v.is_empty());

        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module,
            chain: None,
            byte_offset: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

fn find_child_text(node: &Node, kind: &str, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(&child, src).to_string());
        }
    }
    None
}

/// Extract the qualifier (module) from an `invokation_expression` node.
///
/// Two patterns:
/// - `[Type]::Method()` — first named child is `type_literal`; collect dotted
///   type name from nested `type_identifier` leaves (e.g. `System.IO.File`).
/// - `$obj.Method()`    — first named child is `variable`; strip leading `$`.
fn invokation_module(node: &Node, src: &str) -> Option<String> {
    // Find the first named child by index to avoid borrowing cursor across the match.
    let first_named_idx = (0..node.child_count()).find(|&i| {
        node.child(i).map_or(false, |c| c.is_named())
    })?;
    let first = node.child(first_named_idx)?;

    match first.kind() {
        "type_literal" => {
            // Collect all type_identifier leaves in document order, join with "."
            let mut parts: Vec<String> = Vec::new();
            collect_type_identifiers(first, src, &mut parts);
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("."))
            }
        }
        "variable" => {
            let raw = node_text(&first, src);
            let stripped = raw.trim_start_matches('$');
            if stripped.is_empty() {
                None
            } else {
                Some(stripped.to_string())
            }
        }
        _ => None,
    }
}

/// Recursively collect all `type_identifier` leaf texts under `node`.
fn collect_type_identifiers(node: tree_sitter::Node, src: &str, out: &mut Vec<String>) {
    if node.kind() == "type_identifier" && node.child_count() == 0 {
        let t = node_text(&node, src).to_string();
        if !t.is_empty() {
            out.push(t);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_identifiers(child, src, out);
    }
}

/// First `simple_name` child (used for class/enum names)
fn first_simple_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "simple_name" {
            let name = node_text(&child, src).to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}
