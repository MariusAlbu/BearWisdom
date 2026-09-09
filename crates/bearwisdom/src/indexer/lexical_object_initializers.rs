//! Object inventories keep source members apart from inferred structural types.
use super::*;
use crate::indexer::lexical::NameId;
use crate::type_checker::core::types::LitValue;

pub(crate) struct Forms {
    pub object: &'static str,
    pub property: (&'static str, &'static str, &'static str),
    pub shorthand: &'static str,
    pub declarations: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Member<R, D = usize> {
    pub name: NameId,
    pub span: SourceSpan,
    pub declaration: Option<D>,
    pub kind: Kind,
    pub value: Expression<R>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Input<R = TypeExpr, D = usize> {
    pub span: SourceSpan,
    pub members: Option<Vec<Member<R, D>>>,
}

fn key(node: Node, source: &[u8], syntax: &LexicalSyntax) -> Option<String> {
    if syntax.globals.member_names.contains(&node.kind())
        || node.kind() == syntax.initializer_forms.objects.shorthand
    {
        return node.utf8_text(source).ok().map(String::from);
    }
    match atoms::capture(node, source, syntax.initializer_forms.atoms) {
        TypeExpr::Literal(LitValue::Str(key)) => Some(key),
        _ => None,
    }
}

pub(super) fn intern(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
) {
    let forms = syntax.initializer_forms.objects;
    if node.kind() != forms.object {
        return;
    }
    let mut cursor = node.walk();
    for member in node.named_children(&mut cursor).filter(|n| !n.is_extra()) {
        let name = if member.kind() == forms.property.0 {
            member.child_by_field_name(forms.property.1)
        } else if member.kind() == forms.shorthand {
            Some(member)
        } else {
            member.child_by_field_name("name")
        };
        if let Some(name) = name.and_then(|n| key(n, source, syntax)) {
            graph.intern(&name);
        }
    }
}

pub(super) fn capture(capture: &Capture, node: Node) -> Option<Input> {
    let forms = capture.syntax.initializer_forms.objects;
    if node.kind() != forms.object {
        return None;
    }
    let mut cursor = node.walk();
    let members = (!node.has_error())
        .then(|| {
            node.named_children(&mut cursor)
                .filter(|n| !n.is_extra())
                .map(|member| {
                    let (kind, name, value) = if member.kind() == forms.property.0 {
                        (
                            Kind::Property,
                            member.child_by_field_name(forms.property.1)?,
                            expression(capture, member.child_by_field_name(forms.property.2)?, 0),
                        )
                    } else if member.kind() == forms.shorthand {
                        (
                            Kind::Property,
                            member,
                            Expression::Read(super::super::unique_symbols::span(member)),
                        )
                    } else {
                        if !capture
                            .syntax
                            .globals
                            .surface
                            .kinds
                            .iter()
                            .any(|&(form, kind)| form == member.kind() && kind == Kind::Method)
                        {
                            return None;
                        }
                        let mut cursor = member.walk();
                        if member.children(&mut cursor).any(|n| {
                            capture
                                .syntax
                                .callback_forms
                                .forbidden_tokens
                                .contains(&n.kind())
                                || [
                                    capture.syntax.globals.surface.accessor_tokens.0,
                                    capture.syntax.globals.surface.accessor_tokens.1,
                                ]
                                .contains(&n.kind())
                        }) {
                            return None;
                        }
                        (
                            Kind::Method,
                            member.child_by_field_name("name")?,
                            callable(capture, member, 0),
                        )
                    };
                    let name =
                        capture
                            .graph
                            .name_id(&key(name, capture.source, capture.syntax)?)?;
                    Some(Member {
                        name,
                        span: super::super::unique_symbols::span(member),
                        declaration: capture.slot(member),
                        kind,
                        value,
                    })
                })
                .collect()
        })
        .flatten();
    Some(Input {
        span: super::super::unique_symbols::span(node),
        members,
    })
}

pub(super) fn unwrap<'a>(mut node: Node<'a>, syntax: &LexicalSyntax) -> Option<Node<'a>> {
    for _ in 0..64 {
        if !syntax.initializer_forms.groups.contains(&node.kind()) {
            return Some(node);
        }
        let mut cursor = node.walk();
        let mut children = node.named_children(&mut cursor).filter(|n| !n.is_extra());
        let child = children.next()?;
        if children.next().is_some() {
            return None;
        }
        node = child;
    }
    None
}

pub(super) fn body<'a>(function: Node<'a>, syntax: &LexicalSyntax) -> Option<Node<'a>> {
    let forms = syntax.callback_forms;
    let mut cursor = function.walk();
    if function.has_error()
        || function
            .children(&mut cursor)
            .any(|n| forms.forbidden_tokens.contains(&n.kind()))
    {
        return None;
    }
    let body = function.child_by_field_name("body")?;
    if body.kind() != forms.block {
        return Some(body);
    }
    let mut cursor = body.walk();
    let statements: Vec<_> = body
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra())
        .collect();
    let (last, before) = statements.split_last()?;
    if last.kind() != forms.return_
        || before.iter().any(|n| {
            !syntax
                .initializer_forms
                .objects
                .declarations
                .contains(&n.kind())
        })
    {
        return None;
    }
    let mut cursor = last.walk();
    let mut values = last.named_children(&mut cursor).filter(|n| !n.is_extra());
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

pub(super) fn callable(capture: &Capture, node: Node, depth: usize) -> Expression<TypeExpr> {
    let mut cursor = node.walk();
    if node.children(&mut cursor).any(|n| {
        capture
            .syntax
            .callback_forms
            .forbidden_tokens
            .contains(&n.kind())
    }) {
        return Expression::Unknown;
    }
    Expression::Callable {
        signature: SignatureId(super::super::unique_symbols::span(node)),
        body: Box::new(
            body(node, capture.syntax)
                .map(|node| expression(capture, node, depth + 1))
                .unwrap_or(Expression::Unknown),
        ),
    }
}
