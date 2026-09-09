//! Persist source arenas explicitly; physical member rows are optional evidence.
use crate::indexer::{
    lexical::globals::member_surface::{Key, Member, Root},
    symbol_ids::SymbolIds,
};

pub(in crate::indexer::resolve::engine) type Input = Member<i64, usize>;

pub(in crate::indexer::resolve::engine) fn lower(
    member: &Member,
    path: &str,
    ids: &SymbolIds,
) -> Input {
    let key = match &member.key {
        Key::Named(name) => Key::Named(*name),
        Key::Private(name) => Key::Private(*name),
        Key::Literal(span) => Key::Literal(*span),
        Key::Call => Key::Call,
        Key::Construct => Key::Construct,
        Key::Index => Key::Index,
        Key::Unknown => Key::Unknown,
        Key::Computed {
            expression,
            root,
            selectors,
            usage,
        } => Key::Computed {
            expression: *expression,
            selectors: selectors.clone(),
            usage: *usage,
            root: match root {
                Root::Binding(id) => Root::Binding(id.0),
                Root::Global(name) => Root::Global(*name),
                Root::Unknown => Root::Unknown,
            },
        },
    };
    Member {
        span: member.span,
        key_span: member.key_span,
        kind: member.kind,
        key,
        modifiers: member.modifiers.clone(),
        signature: member.signature.clone(),
        slot: member.slot.and_then(|slot| ids.row_id(path, slot)),
    }
}

#[cfg(test)]
#[path = "program_member_inputs_tests.rs"]
mod tests;
