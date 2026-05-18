// =============================================================================
// languages/erlang/attributes.rs  —  Module-level attribute extractors
//
// Handles every top-level form that isn't a function declaration:
//   -module, -export, -record, -behaviour, -import, -include / -include_lib,
//   -type / -opaque, -callback, and wild attributes (-name(value).).
// Plus the helpers each emitter uses (export collection, fa list walker).
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility,
};
use tree_sitter::Node;

use super::extract::{extract_attr_value, extract_attr_value_str, node_text};
use super::functions::arity_value;

// ---------------------------------------------------------------------------
// Pass 1: collect exported names
// ---------------------------------------------------------------------------

pub(super) fn collect_exports(root: Node, src: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "export_attribute" {
            // export_attribute → list of fa (fun/arity) nodes
            collect_fa_list(&child, src, &mut set);
        }
    }
    set
}

fn collect_fa_list(node: &Node, src: &str, set: &mut std::collections::HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "fa" {
            // fa has `fun` (atom) and `arity` (arity node) fields.
            let fun_name = child
                .child_by_field_name("fun")
                .map(|n| node_text(&n, src).to_string())
                .unwrap_or_default();
            let arity = child
                .child_by_field_name("arity")
                .map(|n| arity_value(&n, src).to_string())
                .unwrap_or_default();
            if !fun_name.is_empty() && !arity.is_empty() {
                set.insert(format!("{}/{}", fun_name, arity));
            }
        } else {
            collect_fa_list(&child, src, set);
        }
    }
}

// ---------------------------------------------------------------------------
// Module attribute
// ---------------------------------------------------------------------------

pub(super) fn extract_module(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // -module(name).  The atom inside is the module name
    let text = node_text(node, src);
    // Extract atom from `-module(atom).`
    let name = extract_attr_value(&text, "module");
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("-module({}).", name)),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Record
// ---------------------------------------------------------------------------

pub(super) fn extract_record(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let text = node_text(node, src);
    let name = extract_attr_value(&text, "record");
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Struct,
        visibility: None,
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("-record({}, {{...}}).", name)),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Behaviour → Implements edge
// ---------------------------------------------------------------------------

pub(super) fn extract_behaviour(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    let beh1 = extract_attr_value(&text, "behaviour");
    let behaviour = if beh1.is_empty() {
        extract_attr_value_str(&text, "behavior")
    } else {
        beh1
    };
    if behaviour.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: behaviour.clone(),
        kind: EdgeKind::Implements,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Import attribute
// ---------------------------------------------------------------------------

pub(super) fn extract_import_attr(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // -import(module, [fun/1, ...]).
    // Use structured tree-sitter fields: `module` and `funs` (list of `fa` nodes).
    // Emit one Imports ref per imported function so the resolver can do exact
    // `name/arity → source_module` lookup at resolution time.
    let module_node = match node.child_by_field_name("module") {
        Some(n) => n,
        None => return,
    };
    let module_name = node_text(&module_node, src).trim_matches('\'').to_string();
    if module_name.is_empty() {
        return;
    }

    let line = node.start_position().row as u32;
    let mut cursor = node.walk();
    let mut emitted = false;
    for child in node.children(&mut cursor) {
        if child.kind() != "fa" {
            continue;
        }
        let fun_name = child
            .child_by_field_name("fun")
            .map(|n| node_text(&n, src).to_string())
            .unwrap_or_default();
        let arity_str = child
            .child_by_field_name("arity")
            .map(|n| arity_value(&n, src).to_string())
            .unwrap_or_default();
        if fun_name.is_empty() || arity_str.is_empty() {
            continue;
        }
        let target = format!("{}/{}", fun_name, arity_str);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line,
            module: Some(module_name.clone()),
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
                    col: 0,
});
        emitted = true;
    }

    // When the `funs` list is empty or not structured (parse error), fall back
    // to a single module-level import so the resolver can still wildcard-match.
    if !emitted {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: module_name.clone(),
            kind: EdgeKind::Imports,
            line,
            module: Some(module_name),
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
                    col: 0,
});
    }
}

// ---------------------------------------------------------------------------
// Include directives
// ---------------------------------------------------------------------------

pub(super) fn extract_include(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    // -include("file.hrl"). or -include_lib("app/include/file.hrl").
    let file = if let Some(rest) = text.strip_prefix("-include_lib(") {
        rest.trim_end_matches(").").trim().trim_matches('"').to_string()
    } else if let Some(rest) = text.strip_prefix("-include(") {
        rest.trim_end_matches(").").trim().trim_matches('"').to_string()
    } else {
        return;
    };

    if !file.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: file.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            col: 0,
            module: Some(file),
            chain: None,
            byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// Type alias (-type / -opaque)
// ---------------------------------------------------------------------------

pub(super) fn extract_type_alias(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // type_alias / opaque both have a `name` field → type_name → name field (atom)
    if let Some(type_name_node) = node.child_by_field_name("name") {
        let name = if let Some(inner) = type_name_node.child_by_field_name("name") {
            node_text(&inner, src).to_string()
        } else {
            node_text(&type_name_node, src).to_string()
        };
        if name.is_empty() {
            return;
        }
        let line = node.start_position().row as u32;
        let prefix = if node.kind() == "opaque" { "-opaque" } else { "-type" };
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("{}({}).", prefix, name)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// Callback (-callback)
// ---------------------------------------------------------------------------

pub(super) fn extract_callback(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // callback has a `fun` field → _name (atom text)
    if let Some(fun_node) = node.child_by_field_name("fun") {
        let name = node_text(&fun_node, src).to_string();
        if name.is_empty() {
            return;
        }
        let line = node.start_position().row as u32;
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("-callback {}(...).", name)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// Wild attribute (-name(value).) → Variable
// ---------------------------------------------------------------------------

pub(super) fn extract_wild_attr(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // wild_attribute has a `name` field → attr_name → name field (atom)
    // Skip well-known directives that are already handled by other arms or
    // that do not represent meaningful module-level metadata.
    if let Some(attr_name_node) = node.child_by_field_name("name") {
        let name = if let Some(inner) = attr_name_node.child_by_field_name("name") {
            node_text(&inner, src).to_string()
        } else {
            node_text(&attr_name_node, src).to_string()
        };
        // Skip known directives; only emit Variable for genuine custom attributes.
        const SKIP: &[&str] = &[
            "module", "export", "export_type", "import", "behaviour", "behavior",
            "record", "type", "opaque", "spec", "callback", "define",
            "include", "include_lib", "compile", "file", "on_load",
            "doc", "moduledoc", "deprecated", "feature", "vsn", "author",
        ];
        if name.is_empty() || SKIP.contains(&name.as_str()) {
            return;
        }
        let line = node.start_position().row as u32;
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::Variable,
            visibility: None,
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("-{}(...).", name)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }
}
