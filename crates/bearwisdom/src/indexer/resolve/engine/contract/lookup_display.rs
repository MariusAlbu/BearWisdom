//! Legacy display boundary shared by the lookup's field/return string adapters.
use crate::type_checker::core::types::{TypeArena, TypeId};

pub(super) fn render<'a>(
    ty: Option<TypeId>,
    arena: Option<&TypeArena>,
    legacy: impl FnOnce() -> Option<&'a str>,
) -> Option<String> {
    if let (Some(id), Some(arena)) = (ty, arena) {
        return Some(arena.format_type(id));
    }
    legacy().map(str::to_owned)
}

#[cfg(test)]
#[path = "lookup_display_tests.rs"]
mod tests;
