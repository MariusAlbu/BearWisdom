use super::svelte_store_base;

#[test]
fn store_subscription_strips_dollar() {
    assert_eq!(svelte_store_base("$t"), Some("t"));
    assert_eq!(svelte_store_base("$count"), Some("count"));
    assert_eq!(
        svelte_store_base("$optionClickCallbackStore"),
        Some("optionClickCallbackStore")
    );
    assert_eq!(svelte_store_base("$_underscore"), Some("_underscore"));
}

#[test]
fn runes_are_not_stores() {
    for rune in [
        "$state",
        "$derived",
        "$effect",
        "$props",
        "$bindable",
        "$inspect",
        "$host",
    ] {
        assert_eq!(
            svelte_store_base(rune),
            None,
            "{rune} is a rune, not a store"
        );
    }
}

#[test]
fn double_dollar_specials_are_not_stores() {
    assert_eq!(svelte_store_base("$$props"), None);
    assert_eq!(svelte_store_base("$$restProps"), None);
    assert_eq!(svelte_store_base("$$slots"), None);
}

#[test]
fn non_store_shapes_return_none() {
    assert_eq!(svelte_store_base("t"), None, "no leading $");
    assert_eq!(svelte_store_base("$"), None, "bare $");
    assert_eq!(
        svelte_store_base("$1abc"),
        None,
        "must start with letter/underscore"
    );
    assert_eq!(
        svelte_store_base("$page.url"),
        None,
        "dotted is not a bare identifier"
    );
}
