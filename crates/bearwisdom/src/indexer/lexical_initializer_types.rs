//! Initializer syntax owns expression, signature and value-binding addresses.
use super::{atoms, signatures::SignatureId, Capture, TypeExpr};
use crate::indexer::lexical::{
    globals::member_surface::{self, Key, Kind, Root, ValueUse},
    LexicalBindings, LexicalSyntax,
};
use crate::types::SourceSpan;
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

#[path = "lexical_object_initializers.rs"]
pub(crate) mod objects;

pub(crate) struct Forms {
    pub construct: (&'static str, &'static str),
    pub groups: &'static [&'static str],
    pub atoms: &'static atoms::Forms,
    pub call: (&'static str, &'static str),
    pub objects: &'static objects::Forms,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Expression<R> {
    Unknown,
    Read(SourceSpan),
    Typed(R),
    Construct {
        callee: SourceSpan,
        arguments: Vec<Self>,
        types: Vec<R>,
    },
    Object(SourceSpan),
    Callable {
        signature: SignatureId,
        body: Box<Self>,
    },
    Iife {
        signature: SignatureId,
        arguments: Vec<Self>,
        body: Box<Self>,
    },
}

impl<R> Expression<R> {
    pub(crate) fn map<T>(&self, lower: &impl Fn(&R) -> T) -> Expression<T> {
        match self {
            Self::Unknown => Expression::Unknown,
            Self::Read(site) => Expression::Read(*site),
            Self::Typed(ty) => Expression::Typed(lower(ty)),
            Self::Object(span) => Expression::Object(*span),
            Self::Callable { signature, body } => Expression::Callable {
                signature: *signature,
                body: Box::new(body.map(lower)),
            },
            Self::Iife {
                signature,
                arguments,
                body,
            } => Expression::Iife {
                signature: *signature,
                arguments: arguments.iter().map(|a| a.map(lower)).collect(),
                body: Box::new(body.map(lower)),
            },
            Self::Construct {
                callee,
                arguments,
                types,
            } => Expression::Construct {
                callee: *callee,
                arguments: arguments.iter().map(|a| a.map(lower)).collect(),
                types: types.iter().map(lower).collect(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Input {
    pub declaration: Option<usize>,
    pub signature: SignatureId,
    pub expression: Expression<TypeExpr>,
    pub target: Option<SourceSpan>,
    pub annotated: bool,
}

fn value<'a>(node: Node<'a>, syntax: &LexicalSyntax) -> Option<Node<'a>> {
    if !syntax.variables.contains(&node.kind())
        && !syntax
            .globals
            .surface
            .kinds
            .iter()
            .any(|&(kind, form)| kind == node.kind() && form == Kind::Property)
    {
        return None;
    }
    node.child_by_field_name("value")
}

pub(super) fn intern_paths(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
) {
    objects::intern(node, source, syntax, graph);
    if let Some(value) = value(node, syntax) {
        paths(value, source, syntax, graph, 0);
    }
    if node.kind() == syntax.initializer_forms.objects.object {
        let mut cursor = node.walk();
        for member in node.named_children(&mut cursor).filter(|n| !n.is_extra()) {
            if member.kind() == syntax.initializer_forms.objects.shorthand {
                path(member, source, syntax, graph);
            }
            if let Some(value) = member.child_by_field_name("value") {
                paths(value, source, syntax, graph, 0);
            }
            if let Some(body) = objects::body(member, syntax) {
                paths(body, source, syntax, graph, 0);
            }
        }
    }
}

pub(super) fn capture_object(capture: &Capture, node: Node) -> Option<objects::Input> {
    objects::capture(capture, node)
}

fn paths(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
    depth: usize,
) {
    if depth >= 64 || node.has_error() {
        return;
    }
    let forms = syntax.initializer_forms;
    if node.kind() == forms.call.0 {
        if let Some(function) = node
            .child_by_field_name(forms.call.1)
            .and_then(|n| objects::unwrap(n, syntax))
        {
            if syntax.callback_forms.functions.contains(&function.kind()) {
                if let Some(body) = objects::body(function, syntax) {
                    paths(body, source, syntax, graph, depth + 1);
                }
            }
        }
    }
    if syntax.callback_forms.functions.contains(&node.kind()) {
        if let Some(body) = objects::body(node, syntax) {
            paths(body, source, syntax, graph, depth + 1);
        }
    }
    if node.kind() == forms.call.0 {
        if let Some(arguments) = node.child_by_field_name("arguments") {
            let mut cursor = arguments.walk();
            for argument in arguments
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra())
            {
                paths(argument, source, syntax, graph, depth + 1);
            }
        }
    }
    if forms.groups.contains(&node.kind()) {
        if let Some(child) = node.named_child(0) {
            paths(child, source, syntax, graph, depth + 1);
        }
    } else if node.kind() == forms.construct.0 {
        if let Some(callee) = node.child_by_field_name(forms.construct.1) {
            path(callee, source, syntax, graph);
        }
        if let Some(arguments) = node.child_by_field_name("arguments") {
            let mut cursor = arguments.walk();
            for argument in arguments
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra())
            {
                paths(argument, source, syntax, graph, depth + 1);
            }
        }
    } else if syntax.globals.names.contains(&node.kind())
        || syntax
            .modules
            .selections
            .iter()
            .any(|&(kind, _, _, domain)| kind == node.kind() && !domain)
    {
        path(node, source, syntax, graph);
    }
}

fn path(node: Node, source: &[u8], syntax: &LexicalSyntax, graph: &mut LexicalBindings) {
    let site = super::unique_symbols::span(node);
    let (root, selectors) =
        member_surface::path(node, source, syntax, graph, 0).unwrap_or((Root::Unknown, vec![]));
    graph.types.value_queries.push((
        site,
        Key::Computed {
            expression: site,
            root,
            selectors,
            usage: ValueUse::Runtime,
        },
    ));
}

pub(super) fn capture(capture: &Capture, node: Node) -> Option<Input> {
    let value = value(node, capture.syntax)?;
    let target = capture
        .syntax
        .variables
        .contains(&node.kind())
        .then(|| node.child_by_field_name("name"))
        .flatten()
        .map(super::unique_symbols::span)
        .filter(|site| capture.graph.declarations.contains_key(site));
    Some(Input {
        declaration: capture.slot(node),
        signature: SignatureId(super::unique_symbols::span(node)),
        expression: expression(capture, value, 0),
        target,
        annotated: node.child_by_field_name("type").is_some(),
    })
}

fn expression(capture: &Capture, node: Node, depth: usize) -> Expression<TypeExpr> {
    if depth >= 64 || node.has_error() {
        return Expression::Unknown;
    }
    let forms = capture.syntax.initializer_forms;
    if node.kind() == forms.objects.object {
        return Expression::Object(super::unique_symbols::span(node));
    }
    if capture
        .syntax
        .callback_forms
        .functions
        .contains(&node.kind())
    {
        return objects::callable(capture, node, depth);
    }
    if node.kind() == forms.call.0 {
        let Some(function) = node
            .child_by_field_name(forms.call.1)
            .and_then(|n| objects::unwrap(n, capture.syntax))
        else {
            return Expression::Unknown;
        };
        if !capture
            .syntax
            .callback_forms
            .functions
            .contains(&function.kind())
        {
            return Expression::Unknown;
        }
        let body = objects::body(function, capture.syntax);
        let Some(args) = node.child_by_field_name("arguments") else {
            return Expression::Unknown;
        };
        let mut cursor = args.walk();
        return Expression::Iife {
            signature: SignatureId(super::unique_symbols::span(function)),
            arguments: args
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra())
                .map(|arg| expression(capture, arg, depth + 1))
                .collect(),
            body: Box::new(
                body.map(|n| expression(capture, n, depth + 1))
                    .unwrap_or(Expression::Unknown),
            ),
        };
    }
    if forms.groups.contains(&node.kind()) {
        let mut cursor = node.walk();
        let mut children = node.named_children(&mut cursor).filter(|n| !n.is_extra());
        return match (children.next(), children.next()) {
            (Some(child), None) => expression(capture, child, depth + 1),
            _ => Expression::Unknown,
        };
    }
    if node.kind() == forms.construct.0 {
        let Some(callee) = node.child_by_field_name(forms.construct.1) else {
            return Expression::Unknown;
        };
        let arguments = node
            .child_by_field_name("arguments")
            .map(|args| {
                let mut cursor = args.walk();
                args.named_children(&mut cursor)
                    .filter(|n| !n.is_extra())
                    .map(|n| expression(capture, n, depth + 1))
                    .collect()
            })
            .unwrap_or_default();
        return Expression::Construct {
            callee: super::unique_symbols::span(callee),
            arguments,
            types: node
                .child_by_field_name("type_arguments")
                .map(|n| capture.children(n))
                .unwrap_or_default(),
        };
    }
    if forms
        .atoms
        .literals
        .iter()
        .any(|&(kind, _)| kind == node.kind())
        || node.kind() == forms.atoms.negative.0
        || forms
            .atoms
            .intrinsics
            .iter()
            .any(|&(kind, _)| kind == node.kind())
    {
        return Expression::Typed(atoms::capture(node, capture.source, forms.atoms));
    }
    if capture.syntax.globals.names.contains(&node.kind())
        || capture
            .syntax
            .modules
            .selections
            .iter()
            .any(|&(kind, _, _, domain)| kind == node.kind() && !domain)
    {
        return Expression::Read(super::unique_symbols::span(node));
    }
    Expression::Unknown
}

#[cfg(test)]
#[path = "lexical_initializer_types_tests.rs"]
mod tests;
