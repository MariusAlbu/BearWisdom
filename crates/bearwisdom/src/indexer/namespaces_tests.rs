use super::*;

#[test]
fn same_spelling_is_scoped_and_namespace_domains_are_independent() {
    let mut data = NamespaceData::default();
    let root = data.graph.add_scope(None, 0, 100, true);
    data.units.push(Unit {
        parent: None,
        scope: root,
        name: None,
        path: None,
        range: (0, 100),
    });
    data.scope_units.insert(root, SourceModuleId(0));
    let name = data.graph.intern("Item");
    let value = data.declare(root, name, ExportDomain::Value, Target::Declaration(7));
    let ty = data.declare(root, name, ExportDomain::Type, Target::Declaration(8));
    assert_ne!(value, ty);
    assert!(
        matches!(data.lookup(root, name, ExportDomain::Type), Some(Target::Binding(id)) if id == ty)
    );
}
