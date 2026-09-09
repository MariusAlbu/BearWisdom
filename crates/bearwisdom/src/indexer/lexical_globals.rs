//! Profile-attested global declaration capture; no workspace/name-based merging.
use super::modules::scopes::Kind as ModuleKind;
use super::{BindingId, LexicalBindings, LexicalSyntax, NameId};
use crate::indexer::namespaces::SourceModuleId;
use crate::types::{ExtractedSymbol, SourceSpan, SymbolKind};
use std::collections::HashMap;
use tree_sitter::Node;
type Anchors = HashMap<(u32, u32, SymbolKind), Vec<usize>>;

#[path = "lexical_private_members.rs"]
pub(super) mod private_members;

#[path = "lexical_member_surface.rs"]
pub(crate) mod member_surface;

#[path = "lexical_call_arguments.rs"]
pub(crate) mod call_arguments;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum InheritedMembers {
    #[default]
    Unordered,
    DeclarationOrder,
}

pub(crate) struct Forms {
    pub inherited_members: InheritedMembers,
    pub surface: &'static member_surface::Forms,
    pub names: &'static [&'static str],
    pub member_names: &'static [&'static str],
    pub private_member: (&'static str, &'static str),
    pub declaration_wrappers: &'static [&'static str],
    pub augmentation: (&'static str, &'static str, &'static str),
    pub ignored_roots: &'static [&'static str],
    pub unsupported_named: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub(crate) struct Declaration {
    pub unit: SourceModuleId,
    pub name: NameId,
    pub binding: Option<BindingId>,
    pub slot: Option<usize>,
    pub kind: SymbolKind,
    pub type_space: bool,
    pub parameters: Vec<NameId>,
    pub plain_parameters: bool,
    pub members: Vec<NameId>,
    pub plain_merge: bool,
    pub plain_header: bool,
    pub surface: Option<Vec<member_surface::Member>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Capture {
    pub calls: call_arguments::Capture,
    pub inherited_members: InheritedMembers,
    pub values: HashMap<u32, NameId>,
    pub arguments: HashMap<SourceSpan, NameId>,
    pub selectors: HashMap<u32, NameId>,
    pub private_selectors: HashMap<u32, Option<usize>>,
    pub isolated: bool,
    pub complete: bool,
    pub roots: Vec<Declaration>,
    pub augmentations: Vec<Declaration>,
    pub interfaces: Vec<Declaration>,
    pub classes: Vec<(Declaration, bool, bool)>,
    pub merge_rules: Vec<(SymbolKind, SymbolKind)>,
}

pub(super) fn capture(
    root: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    symbols: &[ExtractedSymbol],
    graph: &mut LexicalBindings,
) -> Capture {
    let mut cursor = root.walk();
    let isolated = root
        .named_children(&mut cursor)
        .any(|n| n.kind() == syntax.modules.import || n.kind() == syntax.modules.export);
    let mut result = Capture {
        isolated,
        complete: !root.has_error(),
        merge_rules: syntax.merge_declarations.to_vec(),
        inherited_members: syntax.globals.inherited_members,
        ..Default::default()
    };
    let mut anchors = Anchors::new();
    for (slot, symbol) in symbols.iter().enumerate() {
        anchors
            .entry((symbol.start_line, symbol.start_col, symbol.kind))
            .or_default()
            .push(slot);
    }
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if super::modules::scopes::kind(node, syntax.modules) == Some(ModuleKind::Augmentation) {
            continue;
        }
        if !isolated {
            collect(
                node,
                source,
                syntax,
                &anchors,
                graph,
                &mut result.roots,
                &mut result.complete,
            );
        }
    }
    let units = graph.module.units.clone();
    for unit in &units {
        if unit.kind == ModuleKind::Literal {
            result.complete &= unit.complete && unit.container_valid && unit.parent.0 == 0;
        }
        if unit.kind != ModuleKind::Augmentation {
            continue;
        }
        let container = unit.parent.0 == 0
            || units.iter().any(|parent| {
                parent.id == unit.parent
                    && parent.kind == ModuleKind::Literal
                    && parent.parent.0 == 0
                    && parent.container_valid
            });
        if !unit.complete || !unit.container_valid || !container {
            result.complete = false;
            continue;
        }
        let body = root
            .named_descendant_for_byte_range(unit.body.start as usize, unit.body.end as usize)
            .filter(|node| node.kind() == syntax.globals.augmentation.2);
        let Some(body) = body else {
            result.complete = false;
            continue;
        };
        let begin = result.augmentations.len();
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            collect(
                child,
                source,
                syntax,
                &anchors,
                graph,
                &mut result.augmentations,
                &mut result.complete,
            );
        }
        for part in &mut result.augmentations[begin..] {
            part.unit = unit.id;
        }
    }
    collect_interfaces(
        root,
        source,
        syntax,
        &anchors,
        graph,
        &mut result.interfaces,
    );
    collect_classes(root, source, syntax, &anchors, graph, &mut result.classes);
    call_arguments::capture(root, source, syntax, &mut result.calls);
    call_arguments::bind_reads(&result.calls, source, graph, &mut result.arguments);
    result
}

