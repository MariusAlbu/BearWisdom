// =============================================================================
// languages/pascal/extract.rs  —  Pascal / Delphi extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — declProc / defProc (procedure_declaration / function_declaration)
//   Class     — declType wrapping declClass
//   Interface — declType wrapping declIntf
//   Enum      — declType wrapping declEnum
//   Struct    — declSection with record keyword (record_type)
//   Field     — declField inside declSection
//   Property  — declProp inside declSection
//   Variable  — declVar (module-level var section) / declConst
//   Namespace — unit (unit declaration)
//
// REFERENCES:
//   Imports   — declUses (uses clause)
//   Calls     — exprCall (function/method calls)
//   Inherits  — declClass parent typeref
//   TypeRef   — typeref nodes (type references in signatures)
//
// Grammar: tree-sitter-pascal 0.10.2 (tree-sitter-language ABI, LANGUAGE constant).
// Pascal uses '.' as namespace separator in unit names.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

use super::decls::{
    extract_class, extract_const, extract_decl_type, extract_intf, extract_proc, extract_program,
    extract_section, extract_unit, extract_uses, extract_var,
};
use super::error_recovery::{
    recover_type_decls_from_siblings, try_extract_error_type_decl, try_extract_root_type_decls,
};
use super::normalise::normalise_source;
use super::refs::{extract_call, extract_typeref};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_pascal::LANGUAGE.into())
        .expect("Failed to load Pascal grammar");

    // Normalise source before parsing to prevent cascading parse errors.
    //
    // Known patterns that produce token sequences the tree-sitter-pascal grammar
    // cannot handle, causing it to wipe out all preceding declarations:
    //
    //   1. `{$ifdef FPC}object{$else}record{$endif}` — after pp stripping both
    //      `object` and `record` appear in the token stream. Collapsed to `record`.
    //
    //   2. `bitpacked record` — FPC extension not in grammar; tree-sitter sees
    //      an unknown identifier followed by `record`. Collapsed to `record`.
    //
    //   3. Blank line between `end;` and `);` in variant-record case arms —
    //      prevents tree-sitter from closing the anonymous nested record boundary.
    let normalised;
    let src = if source.contains("{$ifdef") || source.contains("{$if ")
        || source.contains("{$IF") || source.contains("bitpacked")
        || source.contains("end;") || source.contains('<')
    {
        normalised = normalise_source(source);
        normalised.as_str()
    } else {
        source
    };

    let tree = match parser.parse(src, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_root(tree.root_node(), src, &mut symbols, &mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Root traversal
// ---------------------------------------------------------------------------

fn visit_root(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // For complete units/programs, the root has children we iterate normally.
    // For .inc fragments, three error-recovery variants apply:
    //
    //   Root is ERROR (Variants A/C): dispatch the root so try_extract_error_type_decl
    //   can see all children (name-error + body children) in one call.
    //
    //   Root is 'root' with ERROR children (Variant B: interface/class where the
    //   type keyword is inline in the same ERROR as identifier+kEq): collect all
    //   root children and call try_extract_error_type_decl with the whole set so
    //   the body siblings are accessible for member extraction.
    if node.kind() == "ERROR" {
        dispatch(node, src, symbols, refs, None);
        return;
    }

    let mut cursor = node.walk();
    let root_children: Vec<Node> = node.children(&mut cursor).collect();

    // Detect Variant B at the root level: at least one ERROR child starts with
    // [identifier, kEq, type_keyword].  If found, group all children into a
    // virtual container for try_extract_error_type_decl.
    if try_extract_root_type_decls(&root_children, src, symbols, refs) {
        return;
    }

    for child in root_children {
        dispatch(child, src, symbols, refs, None);
    }
}

pub(super) fn dispatch(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "unit" => extract_unit(node, src, symbols, refs),
        "program" | "library" => extract_program(node, src, symbols, refs),
        "declProc" | "defProc" => extract_proc(node, src, symbols, refs, parent_index),
        // declType is the wrapper that carries the name for class/intf/enum/record bodies.
        "declType" => extract_decl_type(node, src, symbols, refs, parent_index),
        // declClass / declIntf dispatched directly (e.g. inside a unit body without declType)
        // are handled with name fallback via find_decl_type_name.
        "declClass" => extract_class(node, src, symbols, refs, parent_index, None),
        "declIntf" => extract_intf(node, src, symbols, refs, parent_index, None),
        "declSection" => extract_section(node, src, symbols, refs, parent_index),
        "declUses" => extract_uses(node, src, symbols, refs, parent_index),
        // declVars / declConsts — container nodes; dispatch each declVar / declConst child.
        "declVars" | "declConsts" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
        "declVar" => extract_var(node, src, symbols, refs, parent_index),
        "declConst" => {
            // Check whether error recovery folded a `TypeName = class(...)` declaration
            // into this constant node (the `type(typeref(Name)) defaultValue(= class(...))`
            // pattern produced when a generic class declaration follows an error-recovery
            // constant boundary).  When found, extract the embedded type declaration and
            // skip the spurious constant symbol.  Otherwise extract normally.
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            let embedded_type = children.windows(2).find(|w| {
                w[0].kind() == "type" && w[1].kind() == "defaultValue"
            });
            if let Some(w) = embedded_type {
                let type_node = w[0];
                let dv_node = w[1];
                if let Some(sym_kind) = infer_type_kind_from_default_value(dv_node, src) {
                    // Extract the name from the `type` node: it wraps a `typeref` which
                    // contains an `identifier`.
                    let name_opt = {
                        let mut tc = type_node.walk();
                        let type_children: Vec<Node> = type_node.children(&mut tc).collect();
                        type_children.iter().find_map(|c| {
                            if c.kind() == "typeref" {
                                let mut rc = c.walk();
                                let rc_ch: Vec<Node> = c.children(&mut rc).collect();
                                rc_ch.into_iter().find(|n| n.kind() == "identifier")
                            } else if c.kind() == "identifier" {
                                Some(*c)
                            } else {
                                None
                            }
                        })
                    };
                    if let Some(name_node) = name_opt {
                        let name = node_text(name_node, src);
                        if !name.is_empty() {
                            symbols.push(make_symbol(
                                name.clone(),
                                name,
                                sym_kind,
                                &type_node,
                                None,
                                parent_index,
                            ));
                            return;
                        }
                    }
                }
            }
            extract_const(node, src, symbols, refs, parent_index);
        }
        "exprCall" => {
            extract_call(node, src, refs, parent_index);
            // Recurse into arguments and nested sub-expressions so that
            // exprCall nodes inside arguments are also dispatched.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
        "typeref" => extract_typeref(node, src, refs, parent_index),
        "ERROR" => {
            // When a .inc file contains `TypeName = class ...` without the surrounding
            // `type` keyword, tree-sitter produces an ERROR node rather than declType →
            // declClass. Try to recover the declaration; fall back to generic recursion.
            if !try_extract_error_type_decl(node, src, symbols, refs, parent_index) {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    dispatch(child, src, symbols, refs, parent_index);
                }
            }
        }
        _ => {
            // Recurse into containers.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
    }
}




// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn find_identifier_child(node: Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "moduleName") {
            return Some(node_text(child, src));
        }
    }
    None
}

