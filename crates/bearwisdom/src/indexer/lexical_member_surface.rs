//! Source member identities, including signatures with no navigation row.
//! Captured syntax is evidence, never a merge-compatibility certificate.
use super::{Anchors, BindingId, LexicalBindings, LexicalSyntax, NameId};
use crate::types::{SourceSpan, SymbolKind};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Property,
    Method,
    Getter,
    Setter,
    Call,
    Construct,
    Index,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Modifier {
    Readonly,
    Optional,
    Static,
    Abstract,
    Public,
    Protected,
    Private,
    Override,
    Declare,
}
pub(crate) struct Forms {
    pub overload_order: OverloadOrder,
    pub specialized_parameter_types: &'static [&'static str],
    pub kinds: &'static [(&'static str, Kind)],
    pub modifiers: &'static [(&'static str, Modifier)],
    pub accessor_tokens: (&'static str, &'static str),
    pub constructor_name: &'static str,
    pub modifier_wrappers: &'static [&'static str],
    pub computed: &'static str,
    pub literals: &'static [&'static str],
    pub optional_parameter: &'static str,
    pub rest_pattern: &'static str,
    pub receiver_parameter: (&'static str, &'static str),
    pub type_annotations: &'static [&'static str],
    pub unique_type: &'static [&'static str],
    pub erased_containers: &'static [&'static str],
    pub erased_modifiers: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ValueUse {
    #[default]
    Runtime,
    Erased,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Root<B> {
    Binding(B),
    Global(NameId),
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Key<B> {
    Named(NameId),
    Private(NameId),
    Literal(SourceSpan),
    Computed {
        expression: SourceSpan,
        root: Root<B>,
        selectors: Vec<NameId>,
        #[serde(default)]
        usage: ValueUse,
    },
    Call,
    Construct,
    Index,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Parameter {
    #[serde(default)]
    pub receiver: bool,
    pub span: SourceSpan,
    pub type_span: Option<SourceSpan>,
    pub optional: bool,
    pub rest: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Signature {
    #[serde(default)]
    pub type_parameters_complete: bool,
    #[serde(default)]
    pub ordering: Option<SignatureOrder>,
    #[serde(default)]
    pub body: Option<SourceSpan>,
    pub type_parameters: Vec<SourceSpan>,
    pub parameters: Vec<Parameter>,
    pub result: Option<SourceSpan>,
    pub initializer: Option<SourceSpan>,
    pub unique_symbol: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OverloadOrder {
    MergedGroupsLiteralFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SignatureOrder {
    pub group: SourceSpan,
    pub policy: OverloadOrder,
    pub specialized: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Member<D = usize, B = BindingId> {
    pub span: SourceSpan,
    pub key_span: Option<SourceSpan>,
    pub kind: Kind,
    pub key: Key<B>,
    pub modifiers: Vec<Modifier>,
    pub signature: Signature,
    pub slot: Option<D>,
}

pub(super) fn capture(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    anchors: &Anchors,
    graph: &mut LexicalBindings,
) -> Option<Vec<Member>> {
    let owner = syntax
        .named_declarations
        .iter()
        .chain(syntax.type_declarations)
        .find(|&&(form, _)| form == node.kind())
        .map(|&(_, kind)| kind);
    if !matches!(owner, Some(SymbolKind::Class | SymbolKind::Interface)) {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let mut cursor = body.walk();
    Some(
        body.named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|member| {
                member_input(
                    member,
                    owner == Some(SymbolKind::Class),
                    source,
                    syntax,
                    anchors,
                    graph,
                )
            })
            .collect(),
    )
}

fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

fn member_input(
    node: Node,
    class_owner: bool,
    source: &[u8],
    syntax: &LexicalSyntax,
    anchors: &Anchors,
    graph: &mut LexicalBindings,
) -> Member {
    let forms = syntax.globals.surface;
    let mut kind = forms
        .kinds
        .iter()
        .find(|&&(form, _)| form == node.kind())
        .map(|&(_, kind)| kind)
        .unwrap_or(Kind::Unknown);
    let name = node.child_by_field_name("name");
    let mut cursor = node.walk();
    let tokens: Vec<_> = node
        .children(&mut cursor)
        .filter(|n| !n.is_named())
        .map(|n| n.kind())
        .collect();
    if kind == Kind::Method {
        if tokens.contains(&forms.accessor_tokens.0) {
            kind = Kind::Getter;
        } else if tokens.contains(&forms.accessor_tokens.1) {
            kind = Kind::Setter;
        } else if class_owner
            && !forms
                .modifiers
                .iter()
                .any(|&(token, modifier)| modifier == Modifier::Static && tokens.contains(&token))
            && name.is_some_and(|n| {
                syntax.globals.member_names.contains(&n.kind())
                    && n.utf8_text(source).ok() == Some(forms.constructor_name)
            })
        {
            kind = Kind::Construct;
        }
    }
    let key = match kind {
        Kind::Call => Key::Call,
        Kind::Construct => Key::Construct,
        Kind::Index => Key::Index,
        _ => name
            .map(|n| key(n, source, syntax, graph))
            .unwrap_or(Key::Unknown),
    };
    let mut modifiers = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let token = if !child.is_named() {
            Some(child.kind())
        } else if forms.modifier_wrappers.contains(&child.kind()) {
            child.utf8_text(source).ok()
        } else {
            None
        };
        if let Some((_, modifier)) = forms
            .modifiers
            .iter()
            .find(|&&(kind, _)| Some(kind) == token)
        {
            modifiers.push(*modifier);
        }
    }
    let point = node.start_position();
    let mut rows = [
        SymbolKind::Method,
        SymbolKind::Property,
        SymbolKind::Field,
        SymbolKind::Constructor,
        SymbolKind::Function,
    ]
    .into_iter()
    .filter_map(|kind| anchors.get(&(point.row as u32, point.column as u32, kind)))
    .flatten()
    .copied();
    let first = rows.next();
    let slot = first.filter(|_| rows.next().is_none());
    Member {
        span: span(node),
        key_span: if matches!(kind, Kind::Index | Kind::Construct) {
            None
        } else {
            name.map(span)
        },
        kind,
        key,
        modifiers,
        signature: signature(node, kind, forms),
        slot,
    }
}

pub(crate) fn key(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
) -> Key<BindingId> {
    if syntax.globals.member_names.contains(&node.kind())
        || node.kind() == syntax.globals.private_member.0
    {
        let Ok(spelling) = node.utf8_text(source) else {
            return Key::Unknown;
        };
        let name = graph.intern(spelling);
        return if node.kind() == syntax.globals.private_member.0 {
            Key::Private(name)
        } else {
            Key::Named(name)
        };
    }
    if syntax.globals.surface.literals.contains(&node.kind()) {
        return Key::Literal(span(node));
    }
    if node.kind() != syntax.globals.surface.computed {
        return Key::Unknown;
    }
    let mut cursor = node.walk();
    let Some(expression) = node.named_children(&mut cursor).find(|n| !n.is_extra()) else {
        return Key::Unknown;
    };
    let (root, selectors) =
        path(expression, source, syntax, graph, 0).unwrap_or((Root::Unknown, Vec::new()));
    Key::Computed {
        expression: span(expression),
        root,
        selectors,
        usage: key_use(node, syntax),
    }
}

fn key_use(node: Node, syntax: &LexicalSyntax) -> ValueUse {
    let forms = syntax.globals.surface;
    if let Some(member) = node.parent() {
        let mut cursor = member.walk();
        if member
            .children(&mut cursor)
            .any(|n| forms.erased_modifiers.contains(&n.kind()))
            || member
                .parent()
                .is_some_and(|n| forms.erased_containers.contains(&n.kind()))
        {
            return ValueUse::Erased;
        }
    }
    let mut parent = node.parent();
    while let Some(node) = parent {
        if syntax.declaration_modifiers.contains(&node.kind()) {
            let mut cursor = node.walk();
            if node
                .children(&mut cursor)
                .any(|n| n.kind() == syntax.modules.ambient_token)
            {
                return ValueUse::Erased;
            }
        }
        parent = node.parent();
    }
    ValueUse::Runtime
}

pub(crate) fn path(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
    depth: usize,
) -> Option<(Root<BindingId>, Vec<NameId>)> {
    if depth >= 64 {
        return None;
    }
    if syntax.globals.names.contains(&node.kind())
        || node.kind() == syntax.initializer_forms.objects.shorthand
    {
        let name = graph.intern(node.utf8_text(source).ok()?);
        let root = match graph.value_expression_binding_at(node.start_byte() as u32, name) {
            Some(binding) => Root::Binding(binding),
            None => {
                let mut scope = graph.scope_at(node.start_byte() as u32);
                while let Some(id) = scope {
                    if graph.capture_barriers.contains(&id) {
                        return Some((Root::Unknown, vec![]));
                    }
                    scope = graph.scopes[id.0].parent;
                }
                Root::Global(name)
            }
        };
        return Some((root, vec![]));
    }
    let &(_, object, property, _) = syntax
        .modules
        .selections
        .iter()
        .find(|&&(kind, _, _, type_space)| kind == node.kind() && !type_space)?;
    let property = crate::indexer::lexical::selections::field_child(node, property)?;
    if !syntax.globals.member_names.contains(&property.kind()) {
        return None;
    }
    let (root, mut selectors) = path(
        crate::indexer::lexical::selections::field_child(node, object)?,
        source,
        syntax,
        graph,
        depth + 1,
    )?;
    selectors.push(graph.intern(property.utf8_text(source).ok()?));
    Some((root, selectors))
}

fn type_node<'a>(node: Node<'a>, forms: &Forms) -> Node<'a> {
    if forms.type_annotations.contains(&node.kind()) {
        node.named_child(0).unwrap_or(node)
    } else {
        node
    }
}

pub(crate) fn signature(node: Node, kind: Kind, forms: &Forms) -> Signature {
    let result = node
        .child_by_field_name("return_type")
        .or_else(|| node.child_by_field_name("type"))
        .map(|n| type_node(n, forms));
    let mut signature = Signature {
        body: node.child_by_field_name("body").map(span),
        result: result.map(span),
        initializer: node.child_by_field_name("value").map(span),
        ..Default::default()
    };
    signature.type_parameters_complete = complete_type_parameters(node);
    signature.ordering = node.parent().map(|parent| SignatureOrder {
        group: span(parent),
        policy: forms.overload_order,
        specialized: false,
    });
    if let Some(result) = result {
        let mut cursor = result.walk();
        let tokens: Vec<_> = result
            .children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|n| n.kind())
            .collect();
        signature.unique_symbol = tokens == forms.unique_type;
    }
    if let Some(parameters) = node.child_by_field_name("type_parameters") {
        let mut cursor = parameters.walk();
        signature.type_parameters = parameters
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(span)
            .collect();
    }
    if let Some(parameters) = node.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        if let Some(ordering) = &mut signature.ordering {
            ordering.specialized = parameters
                .named_children(&mut cursor)
                .filter(|p| !p.is_extra())
                .any(|p| {
                    p.child_by_field_name("type").is_some_and(|n| {
                        forms
                            .specialized_parameter_types
                            .contains(&type_node(n, forms).kind())
                    })
                });
        }
        signature.parameters = parameters
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|p| Parameter {
                receiver: p
                    .child_by_field_name(forms.receiver_parameter.0)
                    .is_some_and(|n| n.kind() == forms.receiver_parameter.1),
                span: span(p),
                type_span: p
                    .child_by_field_name("type")
                    .map(|n| span(type_node(n, forms))),
                optional: p.kind() == forms.optional_parameter
                    || p.child_by_field_name("value").is_some(),
                rest: p
                    .child_by_field_name("pattern")
                    .is_some_and(|n| n.kind() == forms.rest_pattern),
            })
            .collect();
    } else if let Some(parameter) = node.child_by_field_name("parameter") {
        signature.parameters.push(Parameter {
            span: span(parameter),
            type_span: None,
            optional: false,
            rest: false,
            receiver: false,
        });
    } else if kind == Kind::Index {
        if let (Some(name), Some(ty)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("index_type"),
        ) {
            signature.parameters.push(Parameter {
                span: SourceSpan {
                    start: name.start_byte() as u32,
                    end: ty.end_byte() as u32,
                },
                type_span: Some(span(ty)),
                optional: false,
                rest: false,
                receiver: false,
            });
        }
    }
    signature
}

fn complete_type_parameters(node: Node) -> bool {
    let Some(parameters) = node.child_by_field_name("type_parameters") else {
        return true;
    };
    if parameters.has_error() {
        return false;
    }
    let mut cursor = parameters.walk();
    let complete = parameters
        .named_children(&mut cursor)
        .filter(|p| !p.is_extra())
        .all(|parameter| {
            let fields = [
                parameter.child_by_field_name("name"),
                parameter.child_by_field_name("constraint"),
                parameter.child_by_field_name("value"),
            ];
            let mut cursor = parameter.walk();
            let complete = fields[0].is_some()
                && parameter
                    .children(&mut cursor)
                    .filter(|n| !n.is_extra())
                    .all(|child| fields.contains(&Some(child)));
            complete
        });
    complete
}

#[cfg(test)]
#[path = "lexical_member_surface_tests.rs"]
mod tests;
