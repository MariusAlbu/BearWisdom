use crate::types::SymbolKind;

#[test]
fn forward_and_nested_nominal_owners_are_bound_without_qname_round_trips() {
    let source = "mod left { impl Model { fn save() {} } struct Model; } mod right { struct Model; impl Model { fn save() {} } }";
    let parsed = super::super::extract::extract(source);
    let members: Vec<_> = parsed
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .collect();
    assert_eq!(members.len(), 2);
    let owners: Vec<_> = members
        .iter()
        .map(|s| s.parent_index.expect("exact nominal owner"))
        .collect();
    assert_ne!(owners[0], owners[1]);
    for owner in owners {
        assert_eq!(parsed.symbols[owner].kind, SymbolKind::Struct);
    }
}

#[test]
fn generic_inherent_impl_targets_the_nominal_not_its_parameter() {
    let parsed = super::super::extract::extract(
        "struct Model<T>(T); impl<T> Model<T> { fn save(&self) {} }",
    );
    let member = parsed
        .symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Method)
        .unwrap();
    let owner = &parsed.symbols[member.parent_index.unwrap()];
    assert_eq!(owner.kind, SymbolKind::Struct);
    assert_eq!(owner.name, "Model");
}

#[test]
fn generic_and_alias_shadows_never_attach_impls_to_outer_nominal_namesakes() {
    for source in [
        "struct T; impl<T> T { fn save() {} }",
        "struct Model; mod inner { type Model = u8; impl Model { fn save() {} } }",
        "struct Model; mod inner { impl Model { fn save() {} } }",
        "struct Model; fn f() { use foreign::Model; impl Model { fn save() {} } }",
        "struct Model; struct Model; impl Model { fn save() {} }",
        "mod other { pub struct Model; } struct Model; impl other::Model { fn save() {} }",
    ] {
        let parsed = super::super::extract::extract(source);
        let members: Vec<_> = parsed.symbols.iter().filter(|s| s.name == "save").collect();
        assert_eq!(members.len(), 1, "{source}");
        assert_eq!(members[0].kind, SymbolKind::Method, "{source}");
        assert_eq!(members[0].parent_index, None, "{source}");
    }
}

#[test]
fn trait_impl_parent_is_the_source_container_not_the_nominal_namesake() {
    let source = "struct Model; impl ForeignTrait for Model { fn save() {} }";
    let parsed = super::super::extract::extract(source);
    let nominal = parsed
        .symbols
        .iter()
        .position(|s| s.kind == SymbolKind::Struct)
        .unwrap();
    let container = parsed
        .symbols
        .iter()
        .position(|s| s.start_col == source.find("impl").unwrap() as u32)
        .unwrap();
    let member = parsed.symbols.iter().find(|s| s.name == "save").unwrap();
    assert_eq!(member.parent_index, Some(container));
    assert_eq!(parsed.symbols[container].kind, SymbolKind::Namespace);
    assert_ne!(
        member.parent_index,
        Some(nominal),
        "trait implementation is not inherent ownership"
    );
}

#[test]
fn unrelated_imports_do_not_erase_local_nominal_ownership() {
    for import in [
        "use foreign::Other;",
        "use foreign::Model as Other;",
        "use foreign::{Other, nested::{Item as Renamed}};",
        "use foreign::Model as _;",
        "use foreign::{self, Other};",
        "use foreign::*;",
    ] {
        let source =
            format!("{import} struct Model; impl Model {{ fn save() {{}} fn reload() {{}} }}");
        let parsed = super::super::extract::extract(&source);
        let owner = parsed
            .symbols
            .iter()
            .position(|s| s.kind == SymbolKind::Struct)
            .unwrap();
        let members: Vec<_> = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(members.len(), 2, "{source}");
        for member in members {
            assert_eq!(member.parent_index, Some(owner), "{source}");
        }
    }
}

#[test]
fn function_local_impl_members_have_nominal_kind_and_owner_ids() {
    let source = "fn f() { use foreign::Other; struct Model; impl Model { fn save(&self) {} fn reload(&self) {} } }";
    let parsed = super::super::extract::extract(source);
    let owner = parsed
        .symbols
        .iter()
        .position(|s| s.kind == SymbolKind::Struct)
        .unwrap();
    let members: Vec<_> = parsed
        .symbols
        .iter()
        .filter(|s| s.name == "save" || s.name == "reload")
        .collect();
    assert_eq!(members.len(), 2);
    for member in members {
        assert_eq!(member.kind, SymbolKind::Method);
        assert_eq!(member.parent_index, Some(owner));
    }
}

#[test]
fn explicit_import_names_are_scope_barriers_even_when_declared_after_the_impl() {
    for import in [
        "use foreign::Model;",
        "use foreign::Other as Model;",
        "use foreign::{Other as Model};",
        "use foreign::{Model::{self}};",
        "use foreign::*;",
    ] {
        let source = format!("struct Model; fn f() {{ impl Model {{ fn save() {{}} }} {import} }}");
        let parsed = super::super::extract::extract(&source);
        let members: Vec<_> = parsed.symbols.iter().filter(|s| s.name == "save").collect();
        assert_eq!(members.len(), 1, "{source}");
        assert_eq!(members[0].parent_index, None, "{source}");
    }
}

#[test]
fn raw_import_names_and_modules_cannot_borrow_outer_nominal_owners() {
    for declaration in [
        "use foreign::r#Model;",
        "use foreign::Other as r#Model;",
        "mod Model {}",
        "use ;",
    ] {
        let source =
            format!("struct Model; fn f() {{ {declaration} impl Model {{ fn save() {{}} }} }}");
        let parsed = super::super::extract::extract(&source);
        let members: Vec<_> = parsed.symbols.iter().filter(|s| s.name == "save").collect();
        assert_eq!(members.len(), 1, "{source}");
        assert_eq!(members[0].parent_index, None, "{source}");
    }
    let parsed = super::super::extract::extract("struct r#Model; impl Model { fn save() {} }");
    let member = parsed.symbols.iter().find(|s| s.name == "save").unwrap();
    assert_eq!(
        parsed.symbols[member.parent_index.unwrap()].kind,
        SymbolKind::Struct
    );
}

#[test]
fn imported_names_conflict_with_local_nominals_in_either_source_order() {
    for source in [
        "use foreign::Model; struct Model; impl Model { fn save() {} }",
        "struct Model; impl Model { fn save() {} } use foreign::Model;",
    ] {
        let parsed = super::super::extract::extract(source);
        let member = parsed.symbols.iter().find(|s| s.name == "save").unwrap();
        assert_eq!(member.parent_index, None, "{source}");
    }
}

#[test]
fn unrelated_nested_imports_leave_visible_outer_nominals_accessible() {
    let source = "struct Model; fn f() { use foreign::Other; impl Model { fn save() {} } }";
    let parsed = super::super::extract::extract(source);
    let member = parsed.symbols.iter().find(|s| s.name == "save").unwrap();
    assert_eq!(
        parsed.symbols[member.parent_index.unwrap()].kind,
        SymbolKind::Struct
    );
}
