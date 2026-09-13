use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};

/// A receiver typed by NAME only — the shape the walk holds when no declaration
/// id was ever bound to the head.
fn named_receiver(arena: &TypeArena, head: &str) -> Receiver {
    Receiver {
        ty: arena.class(head),
        id: None,
    }
}

#[test]
fn bound_internal_declaration_is_blamed_for_the_missing_member() {
    let arena = TypeArena::new();
    let spin = sym(2, "spin", "app.Widget.spin", "method", "src/widget.ts");
    let lookup = Lookup::new()
        .with(sym(1, "Widget", "app.Widget", "class", "src/widget.ts"))
        .with_member("app.Widget", spin);

    let cause = classify(
        &lookup,
        &arena,
        Receiver {
            ty: arena.class("app.Widget"),
            id: Some(1),
        },
    )
    .expect("a bound receiver always carries a cause");
    assert_eq!(cause.kind, CauseKind::MemberMissing);
    assert_eq!(cause.symbol_id, Some(1));
}

#[test]
fn unbound_head_with_one_indexed_type_blames_that_declaration() {
    // The head names a type the index holds under its qualified name, so the
    // declaration exists and no rung reached it from the use site.
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(
        7,
        "Widget",
        "com.example.lib.Widget",
        "class",
        "ext:mvn:Widget.java",
    ));

    let cause = classify(&lookup, &arena, named_receiver(&arena, "Widget"))
        .expect("an unbound head carries a cause");
    assert_eq!(cause.kind, CauseKind::DefinedUnimported);
    assert_eq!(cause.symbol_id, Some(7));
}

#[test]
fn unbound_head_with_several_indexed_types_blames_none_of_them() {
    let arena = TypeArena::new();
    let lookup = Lookup::new()
        .with(sym(
            7,
            "Widget",
            "com.example.lib.Widget",
            "class",
            "ext:mvn:a/Widget.java",
        ))
        .with(sym(
            8,
            "Widget",
            "com.other.Widget",
            "class",
            "ext:mvn:b/Widget.java",
        ));

    let cause = classify(&lookup, &arena, named_receiver(&arena, "Widget"))
        .expect("an unbound head carries a cause");
    assert_eq!(cause.kind, CauseKind::DefinedUnimported);
    assert_eq!(cause.symbol_id, None);
}

#[test]
fn unbound_head_the_index_holds_no_type_for_is_missing_supply() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();

    let cause = classify(&lookup, &arena, named_receiver(&arena, "StackPane"))
        .expect("an unbound head carries a cause");
    assert_eq!(cause.kind, CauseKind::NameUnknown);
    assert_eq!(cause.symbol_id, None);
}

#[test]
fn a_value_row_sharing_the_head_name_is_not_a_type_declaration() {
    // Only TYPE declarations can receive a member; a same-named value row is
    // no evidence that the head's declaration is indexed.
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(3, "Widget", "app.widget", "variable", "src/app.ts"));

    let cause = classify(&lookup, &arena, named_receiver(&arena, "Widget"))
        .expect("an unbound head carries a cause");
    assert_eq!(cause.kind, CauseKind::NameUnknown);
}

#[test]
fn qualified_unbound_head_is_classified_by_its_leaf() {
    let arena = TypeArena::new();
    let lookup = Lookup::new().with(sym(
        9,
        "Widget",
        "com.example.lib.Widget",
        "class",
        "ext:mvn:Widget.java",
    ));

    let cause = classify(&lookup, &arena, named_receiver(&arena, "lib.Widget"))
        .expect("an unbound head carries a cause");
    assert_eq!(cause.kind, CauseKind::DefinedUnimported);
    assert_eq!(cause.symbol_id, Some(9));
}
