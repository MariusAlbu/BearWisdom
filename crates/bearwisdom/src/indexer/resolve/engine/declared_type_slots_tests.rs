use std::sync::Arc;

use super::write;
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::type_checker::core::types::{Type, TypeArena};

fn compilation() -> Compilation {
    Compilation::empty(Arc::new(TypeArena::new()))
}

/// A readable annotation fills both slots; an unreadable one leaves them for
/// the ref-derived pass to fill.
#[test]
fn an_unknown_annotation_never_occupies_the_slot() {
    let mut c = compilation();
    let known = c.arena.intern(Type::Class("QueryClient".to_string()));
    let unknown = c.arena.intern(Type::Unknown);
    write(
        &mut c,
        vec![
            ("app.queryClient".to_string(), 7, known),
            ("app.expect".to_string(), 8, unknown),
        ],
    );
    assert_eq!(c.type_info["app.queryClient"].field_type_id, Some(known));
    assert_eq!(c.type_info_by_id[&7].field_type_id, Some(known));
    assert!(!c.type_info.contains_key("app.expect"));
    assert!(!c.type_info_by_id.contains_key(&8));
}
