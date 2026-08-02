// =============================================================================
// languages/typescript/qualify_members — a member is named by its parent
//
// Member qnames come from the scope tree, which tracks LEXICAL scopes: a class,
// an interface, a function body. A type alias opens none, so members declared
// inside one are qualified against whatever lexical scope encloses the alias —
// at file level, that is nothing at all:
//
//     type TurbopackRuleCondition = { any: … } | { all: … }   → qname `any`
//
// A bare `any` in the qname index is then the exact-qname hit for every receiver
// whose head is `any`, and every member walk from it dead-ends. The structural
// parent is already recorded (`parent_index`) and names the member.
//
// The ancestor that names it is the nearest TYPE, not the immediate parent. An
// anonymous object type has no name for a receiver to carry, so its members are
// reachable only as members of the type that declares them — flattening them
// onto the enclosing type is what lets `binding.turbo.createProject()` find
// `createProject` at all. Walking past the intermediate property preserves that.
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

/// Re-derive every symbol's qualified name from the nearest enclosing type,
/// then every scope path from the immediate structural parent. Qnames first, in
/// ascending order so an ancestor is corrected before the members that build on
/// it; scope paths in a second pass over the final qnames, because SYM-002
/// requires `scope_path == symbols[parent_index].qualified_name` — the
/// IMMEDIATE parent, even when the qname is flattened past it.
pub(super) fn name_under_parents(symbols: &mut [ExtractedSymbol]) {
    for i in 0..symbols.len() {
        let Some(owner) = naming_owner(symbols, i) else {
            continue;
        };
        let owner_qname = symbols[owner].qualified_name.clone();
        if owner_qname.is_empty() {
            continue;
        }
        if !is_named_under(&symbols[i].qualified_name, &owner_qname) {
            symbols[i].qualified_name = format!("{owner_qname}.{}", symbols[i].name);
        }
    }
    // The scope-tree path drifts for synthetic-name children (index signature
    // parameters `[s]`, computed keys `[Symbol.match]`, mapped-type binders)
    // emitted inside method bodies — their lexical scope is the method, not the
    // declaring type. Re-derive from the structural parent.
    for i in 0..symbols.len() {
        if let Some(p) = symbols[i].parent_index {
            let parent_qname = symbols[p].qualified_name.clone();
            symbols[i].scope_path = Some(parent_qname);
        }
    }
}

/// The ancestor that gives `symbols[i]` its name: its parent, or — when that
/// parent is a property holding an inline object type — the first ancestor past
/// the inline ones. `None` for a symbol with no parent.
///
/// Parents are pushed before their children, so a forward or self reference is
/// not a container relation; the `p < cur` test both honours that and bounds
/// the walk.
fn naming_owner(symbols: &[ExtractedSymbol], i: usize) -> Option<usize> {
    let mut cur = i;
    while let Some(p) = symbols[cur].parent_index.filter(|&p| p < cur) {
        if !holds_an_inline_object(symbols[p].kind) {
            return Some(p);
        }
        cur = p;
    }
    None
}

/// `true` for a member kind whose own type can be an inline object literal
/// (`turbo: { createProject(): … }`). Such a type is anonymous — no receiver can
/// carry its name — so the members declared in it belong to the nearest
/// enclosing declaration that DOES have one.
fn holds_an_inline_object(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Property | SymbolKind::Field)
}

/// `true` when `qname` already sits under `owner_qname` — the owner's name
/// followed by a separator. A discriminated union's synthetic branch encodes its
/// owner with a `\u{1}` sentinel rather than a dot (`Shape\u{1}0`), so any
/// non-identifier byte counts as the boundary; requiring one keeps a sibling
/// whose name merely starts with the owner's (`FooBar` under `Foo`) out.
fn is_named_under(qname: &str, owner_qname: &str) -> bool {
    let Some(rest) = qname.strip_prefix(owner_qname) else {
        return false;
    };
    rest.chars()
        .next()
        .is_some_and(|c| !c.is_alphanumeric() && c != '_' && c != '$')
}

#[cfg(test)]
#[path = "qualify_members_tests.rs"]
mod tests;
