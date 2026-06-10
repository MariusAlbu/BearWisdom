// =============================================================================
// languages/pascal/error_recovery.rs — extract type decls from ERROR nodes
//
// Tree-sitter often emits ERROR nodes for complex/legacy Pascal; these
// helpers recover symbol declarations from sibling-textual analysis.
// =============================================================================

use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

use super::extract::{
    dispatch, dispatch_type_body, first_line_of, infer_type_kind_from_default_value,
    infer_type_kind_from_eq_sibling, make_symbol, node_text, type_keyword_of_node,
};

// ---------------------------------------------------------------------------
// Error-recovery helpers called from visit_root

/// Called when the tree root has kind 'root' (not ERROR). Checks if the root
/// children contain ERROR nodes that look like .inc-style type declarations.
/// Uses the same sibling-scan logic as try_extract_error_type_decl so that
/// multiple sequential declarations at the root level are all recovered.
///
/// Returns true when at least one type declaration was recovered, signalling
/// that normal child iteration should be skipped.
pub(super) fn try_extract_root_type_decls(
    root_children: &[Node],
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) -> bool {
    // Only engage if at least one root child looks like a name-error (ERROR
    // starting with identifier + kEq) so we don't accidentally eat valid code.
    let has_name_err = root_children.iter().any(|c| {
        if c.kind() != "ERROR" {
            return false;
        }
        let mut cc = c.walk();
        let ch: Vec<Node> = c.children(&mut cc).collect();
        ch.len() >= 2 && ch[0].kind() == "identifier" && ch[1].kind() == "kEq"
    });
    if !has_name_err {
        return false;
    }

    recover_type_decls_from_siblings(root_children, src, symbols, refs, None) > 0
}

// ---------------------------------------------------------------------------
// Error-recovery: TypeName = class/record/interface without 'type' keyword
//
// Pascal .inc files are fragments inside a `type` block of the parent .pas.
// Parsed standalone, tree-sitter generates ERROR nodes.  The recovery
// strategy scans a flat list of sibling nodes (children of the root ERROR or
// the root 'root' node) for consecutive [name-ERROR, type-body] pairs and
// emits one symbol per pair.
//
// Node shapes seen in castle-fresh (post-preprocessor-strip):
//
//  A — single class, no guard:
//    ERROR                     ← root
//      ERROR(ident, kEq)       ← name
//      declProc(kClass, ...)   ← body
//
//  B — single interface (kInterface inline in name-ERROR):
//    root                      ← root
//      ERROR(ident, kEq, kInterface)   ← name + keyword fused
//      ERROR(body...)          ← body
//
//  C — multiple forward decls inside {$ifdef}/{$endif}:
//    ERROR                     ← root
//      pp
//      ERROR(ident, kEq)       ← name TypeA
//      declProc(kClass ;)      ← body TypeA (no-body forward decl)
//      ERROR(ident, kEq)       ← name TypeB
//      declProc(kClass ;)      ← body TypeB
//      ...
//      pp
//
// All variants reduce to: scan `children` sequentially, detect name-errors
// (ERROR starting with [identifier, kEq]) and consume their following
// sibling as the body (which provides the type keyword).
// ---------------------------------------------------------------------------

