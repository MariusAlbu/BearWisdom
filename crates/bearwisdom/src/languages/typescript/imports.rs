use super::helpers::node_text;
use crate::ecosystem::ecmascript_imports::{push_import_refs, PushImportOpts};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    push_import_refs(
        node,
        src,
        current_symbol_count,
        refs,
        PushImportOpts::TYPESCRIPT,
    );
}

/// `import * as ns from 'm'` binds `ns` as a module-namespace alias, not a value.
/// Emit a Namespace symbol so a consumer chain `ns.member` roots on a namespace
/// (the resolver's namespace gate) and falls through to the module-scoped
/// bare-name ladder, which binds the member as an export of `m`. Without it `ns`
/// value-types to a foreign same-name binding and `ns.member` is a hard miss.
/// Mirrors the `export * as ns` re-export case in `reexports.rs`.
pub(super) fn push_namespace_import_symbol(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let Some(clause) = first_child_of_kind(node, "import_clause") else {
        return;
    };
    let Some(ns) = first_child_of_kind(&clause, "namespace_import") else {
        return;
    };
    let Some(ident) = first_child_of_kind(&ns, "identifier") else {
        return;
    };
    let name = node_text(ident, src);
    if name.is_empty() || symbols.iter().any(|s| s.qualified_name == name) {
        return;
    }
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: ident.start_position().row as u32 + 1,
        end_line: ident.end_position().row as u32 + 1,
        start_col: ident.start_position().column as u32,
        end_col: ident.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: ident.start_byte() as u32,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
}

fn first_child_of_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(ch) = node.child(i) {
            if ch.kind() == kind {
                return Some(ch);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Heritage clause (extends / implements)
// ---------------------------------------------------------------------------

pub(super) fn extract_heritage(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_heritage" => {
                let mut hc = child.walk();
                for clause in child.children(&mut hc) {
                    match clause.kind() {
                        "extends_clause" => {
                            push_heritage_refs(&clause, src, source_idx, EdgeKind::Inherits, refs)
                        }
                        "implements_clause" => {
                            push_heritage_refs(&clause, src, source_idx, EdgeKind::Implements, refs)
                        }
                        _ => {}
                    }
                }
            }
            // Direct `extends_clause` child for interfaces; `extends_type_clause`
            // is the TS grammar node for `interface B extends A, C`.
            "extends_clause" | "extends_type_clause" => {
                push_heritage_refs(&child, src, source_idx, EdgeKind::Inherits, refs)
            }
            _ => {}
        }
    }
}

/// Emit one heritage ref per base type named in `clause`, pairing each base
/// identifier with a following `type_arguments` node so a generic parent
/// (`Repository<User>`) carries its arguments in `target_name`. The engine
/// decomposes that string into the parent's base class + bound args, which is
/// what lets an inherited generic method (`find_one(): T`) resolve `T` to the
/// concrete argument. A non-generic parent yields the bare name unchanged.
fn push_heritage_refs(
    clause: &Node,
    src: &[u8],
    source_idx: usize,
    kind: EdgeKind,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut c = clause.walk();
    let children: Vec<Node> = clause.children(&mut c).collect();
    let mut i = 0;
    while i < children.len() {
        let n = children[i];
        // The base type named by this supertype and the args to fold into the
        // target name. A class `extends_clause` lays the base and its generic
        // arguments out as flat siblings (`Repository`, `<User>`); an interface
        // `extends_type_clause` wraps a generic supertype in a `generic_type`
        // node (base under field `name`, args under field `type_arguments`) and
        // a qualified supertype in a `nested_type_identifier` (`Chai.Assertion`).
        // All three forms reduce to one Inherits ref carrying `Head<args>`.
        let (base, args) = match n.kind() {
            "identifier" | "type_identifier" | "nested_type_identifier" => {
                let next_args = children
                    .get(i + 1)
                    .filter(|next| next.kind() == "type_arguments")
                    .copied();
                if next_args.is_some() {
                    i += 1;
                }
                (Some(n), next_args)
            }
            "generic_type" => (
                n.child_by_field_name("name"),
                n.child_by_field_name("type_arguments"),
            ),
            _ => (None, None),
        };
        let Some(base) = base else {
            i += 1;
            continue;
        };
        let mut target = node_text(base, src);
        if let Some(args) = args {
            target.push_str(&node_text(args, src));
        }
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: source_idx,
            target_name: target,
            kind,
            line: base.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: base.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
        i += 1;
    }
}
