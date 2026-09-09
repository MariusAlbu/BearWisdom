//! Grammar-owned array roles; spellings end at source NameId ingestion.
use super::{Capture, TypeExpr, TypeForm, TypeUses};
use crate::indexer::lexical::LexicalBindings;
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Kind {
    Mutable,
    Readonly,
}

#[derive(Debug)]
pub(crate) struct Forms {
    pub mutable: &'static str,
    pub readonly: &'static str,
}

pub(super) fn register(forms: &Forms, graph: &mut LexicalBindings, uses: &mut TypeUses) {
    for (kind, spelling) in [
        (Kind::Mutable, forms.mutable),
        (Kind::Readonly, forms.readonly),
    ] {
        uses.array_types.insert(kind, graph.intern(spelling));
    }
}

pub(super) fn readonly(capture: &Capture, node: Node) -> TypeExpr {
    let mut cursor = node.walk();
    let mut children = node.named_children(&mut cursor).filter(|n| !n.is_extra());
    let Some(child) = children.next() else {
        return TypeExpr::Unknown;
    };
    if children.next().is_some() {
        return TypeExpr::Unknown;
    }
    let form = capture
        .syntax
        .type_forms
        .iter()
        .find(|(kind, _)| *kind == child.kind());
    if !matches!(form, Some((_, TypeForm::Array(_) | TypeForm::Tuple))) {
        return TypeExpr::Unknown;
    }
    TypeExpr::Operator(Box::new(
        crate::type_checker::core::types::TypeOperator::Readonly(capture.expr(child)),
    ))
}

#[cfg(test)]
#[path = "lexical_array_types_tests.rs"]
mod tests;