/// For type declarations (class, interface, record): the name is typically
/// the identifier child of the containing `type` block. Walk up one level
/// or look for a varDef / declType wrapping node.
/// Simplified: look for first identifier child of the node itself.
pub(super) fn find_decl_type_name(node: Node, src: &str) -> Option<String> {
    // Try named child "name" field first.
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, src));
    }
    find_identifier_child(node, src)
}

/// Dispatch a node that was identified as a type body in error-recovery context.
///
/// When tree-sitter wraps `TypeName = class ... end;` without the surrounding
/// `type` keyword, the class body ends up as a `declProc` whose first child is
/// `kClass`.  The body's contents are a mix of structured nodes (`declProc`,
/// `declSection`) and bare tokens (`kProcedure`, `identifier`, `;`) produced
/// by the grammar's error-recovery.  This function:
///
///   1. Skips the leading type keyword (kClass/kInterface/kRecord).
///   2. Dispatches structured children (declProc, declSection, etc.) normally.
///   3. For bare token sequences, scans for `kProcedure`/`kFunction` +
///      `identifier` patterns and emits a Function symbol for each.
pub(super) fn dispatch_type_body(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    if type_keyword_of_node(node, src).is_none() {
        dispatch(node, src, symbols, refs, parent_index);
        return;
    }

    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    let mut saw_type_keyword = false;
    let mut pending_proc_kw = false; // set after kProcedure/kFunction/etc.
    // Tracks whether the previous two tokens were `identifier kEq`, indicating
    // the start of a `TypeName = class/record` declaration embedded in the body
    // ERROR node by tree-sitter's error recovery.
    let mut pending_name_for_type: Option<Node> = None; // the identifier before kEq

    for (ci, child) in children.iter().enumerate() {
        match child.kind() {
            "kClass" | "kInterface" | "kRecord" if !saw_type_keyword => {
                saw_type_keyword = true; // skip the leading type keyword
                pending_name_for_type = None;
            }
            // When we see `kClass/kInterface/kRecord` after `identifier kEq`,
            // a new type declaration has been embedded in this body ERROR node.
            // Emit it as a Class/Interface/Struct and recurse for the rest.
            "kClass" | "kInterface" | "kRecord" if pending_name_for_type.is_some() => {
                let sym_kind = match child.kind() {
                    "kClass"     => SymbolKind::Class,
                    "kInterface" => SymbolKind::Interface,
                    _            => SymbolKind::Struct,
                };
                if let Some(name_node) = pending_name_for_type.take() {
                    let name = node_text(name_node, src);
                    if !name.is_empty() {
                        let new_idx = symbols.len();
                        symbols.push(make_symbol(
                            name.clone(),
                            name,
                            sym_kind,
                            child,
                            None,
                            parent_index,
                        ));
                        // Remaining children (from the type keyword onwards)
                        // belong to this new declaration.
                        recover_type_decls_from_siblings(
                            &children[ci..],
                            src,
                            symbols,
                            refs,
                            Some(new_idx),
                        );
                    }
                }
                return; // rest of children consumed by recover_type_decls_from_siblings
            }
            // `typeref` may be the name of a new type declaration when followed
            // by `defaultValue` (e.g. `TSFBool = class(TX3DSingleField)`).
            // In that case, intercept it here instead of dispatching as a ref.
            "typeref" if ci + 1 < children.len() && children[ci + 1].kind() == "defaultValue" => {
                if let Some(sym_kind) = infer_type_kind_from_default_value(children[ci + 1], src) {
                    let mut tc = child.walk();
                    let tc_ch: Vec<Node> = child.children(&mut tc).collect();
                    if let Some(name_node) = tc_ch.iter().find(|n| n.kind() == "identifier") {
                        let name = node_text(*name_node, src);
                        if !name.is_empty() {
                            let new_idx = symbols.len();
                            symbols.push(make_symbol(
                                name.clone(),
                                name,
                                sym_kind,
                                child,
                                None,
                                parent_index,
                            ));
                            // Remaining children from [ci+2] belong to this new declaration.
                            recover_type_decls_from_siblings(
                                &children[ci + 2..],
                                src,
                                symbols,
                                refs,
                                Some(new_idx),
                            );
                            return;
                        }
                    }
                }
                // Fallback: dispatch as a normal typeref.
                pending_proc_kw = false;
                pending_name_for_type = None;
                dispatch(*child, src, symbols, refs, parent_index);
            }
            // Structured children (including ERROR): dispatch normally.
            // ERROR nodes inside a type body may contain embedded type
            // declarations (e.g. class-of metaclass bodies that fold in the
            // following type declaration via error recovery).
            "declProc" | "defProc" | "declSection" | "declVars" | "declConsts"
            | "declUses" | "exprCall" | "typeref" | "ERROR" => {
                pending_proc_kw = false;
                pending_name_for_type = None;
                dispatch(*child, src, symbols, refs, parent_index);
            }
            "kProcedure" | "kFunction" | "kConstructor" | "kDestructor" | "kOperator" => {
                pending_proc_kw = true;
                pending_name_for_type = None;
            }
            "kEq" => {
                // `=` — if the previous child was an identifier, track it as
                // a potential type name (for `Name = class/record/interface`).
                if ci > 0 && children[ci - 1].kind() == "identifier" {
                    pending_name_for_type = Some(children[ci - 1]);
                } else {
                    pending_name_for_type = None;
                }
                pending_proc_kw = false;
            }
            "identifier" if pending_proc_kw => {
                // Bare `kProcedure identifier ;` inside the error-recovery body.
                let name = node_text(*child, src);
                if !name.is_empty() && name != "end" {
                    symbols.push(make_symbol(
                        name.clone(),
                        name,
                        SymbolKind::Function,
                        child,
                        None,
                        parent_index,
                    ));
                }
                pending_proc_kw = false;
                pending_name_for_type = None;
            }
            _ => {
                pending_proc_kw = false;
                pending_name_for_type = None;
            }
        }
    }
}