fn collect_interfaces(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    anchors: &Anchors,
    graph: &mut LexicalBindings,
    output: &mut Vec<Declaration>,
) {
    if syntax.type_bases.0.contains(&node.kind()) {
        collect(node, source, syntax, anchors, graph, output, &mut true);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_interfaces(child, source, syntax, anchors, graph, output);
    }
}

fn collect_classes(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    anchors: &Anchors,
    graph: &mut LexicalBindings,
    output: &mut Vec<(Declaration, bool, bool)>,
) {
    if syntax.base_types.0.contains(&node.kind()) {
        let mut parts = Vec::new();
        collect(node, source, syntax, anchors, graph, &mut parts, &mut true);
        let mut cursor = node.walk();
        let abstract_ = node.children(&mut cursor).any(|child| {
            syntax
                .globals
                .surface
                .modifiers
                .iter()
                .any(|&(token, modifier)| {
                    modifier == member_surface::Modifier::Abstract && child.kind() == token
                })
        });
        let mut cursor = node.walk();
        let has_base = node
            .named_children(&mut cursor)
            .filter(|n| n.kind() == syntax.base_types.1)
            .any(|heritage| {
                let mut cursor = heritage.walk();
                let found = heritage
                    .named_children(&mut cursor)
                    .any(|n| n.kind() == syntax.base_types.2);
                found
            });
        output.extend(
            parts
                .into_iter()
                .filter(|part| part.kind == SymbolKind::Class && part.type_space)
                .map(|part| (part, abstract_, has_base)),
        );
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_classes(child, source, syntax, anchors, graph, output);
    }
}

