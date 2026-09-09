use super::super::testkit::{sym, Lookup};
use super::*;

struct Restricted {
    inner: Lookup,
    denied: i64,
}
impl super::super::contract::FlowCacheLookup for Restricted {
    fn declaration_accessible(&self, id: i64) -> bool {
        id != self.denied
    }
}
impl SymbolLookup for Restricted {
    fn by_name(&self, name: &str) -> super::super::contract::SymbolSet<'_> {
        self.inner.by_name(name)
    }
    fn by_qualified_name(&self, name: &str) -> Option<&super::super::contract::Symbol> {
        self.inner.by_qualified_name(name)
    }
    fn members_of(&self, name: &str) -> super::super::contract::SymbolSet<'_> {
        self.inner.members_of(name)
    }
    fn types_by_name(&self, name: &str) -> super::super::contract::SymbolSet<'_> {
        self.inner.types_by_name(name)
    }
    fn in_namespace(&self, name: &str) -> Vec<&super::super::contract::Symbol> {
        self.inner.in_namespace(name)
    }
    fn has_in_namespace(&self, name: &str) -> bool {
        self.inner.has_in_namespace(name)
    }
    fn in_file(&self, file: &str) -> super::super::contract::SymbolSet<'_> {
        self.inner.in_file(file)
    }
    fn field_type_name(&self, name: &str) -> Option<&str> {
        self.inner.field_type_name(name)
    }
    fn return_type_name(&self, name: &str) -> Option<&str> {
        self.inner.return_type_name(name)
    }
    fn generic_params(&self, name: &str) -> Option<Vec<String>> {
        self.inner.generic_params(name)
    }
    fn reexports_from(&self, file: &str) -> &[(String, String)] {
        self.inner.reexports_from(file)
    }
    fn is_external_name(&self, name: &str, language: &str) -> bool {
        self.inner.is_external_name(name, language)
    }
    fn member_index(&self) -> Option<&super::super::member_index::MemberIndex> {
        self.inner.member_index()
    }
    fn parent_class_ids(&self, id: i64) -> Vec<i64> {
        self.inner.parent_class_ids(id)
    }
    fn symbol_by_id(&self, id: i64) -> Option<&super::super::contract::Symbol> {
        self.inner.symbol_by_id(id)
    }
}

#[test]
fn inaccessible_members_and_owners_remain_barriers_to_base_and_outer_fallbacks() {
    let inner = Lookup::new()
        .with_parent_id(1, 2)
        .with_member_id(1, sym(71, "read", "Doc.read", "method", "private.rs"))
        .with_member_id(2, sym(72, "read", "Base.read", "method", "public.rs"));
    let mut lookup = Restricted { inner, denied: 71 };
    let name = lookup.member_index().unwrap().name("read").unwrap();
    let arena = crate::type_checker::core::types::TypeArena::new();
    let recv = super::super::chain::Receiver::new(arena.intern_type_str("Doc"), 1);
    for denied in [71, 1] {
        lookup.denied = denied;
        assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Inaccessible);
        assert!(matches!(
            super::super::implicit_root::walk_member(
                &lookup,
                &arena,
                recv,
                "read",
                &crate::type_checker::profile::language_profile::DEFAULT_PROFILE
            ),
            Err(Selection::Inaccessible)
        ));
    }
    lookup.denied = 0;
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Unique(71));
    lookup.inner = lookup
        .inner
        .with_member_id(1, sym(73, "read", "Doc.read", "method", "other.rs"));
    assert!(matches!(
        super::super::implicit_root::walk_member(
            &lookup,
            &arena,
            recv,
            "read",
            &crate::type_checker::profile::language_profile::DEFAULT_PROFILE
        ),
        Err(Selection::Ambiguous)
    ));
}

#[test]
fn diamonds_deduplicate_identity_and_direct_declarations_hide_bases() {
    let lookup = Lookup::new()
        .with_parent_id(1, 2)
        .with_parent_id(1, 3)
        .with_parent_id(2, 4)
        .with_parent_id(3, 4)
        .with_member_id(4, sym(71, "read", "Doc.read", "method", "base.rs"));
    let name = lookup.member_index().unwrap().name("read").unwrap();
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Unique(71));
    let lookup = lookup.with_member_id(1, sym(72, "read", "Doc.read", "method", "child.rs"));
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Unique(72));
}

#[test]
fn direct_conflicts_and_kind_rejections_do_not_expose_a_base_member() {
    let lookup = Lookup::new()
        .with_parent_id(1, 2)
        .with_member_id(2, sym(70, "read", "Doc.read", "method", "base.rs"))
        .with_member_id(1, sym(71, "read", "Doc.read", "method", "child.rs"))
        .with_member_id(1, sym(72, "read", "Doc.read", "method", "child.rs"));
    let name = lookup.member_index().unwrap().name("read").unwrap();
    assert_eq!(select(&lookup, 1, name, &|_| true), Selection::Ambiguous);
    assert_eq!(select(&lookup, 1, name, &|_| false), Selection::Missing);
}
