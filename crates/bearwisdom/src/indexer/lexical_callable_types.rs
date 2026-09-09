//! Ingestion converts predicate names into signature-local source parameter IDs.
use super::{signatures, Capture, TypeExpr};
use crate::indexer::lexical::LexicalBindings;
use crate::type_checker::core::types::{
    Callable, CallableGeneric, CallableParameter, CallablePredicate, Intrinsic,
};
use crate::types::SourceSpan;
use tree_sitter::Node;

#[derive(Debug)]
pub(crate) struct Forms {
    pub predicate: &'static str,
    pub assertion: &'static str,
    pub identifier: &'static [&'static str],
    pub pattern: &'static str,
    pub predicate_name: &'static str,
    pub predicate_type: &'static str,
}
fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

pub(super) fn intern(node: Node, source: &[u8], graph: &mut LexicalBindings, forms: &Forms) {
    let mut names = Vec::new();
    if let Some(parameters) = node.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        names.extend(
            parameters
                .named_children(&mut cursor)
                .filter_map(|p| p.child_by_field_name(forms.pattern)),
        );
    }
    if let Some(result) = node.child_by_field_name("return_type") {
        let target = if result.kind() == forms.assertion {
            result.named_child(0)
        } else {
            Some(result)
        };
        if let Some(target) = target {
            names.push(
                target
                    .child_by_field_name(forms.predicate_name)
                    .unwrap_or(target),
            );
        }
    }
    for name in names
        .into_iter()
        .filter(|n| forms.identifier.contains(&n.kind()))
    {
        if let Ok(name) = name.utf8_text(source) {
            graph.intern(name);
        }
    }
}

pub(super) fn capture(capture: &Capture, node: Node, forms: &Forms) -> TypeExpr {
    let Some(signature) = signatures::capture(capture, node) else {
        return TypeExpr::Unknown;
    };
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return TypeExpr::Unknown;
    };
    let mut cursor = parameters.walk();
    let nodes: Vec<_> = parameters
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra())
        .collect();
    let parameters = signature
        .syntax
        .parameters
        .iter()
        .zip(&signature.parameters)
        .map(|(syntax, ty)| CallableParameter {
            declaration: syntax.span,
            ty: ty.clone(),
            optional: syntax.optional,
            rest: syntax.rest,
            receiver: syntax.receiver,
        })
        .collect();
    let generics = signature
        .generics
        .iter()
        .enumerate()
        .map(|(index, p)| CallableGeneric {
            parameter: match signature.declaration {
                Some(owner) => TypeExpr::Parameter {
                    owner: Some(owner),
                    index,
                },
                None => TypeExpr::SignatureParameter {
                    owner: signature.id,
                    index,
                },
            },
            constraint: p.constraint.clone(),
            default: p.default.clone(),
        })
        .collect();
    let legacy = TypeExpr::Function(
        signature.parameters.clone(),
        Box::new(signature.result.clone().unwrap_or(TypeExpr::Unknown)),
    );
    let mut callable = Callable {
        origin: signature.id.0,
        generics,
        parameters,
        result: signature.result.unwrap_or(TypeExpr::Unknown),
        predicate: None,
        complete: !node.has_error() && nodes.len() == signature.syntax.parameters.len(),
    };
    if let Some(result) = node.child_by_field_name("return_type") {
        let asserts = result.kind() == forms.assertion;
        let target = if asserts {
            result.named_child(0)
        } else {
            Some(result)
        };
        if asserts || result.kind() == forms.predicate {
            let predicate = (|| {
                let target = target?;
                let (name, asserted) = if target.kind() == forms.predicate {
                    (
                        target.child_by_field_name(forms.predicate_name)?,
                        Some(capture.expr(target.child_by_field_name(forms.predicate_type)?)),
                    )
                } else {
                    (target, None)
                };
                if !forms.identifier.contains(&name.kind()) {
                    return None;
                }
                let name = capture
                    .graph
                    .name_id(name.utf8_text(capture.source).ok()?)?;
                let matches: Vec<_> = nodes
                    .iter()
                    .filter_map(|p| {
                        let pattern = p.child_by_field_name(forms.pattern)?;
                        if !forms.identifier.contains(&pattern.kind()) {
                            return None;
                        }
                        let id = capture
                            .graph
                            .name_id(pattern.utf8_text(capture.source).ok()?)?;
                        (id == name).then_some(span(*p))
                    })
                    .collect();
                let [parameter] = matches.as_slice() else {
                    return None;
                };
                Some(CallablePredicate {
                    parameter: *parameter,
                    asserted,
                    asserts,
                })
            })();
            callable.complete &= predicate.is_some();
            callable.predicate = predicate;
            callable.result = TypeExpr::Intrinsic(if asserts {
                Intrinsic::Void
            } else {
                Intrinsic::Boolean
            });
        }
    }
    TypeExpr::Callable(Box::new(callable), Box::new(legacy))
}

#[cfg(test)]
#[path = "lexical_callable_types_tests.rs"]
mod tests;
