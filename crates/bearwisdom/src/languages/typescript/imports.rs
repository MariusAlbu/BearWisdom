use super::helpers::node_text;
use crate::ecosystem::ecmascript_imports::{push_import_refs, PushImportOpts};
use crate::types::{EdgeKind, ExtractedRef};
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
