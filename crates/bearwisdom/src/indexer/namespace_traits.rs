//! Source trait contracts, independent of inherent ownership and method selection.
use super::*;
use crate::indexer::lexical::type_syntax::TypeExpr;
use crate::type_checker::core::types::GenericParamKind;
use crate::types::SourceSpan;

pub(crate) struct Forms {
    pub declaration: &'static str,
    pub implementation_trait: &'static str,
    pub members: &'static [&'static str],
    pub bounds: &'static str,
    pub where_clause: &'static str,
    pub where_predicate: &'static str,
    pub predicate_subject: &'static str,
    pub negative_token: &'static str,
    pub qualified_wrapper: &'static str,
    pub qualified_type: (&'static str, &'static str, &'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    Declaration(usize),
    Implementation(BindingId),
}

#[derive(Debug, Clone)]
pub(crate) struct Header {
    pub owner: Owner,
    /// Physical source declaration, including a non-nominal impl container.
    pub declaration: Option<usize>,
    pub self_binding: BindingId,
    pub unit: SourceModuleId,
    pub scope: ScopeId,
    pub span: SourceSpan,
    pub members: Vec<usize>,
    /// Conditional or unretained source cannot attest an active implementation.
    pub enabled: bool,
    pub negative: bool,
    pub parameters: Vec<(BindingId, GenericParamKind)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Implementation {
    pub owner: BindingId,
    pub trait_type: TypeExpr,
    pub receiver: TypeExpr,
}

#[derive(Debug, Clone)]
pub(crate) struct QualifiedCall {
    pub root: SourceSpan,
    pub caller: Option<usize>,
    pub receiver: TypeExpr,
    pub trait_type: TypeExpr,
}

#[derive(Debug, Clone)]
pub(crate) struct Bound {
    pub owner: Owner,
    pub span: SourceSpan,
    pub subject: TypeExpr,
    pub traits: Vec<TypeExpr>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Availability {
    pub parent: Option<ScopeId>,
    pub bindings: Vec<BindingId>,
    pub complete: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Data {
    pub self_bindings: std::collections::HashSet<BindingId>,
    pub value_partners: Vec<(usize, usize)>,
    pub headers: Vec<Header>,
    pub header_at: HashMap<SourceSpan, usize>,
    pub implementations: Vec<Implementation>,
    pub bounds: Vec<Bound>,
    pub parameters: HashMap<BindingId, (Use, usize)>,
    pub available: HashMap<ScopeId, Availability>,
    pub method_scopes: HashMap<u32, ScopeId>,
    pub qualified_calls: HashMap<u32, QualifiedCall>,
    pub anonymous_imports: HashMap<ScopeId, Vec<BindingId>>,
}

/// Associated callable declarations have an attested physical trait parent.
/// Do not infer method ownership from a qualified name or a receiver spelling.
pub(crate) fn classify_methods(symbols: &mut [ExtractedSymbol]) {
    for slot in 0..symbols.len() {
        if symbols[slot].kind == crate::types::SymbolKind::Function
            && symbols[slot]
                .parent_index
                .and_then(|parent| symbols.get(parent))
                .is_some_and(|parent| parent.kind == crate::types::SymbolKind::Trait)
        {
            symbols[slot].kind = crate::types::SymbolKind::Method;
        }
    }
}

#[cfg(test)]
#[path = "namespace_traits_tests.rs"]
mod tests;
