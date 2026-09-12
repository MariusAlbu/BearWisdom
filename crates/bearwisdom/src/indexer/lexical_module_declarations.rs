//! Exports introduced by a declaration statement: a namespace unit, an
//! import alias, or a named declaration, under `export` / `export default`.
use super::{
    local_target, scopes, text, Export, ExportTarget, LexicalBindings, ModuleForms, ModuleSyntax,
};
use tree_sitter::Node;

pub(super) fn declaration_exports(
    node: Node,
    source: &[u8],
    forms: &ModuleForms,
    graph: &LexicalBindings,
    output: &mut ModuleSyntax,
    default: bool,
    type_only: bool,
    units: &[(scopes::Unit, Node)],
) {
    if node.kind() == forms.import_alias {
        if let Some(name) = (forms.first_named_child)(node).and_then(|n| text(n, source, forms)) {
            output.exports.push(Export {
                target: local_target(graph, graph.name_id(&name), node.start_byte() as u32),
                name,
                type_only,
            });
        } else {
            output.complete = false;
        }
        return;
    }
    let module_kind = scopes::kind(node, forms);
    if let Some((unit, _)) = units.iter().find(|(unit, _)| {
        module_kind == Some(unit.kind)
            && unit.range.start == node.start_byte() as u32
            && unit.range.end == node.end_byte() as u32
    }) {
        if unit.kind == scopes::Kind::Namespace {
            if let Some(name) = node
                .child_by_field_name(forms.declaration_name_field)
                .filter(|n| forms.identifier_names.contains(&n.kind()))
                .and_then(|n| text(n, source, forms))
            {
                output.exports.push(Export {
                    name,
                    target: ExportTarget::Module(unit.id),
                    type_only,
                });
            } else {
                output.complete = false;
            }
        }
        return;
    }
    if forms.declaration_wrappers.contains(&node.kind()) {
        let mut cursor = node.walk();
        let children: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .collect();
        if let [child] = children.as_slice() {
            if scopes::kind(*child, forms).is_some() {
                declaration_exports(
                    *child, source, forms, graph, output, default, type_only, units,
                );
                return;
            }
        }
        output.complete = false;
        return;
    }
    if let Some(name) = node.child_by_field_name(forms.declaration_name_field) {
        if let Some(local) = text(name, source, forms) {
            let target = local_target(graph, graph.name_id(&local), name.start_byte() as u32);
            output.exports.push(Export {
                name: if default {
                    forms.default_export_name.into()
                } else {
                    local
                },
                target,
                type_only,
            });
        }
        return;
    }
    if !forms.declaration_lists.contains(&node.kind()) {
        if default {
            output.exports.push(Export {
                name: forms.default_export_name.into(),
                target: ExportTarget::Unknown,
                type_only,
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        declaration_exports(
            child, source, forms, graph, output, default, type_only, units,
        );
    }
}
