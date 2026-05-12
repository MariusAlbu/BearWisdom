use super::PluginStateBag;

#[derive(Debug, Clone, PartialEq)]
struct Foo(u32);

#[derive(Debug, Clone, PartialEq)]
struct Bar(String);

#[derive(Debug, Clone, PartialEq, Default)]
struct Baz(Vec<i32>);

#[test]
fn set_get_round_trip() {
    let mut bag = PluginStateBag::new();
    bag.set(Foo(42));
    assert_eq!(bag.get::<Foo>(), Some(&Foo(42)));
}

#[test]
fn miss_returns_none() {
    let bag = PluginStateBag::new();
    assert_eq!(bag.get::<Foo>(), None);
}

#[test]
fn multiple_types_coexist() {
    let mut bag = PluginStateBag::new();
    bag.set(Foo(1));
    bag.set(Bar("hello".to_string()));
    assert_eq!(bag.get::<Foo>(), Some(&Foo(1)));
    assert_eq!(bag.get::<Bar>(), Some(&Bar("hello".to_string())));
}

#[test]
fn set_overwrites_previous_value() {
    let mut bag = PluginStateBag::new();
    bag.set(Foo(1));
    bag.set(Foo(2));
    assert_eq!(bag.get::<Foo>(), Some(&Foo(2)));
}

#[test]
fn get_or_default_returns_default_for_absent_type() {
    let bag = PluginStateBag::new();
    assert_eq!(bag.get_or_default::<Baz>(), Baz::default());
}

#[test]
fn get_or_default_returns_stored_value() {
    let mut bag = PluginStateBag::new();
    bag.set(Baz(vec![1, 2, 3]));
    assert_eq!(bag.get_or_default::<Baz>(), Baz(vec![1, 2, 3]));
}

#[test]
fn default_constructs_empty_bag() {
    let bag = PluginStateBag::default();
    assert!(bag.get::<Foo>().is_none());
}
