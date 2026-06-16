use super::*;

#[test]
fn parse_plain_type_has_no_args() {
    assert_eq!(TypeSymbol::parse("User"), TypeSymbol::plain("User"));
}

#[test]
fn parse_generic_splits_head_and_arg() {
    let t = TypeSymbol::parse("Repository<User>");
    assert_eq!(t.qname, "Repository");
    assert_eq!(t.type_args, vec![TypeSymbol::plain("User")]);
}

#[test]
fn parse_nested_application_recurses() {
    let t = TypeSymbol::parse("Map<K, List<V>>");
    assert_eq!(t.qname, "Map");
    assert_eq!(t.type_args.len(), 2);
    assert_eq!(t.type_args[0], TypeSymbol::plain("K"));
    assert_eq!(t.type_args[1].qname, "List");
    assert_eq!(t.type_args[1].type_args, vec![TypeSymbol::plain("V")]);
}

#[test]
fn substitute_replaces_bare_parameter_whole() {
    let t = TypeSymbol::plain("T");
    let out = t.substitute(&["T".to_string()], &[TypeSymbol::plain("User")]);
    assert_eq!(out, TypeSymbol::plain("User"));
}

#[test]
fn substitute_recurses_into_type_arguments() {
    // List<T> with T→User → List<User>
    let t = TypeSymbol {
        qname: "List".to_string(),
        type_args: vec![TypeSymbol::plain("T")],
    };
    let out = t.substitute(&["T".to_string()], &[TypeSymbol::plain("User")]);
    assert_eq!(out.qname, "List");
    assert_eq!(out.type_args, vec![TypeSymbol::plain("User")]);
}

#[test]
fn substitute_leaves_non_parameter_heads_untouched() {
    let t = TypeSymbol::plain("User");
    let out = t.clone().substitute(&["T".to_string()], &[TypeSymbol::plain("X")]);
    assert_eq!(out, t);
}

#[test]
fn substitute_maps_each_parameter_to_its_aligned_argument() {
    // Map<K, V> with K→string, V→User → Map<string, User>
    let t = TypeSymbol {
        qname: "Map".to_string(),
        type_args: vec![TypeSymbol::plain("K"), TypeSymbol::plain("V")],
    };
    let out = t.substitute(
        &["K".to_string(), "V".to_string()],
        &[TypeSymbol::plain("string"), TypeSymbol::plain("User")],
    );
    assert_eq!(
        out.type_args,
        vec![TypeSymbol::plain("string"), TypeSymbol::plain("User")]
    );
}