/// Returns the `SymbolKind` that corresponds to the type keyword that begins
/// this node (if any). Used to detect that a `declProc` or `declSection`
/// produced by error-recovery is actually a class/interface/record body.
///
/// The grammar wraps `TSoundAllocator = class ... end;` without the `type`
/// keyword into a `declProc` whose first non-pp child is `kClass`.
pub(super) fn type_keyword_of_node(node: Node, _src: &str) -> Option<SymbolKind> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let mut iter = children.iter().peekable();
    while let Some(child) = iter.next() {
        match child.kind() {
            "pp" | "comment" => continue,
            "kClass" => {
                // `class function`/`class procedure`/`class constructor`/`class destructor`
                // is a method modifier, not a type body opener.
                let next_is_method = iter.peek()
                    .map(|n| matches!(n.kind(), "kFunction" | "kProcedure" | "kConstructor" | "kDestructor"))
                    .unwrap_or(false);
                if next_is_method {
                    return None;
                }
                return Some(SymbolKind::Class);
            }
            "kInterface" => return Some(SymbolKind::Interface),
            "kRecord" => return Some(SymbolKind::Struct),
            _ => return None,
        }
    }
    None
}

/// Infer the SymbolKind from a `defaultValue` node that represents `= class(...)`.
///
/// Tree-sitter's error recovery for `.inc` fragments sometimes produces:
///
///   `identifier("TypeName")  defaultValue(kEq  exprCall(identifier("class") ...))`
///
/// or the equivalent with "record", "object", "interface" as the first identifier
/// in the `exprCall` child of `defaultValue`.
pub(super) fn infer_type_kind_from_default_value(node: Node, src: &str) -> Option<SymbolKind> {
    // node must be `defaultValue` or `kEq` — search for an identifier child
    // whose text is a Pascal type keyword.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "kClass"     => return Some(SymbolKind::Class),
            "kInterface" => return Some(SymbolKind::Interface),
            "kRecord"    => return Some(SymbolKind::Struct),
            "identifier" => {
                let text = node_text(child, src).to_ascii_lowercase();
                match text.as_str() {
                    "class" | "object" => return Some(SymbolKind::Class),
                    "interface"        => return Some(SymbolKind::Interface),
                    "record"           => return Some(SymbolKind::Struct),
                    _ => {}
                }
            }
            // Recurse into kEq, exprCall, and ERROR children.
            // ERROR wraps the class/record/interface keyword when tree-sitter's
            // error recovery groups the type opener with the surrounding context.
            "kEq" | "exprCall" | "ERROR" => {
                if let Some(k) = infer_type_kind_from_default_value(child, src) {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Infer the SymbolKind from a `kEq` node or an ERROR child whose first identifier
/// is a Pascal type keyword (for the case where `kEq` appears as a sibling in an
/// ERROR node's children: `ERROR { identifier "Name", kEq { identifier "class" } }`).
pub(super) fn infer_type_kind_from_eq_sibling(keq_node: Node, src: &str) -> Option<SymbolKind> {
    infer_type_kind_from_default_value(keq_node, src)
}

pub(super) fn has_keyword_child(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

pub(super) fn first_line_of(node: Node, src: &str) -> String {
    let text = node_text(node, src);
    text.lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

pub(super) fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}
}

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
