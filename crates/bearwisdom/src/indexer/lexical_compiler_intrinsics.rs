//! Compiler-provided alias roles are attested only at source syntax ingestion.
use super::{Capture, TypeExpr, TypeUses};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Role {
    IteratorReturn,
}

pub(crate) struct Forms {
    pub keyword: &'static str,
    pub keyword_kind: &'static str,
    pub name_field: &'static str,
    pub parameters_field: &'static str,
    pub zero_parameter_aliases: &'static [(&'static str, Role)],
}

pub(super) fn alias(
    capture: &Capture<'_>,
    declaration: Node,
    value: Node,
    slot: usize,
    uses: &mut TypeUses,
) {
    let binding = capture.graph.symbols.get(&slot).copied();
    let unique = binding
        .and_then(|binding| capture.graph.type_symbol_slots.get(&binding))
        .is_some_and(|slots| slots.as_slice() == [slot]);
    if !unique {
        uses.aliases.insert(slot, TypeExpr::Unknown);
        return;
    }
    let forms = capture.syntax.compiler_intrinsics;
    let keyword = value.kind() == forms.keyword_kind
        && value.utf8_text(capture.source).ok() == Some(forms.keyword);
    if !keyword {
        uses.aliases.insert(slot, capture.expr(value));
        return;
    }
    // Even invalid/unsupported keyword aliases are authoritative barriers;
    // a same-spelled ordinary type declaration cannot supply their body.
    uses.aliases.insert(slot, TypeExpr::Unknown);
    if declaration.has_error()
        || declaration
            .child_by_field_name(forms.parameters_field)
            .is_some()
    {
        return;
    }
    let Some(name) = declaration
        .child_by_field_name(forms.name_field)
        .and_then(|n| n.utf8_text(capture.source).ok())
    else {
        return;
    };
    if let Some((_, role)) = forms
        .zero_parameter_aliases
        .iter()
        .find(|(spelling, _)| *spelling == name)
    {
        uses.compiler_intrinsics.insert(slot, *role);
    }
}
