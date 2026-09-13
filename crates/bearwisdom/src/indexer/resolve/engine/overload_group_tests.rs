use super::super::contract::SymbolLookup;
use super::super::member_selection::{select, select_typed, select_typed_exact, Selection};
use super::super::testkit::{sym, Lookup};

/// Two callable rows on ONE owner are one member under two signatures, so the
/// group's lowest id represents it.
#[test]
fn one_owners_callable_rows_collapse_to_their_lowest_id() {
    let lookup = Lookup::new()
        .with_member_id(1, sym(72, "append", "Buf.append", "method", "buf.rs"))
        .with_member_id(1, sym(71, "append", "Buf.append", "method", "buf.rs"))
        .with_member_id(1, sym(73, "append", "Buf.append", "method", "buf.rs"));
    let name = lookup.member_index().unwrap().name("append").unwrap();
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Unique(71));
}

/// A field/property sharing the name is a homonym, not an overload: only the
/// hop's kind context may separate it, so the level stays ambiguous.
#[test]
fn a_non_callable_row_keeps_the_level_ambiguous() {
    let lookup = Lookup::new()
        .with_member_id(1, sym(71, "value", "Cfg.value", "method", "cfg.rs"))
        .with_member_id(1, sym(72, "value", "Cfg.value", "property", "cfg.rs"));
    let name = lookup.member_index().unwrap().name("value").unwrap();
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Ambiguous);
}

/// Rows reaching one level from TWO supertypes name no single member — nothing
/// at that level says which the receiver meant.
#[test]
fn rows_from_two_owners_at_one_level_stay_ambiguous() {
    let lookup = Lookup::new()
        .with_parent_id(1, 2)
        .with_parent_id(1, 3)
        .with_member_id(2, sym(71, "read", "Left.read", "method", "left.rs"))
        .with_member_id(3, sym(72, "read", "Right.read", "method", "right.rs"));
    let name = lookup.member_index().unwrap().name("read").unwrap();
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Ambiguous);
}

/// The receiver's own overload set answers the hop, so the climb never reaches
/// the base and a base member of the same name cannot surface.
#[test]
fn an_owners_overload_set_still_hides_a_base_member() {
    let lookup = Lookup::new()
        .with_parent_id(1, 2)
        .with_member_id(2, sym(70, "read", "Base.read", "method", "base.rs"))
        .with_member_id(1, sym(71, "read", "Doc.read", "method", "doc.rs"))
        .with_member_id(1, sym(72, "read", "Doc.read", "method", "doc.rs"));
    let name = lookup.member_index().unwrap().name("read").unwrap();
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Unique(71));
}

/// The kind predicate runs before the collapse: rows it rejects never join the
/// group, so a call hop on a property/method pair selects among the methods.
#[test]
fn the_kind_predicate_narrows_the_group_before_it_collapses() {
    use crate::type_checker::profile::chain_specs::kind_ok;
    use crate::types::EdgeKind;

    let lookup = Lookup::new()
        .with_member_id(1, sym(71, "messages", "V.messages", "property", "v.php"))
        .with_member_id(1, sym(72, "messages", "V.messages", "method", "v.php"))
        .with_member_id(1, sym(73, "messages", "V.messages", "method", "v.php"));
    let name = lookup.member_index().unwrap().name("messages").unwrap();
    let table = crate::languages::php::PHP_PROFILE.kind_compatible_table;
    let calls = |kind: &str| kind_ok(table, EdgeKind::Calls, kind);
    assert_eq!(select(&lookup, 1, name, &calls), Selection::Unique(72));
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Ambiguous);
}

/// The typed entry reaches the same decision, so the chain walker's member hop
/// binds an overload set instead of declining it.
#[test]
fn the_typed_entry_collapses_an_overload_set_for_the_chain_walk() {
    let lookup = Lookup::new()
        .with_member_id(1, sym(81, "add", "List.add", "method", "list.rs"))
        .with_member_id(1, sym(80, "add", "List.add", "method", "list.rs"));
    let name = lookup.member_index().unwrap().name("add").unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let recv = super::super::chain::Receiver::new(
        crate::languages::type_text::intern_test_type_text(&arena, "List"),
        1,
    );
    assert_eq!(
        select_typed(&lookup, &arena, recv, name, &|_| true),
        Selection::Unique(80)
    );
    let walked = super::super::implicit_root::walk_member(
        &lookup,
        &arena,
        recv,
        "add",
        &crate::type_checker::profile::language_profile::DEFAULT_PROFILE,
        &|_| true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(walked.id, 80);
}

/// The exact entry reports the set so a caller holding the call's arguments
/// still selects over it; the representing entry answers with a row.
#[test]
fn the_exact_entry_reports_the_set_the_representing_entry_answers() {
    let lookup = Lookup::new()
        .with_member_id(1, sym(81, "add", "List.add", "method", "list.rs"))
        .with_member_id(1, sym(80, "add", "List.add", "method", "list.rs"));
    let name = lookup.member_index().unwrap().name("add").unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let recv = super::super::chain::Receiver::new(
        crate::languages::type_text::intern_test_type_text(&arena, "List"),
        1,
    );
    assert_eq!(
        select_typed_exact(&lookup, &arena, recv, name, &|_| true),
        Selection::Ambiguous
    );
    assert_eq!(
        select_typed(&lookup, &arena, recv, name, &|_| true),
        Selection::Unique(80)
    );
}

/// A candidate the lookup cannot hydrate leaves the group unproven, so the
/// level declines rather than committing to a row it could not read.
#[test]
fn an_unreadable_candidate_leaves_the_group_unselected() {
    let lookup = Lookup::new();
    let mut candidates = rustc_hash::FxHashSet::default();
    candidates.insert(91);
    candidates.insert(92);
    assert_eq!(super::select(&lookup, &candidates, true), None);
}
