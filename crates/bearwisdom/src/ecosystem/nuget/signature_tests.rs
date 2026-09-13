use super::return_type;

#[test]
fn decodes_the_return_slot_after_the_parameter_list() {
    assert_eq!(
        return_type("Greet(string, int): string"),
        Some("string".into())
    );
    assert_eq!(
        return_type("Take<T>(this System.Collections.Generic.IEnumerable<T>, int): System.Collections.Generic.IEnumerable<T>"),
        Some("System.Collections.Generic.IEnumerable<T>".into())
    );
}

#[test]
fn a_constructor_returns_its_declaring_type_even_when_the_name_repeats() {
    assert_eq!(
        return_type("Dictionary<TKey, TValue>(int): System.Collections.Generic.Dictionary"),
        Some("System.Collections.Generic.Dictionary".into())
    );
    assert_eq!(
        return_type("Greeter(string): FakeExt.Greeter"),
        Some("FakeExt.Greeter".into())
    );
}

#[test]
fn nested_parentheses_in_a_parameter_do_not_end_the_list() {
    assert_eq!(
        return_type("Apply(System.Func<(int, int), bool>): bool"),
        Some("bool".into())
    );
}

#[test]
fn void_and_shapes_without_a_return_slot_decode_to_nothing() {
    assert_eq!(return_type("Run(): void"), None);
    assert_eq!(return_type("Run(): System.Void"), None);
    assert_eq!(return_type("string Run(int)"), None);
    assert_eq!(
        return_type("(Ljava/lang/String;)Lcom/example/Result;"),
        None
    );
    assert_eq!(return_type("Run("), None);
}
