use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;

fn app(root: &str, args: &[&str]) -> AliasTarget {
    AliasTarget::Application {
        root: root.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn expands_non_generic_alias_to_its_target() {
    // type UserMap = Map<string, User>
    let lookup = Lookup::new().with_alias("UserMap", app("Map", &["string", "User"]));
    let out = expand(TypeSymbol::plain("UserMap"), &lookup);
    assert_eq!(out.qname, "Map");
    assert_eq!(
        out.type_args,
        vec![TypeSymbol::plain("string"), TypeSymbol::plain("User")]
    );
}

#[test]
fn expands_generic_alias_substituting_its_parameter() {
    // type Box<T> = Container<T>;  Box<User> → Container<User>
    let lookup = Lookup::new()
        .with_alias("Box", app("Container", &["T"]))
        .with_generics("Box", &["T"]);
    let out = expand(
        TypeSymbol {
            qname: "Box".to_string(),
            type_args: vec![TypeSymbol::plain("User")],
        },
        &lookup,
    );
    assert_eq!(out.qname, "Container");
    assert_eq!(out.type_args, vec![TypeSymbol::plain("User")]);
}

#[test]
fn leaves_a_non_alias_unchanged() {
    let lookup = Lookup::new();
    let t = TypeSymbol::plain("User");
    assert_eq!(expand(t.clone(), &lookup), t);
}

#[test]
fn follows_an_alias_of_an_alias() {
    // type A = B;  type B = Map<K, V>   →   A expands through to Map
    let lookup = Lookup::new()
        .with_alias("A", app("B", &[]))
        .with_alias("B", app("Map", &["K", "V"]));
    let out = expand(TypeSymbol::plain("A"), &lookup);
    assert_eq!(out.qname, "Map");
}
