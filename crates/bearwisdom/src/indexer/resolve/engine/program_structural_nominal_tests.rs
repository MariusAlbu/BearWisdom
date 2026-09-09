use super::super::super::tests::with_relation;
use super::*;

#[test]
fn an_unattested_nominal_does_not_become_an_empty_object_surface() {
    with_relation(|relation| {
        let ty = relation
            .arena
            .decl_in(relation.lookup.view.context, "unattested", 999);
        assert!(Eval::new(relation).nominal_members(ty, 0).is_none());
        assert!(Eval::new(relation).key_properties(ty, 0).is_none());
    });
}
