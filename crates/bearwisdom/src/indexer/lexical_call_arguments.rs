//! Call operands are captured from CST sites, independently of reference display data.
use super::*;
use crate::indexer::lexical::type_syntax::{atoms, TypeExpr};
use crate::type_checker::core::types::{Intrinsic, LitValue, Type};
use crate::types::CallArg;

#[path = "lexical_callback_bodies.rs"]
pub(crate) mod callbacks;

#[derive(Debug, Clone)]
pub(crate) enum Atom {
    Literal(LitValue),
    Intrinsic(Intrinsic),
}
impl Atom {
    pub(crate) fn ty(&self) -> Type {
        match self {
            Self::Literal(value) => Type::Literal(value.clone()),
            Self::Intrinsic(value) => Type::Intrinsic(*value),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Capture {
    pub arguments: crate::indexer::namespaces::arguments::Table,
    pub atoms: HashMap<SourceSpan, Atom>,
    pub callbacks: HashMap<(u32, usize), callbacks::Callback>,
}

pub(super) fn capture(node: Node, source: &[u8], syntax: &LexicalSyntax, output: &mut Capture) {
    if let Some(&(_, field)) = syntax
        .call_roots
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
    {
        if let Some(callee) = node.child_by_field_name(field) {
            let selector = syntax
                .call_selectors
                .iter()
                .find(|&&(kind, _)| kind == callee.kind())
                .and_then(|&(_, field)| callee.child_by_field_name(field));
            let supported = selector.is_some() || syntax.globals.names.contains(&callee.kind());
            let byte = selector.unwrap_or(callee).start_byte() as u32;
            let arguments = node
                .child_by_field_name("arguments")
                .filter(|_| supported && !node.has_error())
                .map(|arguments| {
                    let mut captured =
                        crate::languages::common::call_args::extract_call_args(&node, source);
                    let mut cursor = arguments.walk();
                    for (index, (argument, operand)) in arguments
                        .named_children(&mut cursor)
                        .filter(|n| !n.is_extra())
                        .zip(&mut captured)
                        .enumerate()
                    {
                        if let Some(callback) = callbacks::capture(argument, source, syntax) {
                            // Keep source parameter tokens for post-selection seeding.
                            *operand = CallArg::LambdaAt {
                                params: callback.parameters.iter().copied().map(Some).collect(),
                            };
                            output.callbacks.insert((byte, index), callback);
                            continue;
                        }
                        if syntax.call_identifiers.contains(&argument.kind()) {
                            *operand = CallArg::IdentAt(SourceSpan {
                                start: argument.start_byte() as u32,
                                end: argument.end_byte() as u32,
                            });
                            continue;
                        }
                        let atom = match atoms::capture(
                            argument,
                            source,
                            syntax.initializer_forms.atoms,
                        ) {
                            TypeExpr::Literal(value) => Some(Atom::Literal(value)),
                            // These tokens may also be shadowed identifiers. Only
                            // syntax keywords/literals can attest runtime atoms.
                            TypeExpr::Intrinsic(kind)
                                if !syntax.globals.names.contains(&argument.kind()) =>
                            {
                                Some(Atom::Intrinsic(kind))
                            }
                            _ => None,
                        };
                        if let Some(atom) = atom {
                            let span = SourceSpan {
                                start: argument.start_byte() as u32,
                                end: argument.end_byte() as u32,
                            };
                            output.atoms.insert(span, atom);
                            *operand = CallArg::ValueAt(span);
                        }
                    }
                    captured
                });
            output
                .arguments
                .entry(byte)
                .and_modify(|old| *old = None)
                .or_insert(arguments);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        capture(child, source, syntax, output);
    }
}

pub(super) fn bind_reads(
    calls: &Capture,
    source: &[u8],
    graph: &mut LexicalBindings,
    globals: &mut HashMap<SourceSpan, NameId>,
) {
    let mut bind = |span: SourceSpan| {
        let Some(name) = source
            .get(span.start as usize..span.end as usize)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            return;
        };
        let name = graph.intern(name);
        if let Some(binding) = graph.reference_binding_at(span.start, name) {
            graph.argument_reads.insert(span, binding);
        }
        if graph.binding_at(span.start, name).is_none() {
            globals.insert(span, name);
        }
    };
    for argument in calls.arguments.values().flatten().flatten() {
        argument.visit_identifiers(&mut bind);
    }
    for callback in calls.callbacks.values() {
        callback.body.reads(&mut bind);
    }
}

#[cfg(test)]
#[path = "lexical_call_arguments_tests.rs"]
mod tests;
