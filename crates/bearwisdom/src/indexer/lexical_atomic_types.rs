//! Decode literal spelling once. Semantic recipes contain enum/value identities.
use super::TypeExpr;
use crate::type_checker::core::types::{Intrinsic, LitValue};
use tree_sitter::Node;

#[path = "lexical_atomic_numbers.rs"]
mod numbers;
#[path = "lexical_atomic_strings.rs"]
mod strings;

#[derive(Debug, Clone, Copy)]
pub(crate) enum LiteralKind {
    String,
    Number,
    Boolean(bool),
}
#[derive(Debug)]
pub(crate) struct Forms {
    pub member_wrappers: &'static [(Intrinsic, &'static str)],
    pub intrinsics: &'static [(&'static str, Intrinsic)],
    pub literals: &'static [(&'static str, LiteralKind)],
    pub negative: (&'static str, &'static str, &'static str, &'static str),
    pub numeric_separator: char,
    pub bigint_suffix: &'static str,
    pub radix_prefixes: &'static [(&'static str, u32)],
    pub quotes: &'static [char],
    pub escapes: &'static [(char, u16)],
    pub identity_escapes: bool,
    pub unicode_escape: char,
    pub hex_escape: char,
    pub braced_unicode: bool,
}

pub(in crate::indexer::lexical) fn capture(node: Node, source: &[u8], forms: &Forms) -> TypeExpr {
    let Some(text) = node.utf8_text(source).ok() else {
        return TypeExpr::Unknown;
    };
    if let Some(&(_, kind)) = forms.intrinsics.iter().find(|&&(word, _)| word == text) {
        return TypeExpr::Intrinsic(kind);
    }
    let result = if node.kind() == forms.negative.0 {
        let operator = node
            .child_by_field_name(forms.negative.2)
            .and_then(|n| n.utf8_text(source).ok());
        let argument = node.child_by_field_name(forms.negative.1);
        match argument.filter(|n| {
            operator == Some(forms.negative.3)
                && forms
                    .literals
                    .iter()
                    .any(|&(kind, form)| kind == n.kind() && matches!(form, LiteralKind::Number))
        }) {
            Some(n) => n
                .utf8_text(source)
                .ok()
                .and_then(|text| numbers::decode(text, true, forms)),
            None => None,
        }
    } else {
        forms
            .literals
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
            .and_then(|&(_, kind)| match kind {
                LiteralKind::String => strings::decode(text, forms),
                LiteralKind::Number => numbers::decode(text, false, forms),
                LiteralKind::Boolean(value) => Some(LitValue::Bool(value)),
            })
    };
    result.map(TypeExpr::Literal).unwrap_or(TypeExpr::Unknown)
}

#[cfg(test)]
#[path = "lexical_atomic_types_tests.rs"]
mod tests;
