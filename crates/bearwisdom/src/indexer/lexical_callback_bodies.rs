//! Source recipes are captured once; runtime callback evaluation never reads spelling.
use super::{Atom, LexicalSyntax, Node, SourceSpan};
use crate::indexer::lexical::type_syntax::{atoms, TypeExpr};
use crate::type_checker::core::types::{Intrinsic, LitValue};

pub(crate) struct Forms {
    pub functions: &'static [&'static str],
    pub wrappers: &'static [&'static str],
    pub block: &'static str,
    pub return_: &'static str,
    pub forbidden_tokens: &'static [&'static str],
    pub forbidden_expressions: &'static [&'static str],
    pub binary: &'static str,
    pub equality: &'static [(&'static str, bool)],
    pub typeof_: (&'static str, &'static str),
    pub boolean_not: (&'static str, &'static str, &'static str),
    pub type_names: &'static [(&'static str, Intrinsic)],
}

#[derive(Debug, Clone)]
pub(crate) struct Callback {
    pub signature: SourceSpan,
    pub parameters: Vec<SourceSpan>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub(crate) enum Expr {
    Atom(Atom),
    Read(SourceSpan),
    Member {
        receiver: Box<Self>,
        selector: u32,
    },
    Call {
        receiver: Box<Self>,
        selector: u32,
        arguments: Vec<Self>,
    },
    TypeTest {
        operand: Box<Self>,
        kind: Intrinsic,
        negated: bool,
    },
    Not(Box<Self>),
    Unknown,
}

impl Expr {
    pub(super) fn reads(&self, visit: &mut impl FnMut(SourceSpan)) {
        match self {
            Self::Read(span) => visit(*span),
            Self::Member { receiver, .. }
            | Self::TypeTest {
                operand: receiver, ..
            }
            | Self::Not(receiver) => receiver.reads(visit),
            Self::Call {
                receiver,
                arguments,
                ..
            } => {
                receiver.reads(visit);
                for arg in arguments {
                    arg.reads(visit);
                }
            }
            _ => {}
        }
    }
}

fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

fn unwrap<'a>(mut node: Node<'a>, forms: &Forms) -> Option<Node<'a>> {
    for _ in 0..64 {
        if !forms.wrappers.contains(&node.kind()) {
            return Some(node);
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .collect();
        let [child] = children.as_slice() else {
            return None;
        };
        node = *child;
    }
    None
}

pub(super) fn capture(node: Node, source: &[u8], syntax: &LexicalSyntax) -> Option<Callback> {
    let forms = syntax.callback_forms;
    let node = unwrap(node, forms)?;
    if !forms.functions.contains(&node.kind()) {
        return None;
    }
    let mut result = Callback {
        signature: span(node),
        parameters: vec![],
        body: Expr::Unknown,
    };
    let mut cursor = node.walk();
    if node.has_error()
        || node.child_by_field_name("type_parameters").is_some()
        || node
            .children(&mut cursor)
            .any(|n| forms.forbidden_tokens.contains(&n.kind()))
    {
        return Some(result);
    }
    let parameters = if let Some(params) = node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        params
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .collect::<Vec<_>>()
    } else {
        node.child_by_field_name("parameter").into_iter().collect()
    };
    for parameter in parameters {
        let name = parameter
            .child_by_field_name("pattern")
            .unwrap_or(parameter);
        if !syntax.call_identifiers.contains(&name.kind())
            || parameter.child_by_field_name("value").is_some()
        {
            return Some(result);
        }
        result.parameters.push(span(name));
    }
    let Some(mut body) = node.child_by_field_name("body") else {
        return Some(result);
    };
    if body.kind() == forms.block {
        let mut cursor = body.walk();
        let statements: Vec<_> = body
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .collect();
        let [statement] = statements.as_slice() else {
            return Some(result);
        };
        if statement.kind() != forms.return_ {
            return Some(result);
        }
        let Some(value) = statement.named_child(0) else {
            return Some(result);
        };
        body = value;
    }
    result.body = expression(body, source, syntax, 0).unwrap_or(Expr::Unknown);
    Some(result)
}

fn expression(node: Node, source: &[u8], syntax: &LexicalSyntax, depth: usize) -> Option<Expr> {
    if depth >= 64 || node.has_error() {
        return None;
    }
    let forms = syntax.callback_forms;
    let node = unwrap(node, forms)?;
    let mut cursor = node.walk();
    if node
        .children(&mut cursor)
        .any(|n| forms.forbidden_expressions.contains(&n.kind()))
    {
        return None;
    }
    let child = |node| expression(node, source, syntax, depth + 1);
    if node.kind() == forms.boolean_not.0
        && node
            .child_by_field_name("operator")
            .and_then(|n| n.utf8_text(source).ok())
            == Some(forms.boolean_not.1)
    {
        return Some(Expr::Not(Box::new(child(
            node.child_by_field_name(forms.boolean_not.2)?,
        )?)));
    }
    if syntax.call_identifiers.contains(&node.kind()) {
        return Some(Expr::Read(span(node)));
    }
    match atoms::capture(node, source, syntax.initializer_forms.atoms) {
        TypeExpr::Literal(value) => return Some(Expr::Atom(Atom::Literal(value))),
        TypeExpr::Intrinsic(kind) if !syntax.globals.names.contains(&node.kind()) => {
            return Some(Expr::Atom(Atom::Intrinsic(kind)))
        }
        _ => {}
    }
    if node.kind() == forms.binary {
        let operator = node
            .child_by_field_name("operator")?
            .utf8_text(source)
            .ok()?;
        let &(_, negated) = forms.equality.iter().find(|&&(op, _)| op == operator)?;
        let left = unwrap(node.child_by_field_name("left")?, forms)?;
        let right = unwrap(node.child_by_field_name("right")?, forms)?;
        for (test, value) in [(left, right), (right, left)] {
            if test.kind() != forms.typeof_.0
                || test
                    .child_by_field_name("operator")
                    .and_then(|n| n.utf8_text(source).ok())
                    != Some(forms.typeof_.1)
            {
                continue;
            }
            let TypeExpr::Literal(LitValue::Str(value)) =
                atoms::capture(value, source, syntax.initializer_forms.atoms)
            else {
                continue;
            };
            let &(_, kind) = forms.type_names.iter().find(|&&(word, _)| word == value)?;
            return Some(Expr::TypeTest {
                operand: Box::new(child(test.child_by_field_name("argument")?)?),
                kind,
                negated,
            });
        }
    }
    if let Some(&(_, object, property, _)) = syntax
        .modules
        .selections
        .iter()
        .find(|&&(kind, _, _, type_space)| kind == node.kind() && !type_space)
    {
        let selector = node.child_by_field_name(property)?;
        if !syntax.globals.member_names.contains(&selector.kind()) {
            return None;
        }
        return Some(Expr::Member {
            receiver: Box::new(child(node.child_by_field_name(object)?)?),
            selector: selector.start_byte() as u32,
        });
    }
    if let Some(&(_, field)) = syntax
        .call_roots
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
    {
        if node.child_by_field_name("type_arguments").is_some() {
            return None;
        }
        let Expr::Member { receiver, selector } = child(node.child_by_field_name(field)?)? else {
            return None;
        };
        let args = node.child_by_field_name("arguments")?;
        let mut cursor = args.walk();
        let arguments = args
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(child)
            .collect::<Option<Vec<_>>>()?;
        return Some(Expr::Call {
            receiver,
            selector,
            arguments,
        });
    }
    None
}

#[cfg(test)]
#[path = "lexical_callback_bodies_tests.rs"]
mod tests;