/// Scan a slice of sibling AST nodes for type declaration patterns and emit
/// one symbol per declaration found.
///
/// Handles two sibling layouts produced by tree-sitter for .inc-style fragments:
///
///  Layout 1 — one name-error, one body node:
///    ERROR(ident kEq)  declProc(kClass body...)
///
///  Layout 2 — fused body: the body ERROR absorbs the next decl's name.
///  Arises for consecutive forward declarations where each `class;` is
///  swallowed into the same body ERROR as the subsequent name:
///    ERROR(ident_A kEq)
///    ERROR(kClass ";" ident_B kEq)
///    ERROR(kClass ";" ident_C kEq)
///    ERROR(kClass ";" pp)
///
/// Returns the number of type declarations recovered.
pub(super) fn recover_type_decls_from_siblings(
    siblings: &[Node],
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> usize {
    let mut count = 0;

    // Collect all tokens from the sibling list into a flat sequence so we can
    // slide a window over them.  Each (kind, node) pair is one token.
    // We process sequentially, tracking a pending name to be paired with
    // the next type keyword we encounter.
    let mut pending_name: Option<Node> = None;
    let mut pending_kind: Option<SymbolKind> = None;
    let mut body_children: Vec<Node> = Vec::new();

    // Flush any pending (name, kind) pair as a new symbol, recursing into body_children
    // for member extraction.  This is an inline macro-style block rather than a closure
    // to avoid multiple-mutable-borrow issues.
    macro_rules! emit_pending {
        ($anchor:expr) => {{
            if let (Some(nm), Some(kd)) = (pending_name.take(), pending_kind.take()) {
                let name = node_text(nm, src);
                if !name.is_empty() {
                    let sig = first_line_of($anchor, src);
                    let idx = symbols.len();
                    symbols.push(make_symbol(
                        name.clone(),
                        name,
                        kd,
                        &$anchor,
                        Some(sig),
                        parent_index,
                    ));
                    count += 1;
                    let drained: Vec<Node> = body_children.drain(..).collect();
                    // First pass: dispatch any nodes that are class/record/interface bodies.
                    // Second pass: run recover_type_decls_from_siblings on the collected body
                    // to pick up further type declarations embedded in the body (e.g. a class
                    // declaration whose name is a typeref followed by a defaultValue node).
                    let has_embedded = drained
                        .iter()
                        .any(|n| n.kind() == "typeref" || n.kind() == "identifier");
                    if has_embedded {
                        // Try sibling-scan first so nested type declarations are picked up.
                        let extra = recover_type_decls_from_siblings(
                            &drained,
                            src,
                            symbols,
                            refs,
                            Some(idx),
                        );
                        if extra == 0 {
                            // No nested declarations found; dispatch bodies individually.
                            for bc in &drained {
                                dispatch_type_body(*bc, src, symbols, refs, Some(idx));
                            }
                        }
                    } else {
                        for bc in &drained {
                            dispatch_type_body(*bc, src, symbols, refs, Some(idx));
                        }
                    }
                }
            }
            body_children.clear();
        }};
    }

    let mut si_outer = 0;
    while si_outer < siblings.len() {
        let sibling = &siblings[si_outer];
        si_outer += 1;

        if matches!(sibling.kind(), "pp" | "comment") {
            continue;
        }

        // Top-level `identifier` + `kEq` pattern: a bare identifier followed by
        // `=` in a flat sibling list (e.g. children of an ERROR node passed
        // directly to this function).  The kind is inferred from the sibling
        // after `kEq` (kClass/kInterface/kRecord or defaultValue).
        if sibling.kind() == "identifier"
            && si_outer < siblings.len()
            && siblings[si_outer].kind() == "kEq"
        {
            // Look past the kEq to find the type keyword.
            let kind_opt = if si_outer + 1 < siblings.len() {
                let after_eq = siblings[si_outer + 1];
                match after_eq.kind() {
                    "kClass" => Some(SymbolKind::Class),
                    "kInterface" => Some(SymbolKind::Interface),
                    "kRecord" => Some(SymbolKind::Struct),
                    "defaultValue" => infer_type_kind_from_default_value(after_eq, src),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(sym_kind) = kind_opt {
                emit_pending!(*sibling);
                pending_name = Some(*sibling);
                pending_kind = Some(sym_kind);
                si_outer += 2; // skip kEq and the type keyword/defaultValue
                for later in &siblings[si_outer..] {
                    body_children.push(*later);
                }
                si_outer = siblings.len();
                continue;
            }
        }

        // Top-level `typeref` + `defaultValue` pattern: the type name was parsed as a
        // typeref node in the sibling list itself (not embedded inside another node).
        if sibling.kind() == "typeref"
            && si_outer < siblings.len()
            && siblings[si_outer].kind() == "defaultValue"
        {
            if let Some(sym_kind) = infer_type_kind_from_default_value(siblings[si_outer], src) {
                let mut tc = sibling.walk();
                let tc_ch: Vec<Node> = sibling.children(&mut tc).collect();
                if let Some(name_node) = tc_ch.iter().find(|n| n.kind() == "identifier") {
                    emit_pending!(*sibling);
                    pending_name = Some(*name_node);
                    pending_kind = Some(sym_kind);
                    let dv_node = siblings[si_outer];
                    // The defaultValue was consumed; skip it.
                    si_outer += 1;
                    // Push the interior of the defaultValue (non-kEq children) into
                    // body_children so any nested type declarations inside it are
                    // processed (e.g. an exprBinary representing a new type decl
                    // that was folded into the defaultValue by error recovery).
                    let mut dvc = dv_node.walk();
                    for dv_child in dv_node.children(&mut dvc) {
                        if dv_child.kind() != "kEq" {
                            body_children.push(dv_child);
                        }
                    }
                    // Remaining siblings become body_children.
                    for later in &siblings[si_outer..] {
                        body_children.push(*later);
                    }
                    si_outer = siblings.len(); // consume all remaining
                    continue;
                }
            }
        }

        if sibling.kind() != "ERROR" {
            // `exprBinary` produced by error recovery for `TypeName = class(...)`.
            // Children: identifier, operator(=), then the class/interface/record body.
            if sibling.kind() == "exprBinary" {
                let mut eb = sibling.walk();
                let eb_ch: Vec<Node> = sibling.children(&mut eb).collect();
                if eb_ch.len() >= 3 {
                    let name_ident = eb_ch.iter().find(|n| n.kind() == "identifier").copied();
                    let has_eq = eb_ch.iter().any(|n| {
                        n.kind() == "kEq" || (n.kind() == "operator" && node_text(*n, src) == "=")
                    });
                    let kind_opt = eb_ch.iter().find_map(|n| match n.kind() {
                        "kClass" => Some(SymbolKind::Class),
                        "kInterface" => Some(SymbolKind::Interface),
                        "kRecord" => Some(SymbolKind::Struct),
                        "exprCall" | "ERROR" => infer_type_kind_from_default_value(*n, src),
                        _ => None,
                    });
                    if has_eq {
                        if let (Some(sym_kind), Some(name_node)) = (kind_opt, name_ident) {
                            emit_pending!(*sibling);
                            pending_name = Some(name_node);
                            pending_kind = Some(sym_kind);
                            continue;
                        }
                    }
                }
            }

            // Non-ERROR siblings: check if this is a type-body node (declProc/declSection
            // starting with kClass/kInterface/kRecord) when we have a pending name.
            // tree-sitter wraps the body of `TypeName = class ... end;` in a declProc
            // whose first child is kClass, with the full class body nested inside.
            if pending_name.is_some() {
                let type_kw = type_keyword_of_node(*sibling, src);
                if let Some(kd) = type_kw {
                    // This node IS the class/interface/record body.
                    if pending_kind.is_none() {
                        pending_kind = Some(kd);
                    }
                    body_children.push(*sibling);
                    continue;
                }
            }

            // Scan the direct children of this non-ERROR sibling for the pattern:
            //   `identifier("TypeName")  defaultValue(kEq exprCall("class"/"record"/...))`
            //
            // This is how tree-sitter's error recovery represents a new type declaration
            // embedded inside a `declProc` node belonging to the previous type's body.
            // When found: flush the current pending declaration and start a new one.
            {
                let mut sc = sibling.walk();
                let sib_children: Vec<Node> = sibling.children(&mut sc).collect();
                let mut found_embedded = false;
                for (si, sc_node) in sib_children.iter().enumerate() {
                    // Resolve the name node: either a plain `identifier` or a `typeref`
                    // wrapping a single identifier (tree-sitter may parse the class name
                    // as a type reference when it appears after a method signature tail).
                    let name_ident: Option<Node> = if sc_node.kind() == "identifier" {
                        Some(*sc_node)
                    } else if sc_node.kind() == "typeref" {
                        let mut tc = sc_node.walk();
                        let tc_children: Vec<Node> = sc_node.children(&mut tc).collect();
                        tc_children.into_iter().find(|n| n.kind() == "identifier")
                    } else {
                        None
                    };

                    if let Some(name_node) = name_ident {
                        if si + 1 < sib_children.len() {
                            let next = sib_children[si + 1];
                            // Look for `defaultValue` sibling that represents `= class(...)`.
                            if next.kind() == "defaultValue" {
                                if let Some(sym_kind) =
                                    infer_type_kind_from_default_value(next, src)
                                {
                                    emit_pending!(*sibling);
                                    pending_name = Some(name_node);
                                    pending_kind = Some(sym_kind);
                                    for later in sib_children.iter().skip(si + 2) {
                                        body_children.push(*later);
                                    }
                                    found_embedded = true;
                                    break;
                                }
                            }
                        }
                    }
                    // When a child is an ERROR, its LAST identifier may be the name of the
                    // next type declaration.  Two sub-patterns:
                    //
                    //  a) ERROR + defaultValue: the `= class(...)` form.
                    //  b) ERROR + kClass/kInterface/kRecord: the bare `class(ParentType)`
                    //     form, where the type keyword stands alone as the next token.
                    if sc_node.kind() == "ERROR" && si + 1 < sib_children.len() {
                        let next = sib_children[si + 1];
                        let sym_kind_opt: Option<SymbolKind> = if next.kind() == "defaultValue" {
                            infer_type_kind_from_default_value(next, src)
                        } else {
                            match next.kind() {
                                "kClass" => Some(SymbolKind::Class),
                                "kInterface" => Some(SymbolKind::Interface),
                                "kRecord" => Some(SymbolKind::Struct),
                                _ => None,
                            }
                        };
                        if let Some(sym_kind) = sym_kind_opt {
                            // Grab the last identifier child of the ERROR as the type name.
                            let mut ec = sc_node.walk();
                            let err_ch: Vec<Node> = sc_node.children(&mut ec).collect();
                            let last_ident = err_ch.iter().rev().find(|n| n.kind() == "identifier");
                            if let Some(name_node) = last_ident {
                                emit_pending!(*sibling);
                                pending_name = Some(*name_node);
                                pending_kind = Some(sym_kind);
                                // Skip past the ERROR and the type keyword/defaultValue;
                                // subsequent children are the body of this new type.
                                let skip = if next.kind() == "defaultValue" {
                                    si + 2
                                } else {
                                    si + 2
                                };
                                for later in sib_children.iter().skip(skip) {
                                    body_children.push(*later);
                                }
                                found_embedded = true;
                                break;
                            }
                        }
                    }
                    // Skip `kClass`/`kInterface`/`kRecord` when it is a method modifier
                    // (`class function`/`class procedure` etc.).  Otherwise stop scanning —
                    // we've reached a type body that belongs to a pending declaration and
                    // no further embedded names follow (they'd have been handled above).
                    if matches!(sc_node.kind(), "kClass" | "kInterface" | "kRecord") {
                        let next_is_method_kw = sib_children
                            .get(si + 1)
                            .map(|n| {
                                matches!(
                                    n.kind(),
                                    "kFunction" | "kProcedure" | "kConstructor" | "kDestructor"
                                )
                            })
                            .unwrap_or(false);
                        if !next_is_method_kw {
                            break;
                        }
                    }
                }
                if found_embedded {
                    continue;
                }
            }

            // No pending name, or not a type-body node → dispatch generically.
            if pending_name.is_some() && pending_kind.is_some() {
                body_children.push(*sibling);
            } else {
                dispatch(*sibling, src, symbols, refs, parent_index);
            }
            continue;
        }

        // Walk the children of this ERROR node token by token.
        let mut ec = sibling.walk();
        let err_children: Vec<Node> = sibling.children(&mut ec).collect();
        let mut j = 0;
        while j < err_children.len() {
            let tok = err_children[j];
            match tok.kind() {
                "identifier" => {
                    // If followed by kEq, this is a type name.
                    if j + 1 < err_children.len() && err_children[j + 1].kind() == "kEq" {
                        // Flush any pending declaration first.
                        emit_pending!(*sibling);
                        pending_name = Some(tok);
                        // Check if the type kind is embedded in kEq's children.
                        let keq_node = err_children[j + 1];
                        if let Some(kind) = infer_type_kind_from_eq_sibling(keq_node, src) {
                            pending_kind = Some(kind);
                        }
                        j += 2; // skip identifier and kEq
                        continue;
                    }
                    // If followed by a `defaultValue` node (alternative error-recovery form),
                    // check if it represents `= class(...)`.
                    if j + 1 < err_children.len() && err_children[j + 1].kind() == "defaultValue" {
                        if let Some(sym_kind) =
                            infer_type_kind_from_default_value(err_children[j + 1], src)
                        {
                            emit_pending!(*sibling);
                            pending_name = Some(tok);
                            pending_kind = Some(sym_kind);
                            j += 2; // skip identifier and defaultValue
                            continue;
                        }
                    }
                    // Not a name → body content.
                    if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "kClass" => {
                    if pending_name.is_some() && pending_kind.is_none() {
                        pending_kind = Some(SymbolKind::Class);
                    } else if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "kInterface" => {
                    if pending_name.is_some() && pending_kind.is_none() {
                        pending_kind = Some(SymbolKind::Interface);
                    } else if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "kRecord" => {
                    if pending_name.is_some() && pending_kind.is_none() {
                        pending_kind = Some(SymbolKind::Struct);
                    } else if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "pp" | "comment" => {} // skip preprocessor/comments in body
                _ => {
                    if pending_name.is_some() {
                        // When we have a name but no kind yet, try to infer the kind
                        // from the node.  `declClass` and `declIntf` are produced when
                        // tree-sitter successfully parses the class body after error
                        // recovery strips the generic params from the parent type.
                        if pending_kind.is_none() {
                            if let Some(kd) = type_keyword_of_node(tok, src) {
                                pending_kind = Some(kd);
                            } else if tok.kind() == "declClass" {
                                pending_kind = Some(SymbolKind::Class);
                            } else if tok.kind() == "declIntf" {
                                pending_kind = Some(SymbolKind::Interface);
                            }
                        }
                        body_children.push(tok);
                    }
                }
            }
            j += 1;
        }

        // Non-token children (e.g. typeref, declSection) that are not leaves.
        // Already handled above by the general arm.
    }

    // Flush any remaining pending declaration.
    let anchor_dummy = if let Some(last) = siblings.last() {
        *last
    } else {
        return count;
    };
    emit_pending!(anchor_dummy);

    count
}

/// Attempt to recover type declaration(s) from an ERROR node produced when
/// `TypeName = class/interface/record ...` appears without the surrounding
/// `type` keyword (typical in Pascal .inc fragment files).
///
/// Returns `true` when at least one symbol was recovered.
pub(super) fn try_extract_error_type_decl(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    recover_type_decls_from_siblings(&children, src, symbols, refs, parent_index) > 0
}