fn collect(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    anchors: &Anchors,
    graph: &mut LexicalBindings,
    output: &mut Vec<Declaration>,
    complete: &mut bool,
) {
    if super::modules::scopes::kind(node, syntax.modules) == Some(ModuleKind::Literal) {
        *complete &= graph.module.units.iter().any(|unit| {
            unit.kind == ModuleKind::Literal
                && unit.range.start == node.start_byte() as u32
                && unit.range.end == node.end_byte() as u32
                && unit.complete
                && unit.container_valid
                && unit.parent.0 == 0
        });
        return;
    }
    if syntax.globals.declaration_wrappers.contains(&node.kind()) {
        let mut cursor = node.walk();
        for child in node
            .named_children(&mut cursor)
            .filter(|n| syntax.globals.unsupported_named.contains(&n.kind()))
        {
            collect(child, source, syntax, anchors, graph, output, complete);
        }
        return;
    }
    if syntax.globals.ignored_roots.contains(&node.kind()) || node.is_extra() {
        return;
    }
    if syntax.modules.declaration_lists.contains(&node.kind()) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect(child, source, syntax, anchors, graph, output, complete);
        }
        return;
    }
    let kind = syntax
        .named_declarations
        .iter()
        .chain(syntax.type_declarations)
        .find(|&&(kind, _)| kind == node.kind())
        .map(|&(_, kind)| kind)
        .or_else(|| {
            syntax
                .variables
                .contains(&node.kind())
                .then_some(SymbolKind::Variable)
        });
    if kind.is_none() && !syntax.globals.unsupported_named.contains(&node.kind()) {
        *complete = false;
        return;
    }
    let Some(name) = node.child_by_field_name("name") else {
        *complete = false;
        return;
    };
    let Ok(spelling) = name.utf8_text(source) else {
        *complete = false;
        return;
    };
    if !syntax.globals.names.contains(&name.kind()) {
        *complete = false;
        return;
    }
    let name_id = graph.intern(spelling);
    let binding = graph
        .declarations
        .get(&SourceSpan {
            start: name.start_byte() as u32,
            end: name.end_byte() as u32,
        })
        .copied();
    let mut parameters = Vec::new();
    let mut plain_parameters = true;
    if let Some(params) = node.child_by_field_name("type_parameters") {
        let mut cursor = params.walk();
        for param in params.named_children(&mut cursor).filter(|n| !n.is_extra()) {
            let Some(name) = param.child_by_field_name("name") else {
                plain_parameters = false;
                continue;
            };
            if let Ok(name) = name.utf8_text(source) {
                parameters.push(graph.intern(name));
            } else {
                plain_parameters = false;
            }
            plain_parameters &= param.named_child_count() == 1;
        }
    }
    let supported = kind.is_some();
    let (members, plain_merge) = member_surface(node, source, syntax, graph);
    let surface = member_surface::capture(node, source, syntax, anchors, graph);
    let kind = kind.unwrap_or(SymbolKind::Namespace);
    // Match the existing lexical symbol bridge: named declarations use the
    // declaration node; variable patterns use their identifier token.
    let point = if syntax.variables.contains(&node.kind()) {
        name.start_position()
    } else {
        super::declaration_sites::start(node, syntax)
    };
    let slot = match anchors
        .get(&(point.row as u32, point.column as u32, kind))
        .map(Vec::as_slice)
    {
        Some([slot]) if supported => Some(*slot),
        _ => None,
    };
    let type_only = syntax.type_declarations.iter().any(|&(_, k)| k == kind);
    let dual = syntax.dual_declarations.contains(&kind)
        || syntax.globals.unsupported_named.contains(&node.kind());
    for type_space in [false, true] {
        if (type_only && !type_space) || (!type_only && !dual && type_space) {
            continue;
        }
        let binding = binding.map(|b| {
            if type_space {
                graph.dual_types.get(&b).copied().unwrap_or(b)
            } else {
                b
            }
        });
        output.push(Declaration {
            unit: SourceModuleId(0),
            name: name_id,
            binding,
            slot,
            kind,
            type_space,
            parameters: parameters.clone(),
            plain_parameters,
            members: members.clone(),
            plain_merge,
            plain_header: plain_header(node),
            surface: surface.clone(),
        });
    }
}

#[cfg(test)]
#[path = "lexical_globals_tests.rs"]
mod tests;

fn member_surface(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
) -> (Vec<NameId>, bool) {
    let Some(body) = node.child_by_field_name("body") else {
        return (vec![], false);
    };
    // Base clauses and other modifiers require semantic compatibility evidence.
    let mut complete = plain_header(node);
    let mut members = Vec::new();
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor).filter(|n| !n.is_extra()) {
        // An index parameter's identifier is not a property key. Keep the
        // compatibility barrier until index domains/types are checked by IDs.
        if syntax
            .globals
            .surface
            .kinds
            .iter()
            .any(|&(form, kind)| form == member.kind() && kind == member_surface::Kind::Index)
        {
            complete = false;
            continue;
        }
        let Some(name) = member.child_by_field_name("name") else {
            complete = false;
            continue;
        };
        if !syntax.globals.member_names.contains(&name.kind()) {
            complete = false;
            continue;
        }
        if let Ok(name) = name.utf8_text(source) {
            members.push(graph.intern(name));
        } else {
            complete = false;
        }
    }
    (members, complete)
}

fn plain_header(node: Node) -> bool {
    let fields = ["name", "type_parameters", "body"].map(|field| node.child_by_field_name(field));
    let mut cursor = node.walk();
    let complete = node
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra())
        .all(|n| fields.contains(&Some(n)));
    complete
}

fn nested_augmentation(node: Node, syntax: &LexicalSyntax) -> bool {
    let (kind, token, body) = syntax.globals.augmentation;
    let mut cursor = node.walk();
    if node.kind() == kind && node.children(&mut cursor).any(|n| n.kind() == token) {
        return true;
    }
    let container = syntax.globals.unsupported_named.contains(&node.kind())
        || syntax.globals.declaration_wrappers.contains(&node.kind())
        || syntax.modules.declaration_lists.contains(&node.kind())
        || node.kind() == syntax.modules.export
        || node.kind() == body;
    if !container {
        return false;
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .any(|n| nested_augmentation(n, syntax));
    found
}
