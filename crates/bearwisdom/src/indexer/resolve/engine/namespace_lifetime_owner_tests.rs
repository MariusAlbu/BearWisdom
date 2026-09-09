use super::*;

pub(super) const ELIDED_ALIAS: &str =
    "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
    impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: Alias<'_>) { p.touch(); }";

pub(super) fn cases() -> Vec<(&'static str, &'static [&'static str], bool)> {
    vec![
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'x,X> Alias<'x,X> { fn touch(&self) {} }
            fn make<'a,T>(p: &'a T) -> Alias<'a,T> { loop {} }
            fn f<'z>(p: &'z b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a a::Doc>;
            impl<'x> Alias<'x> { fn make<'y>(&self, p: &'y b::Doc) -> &'y b::Doc { p } }
            fn f<'z>(p: Alias<'z>, q: &'z b::Doc) { p.make(q).touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'x> Alias<'x> { fn touch(&self) {} } fn f(p: Alias<'static>) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'x> Alias<'x> { fn touch(&self) {} } fn f<'z>(p: Item<&'z b::Doc>) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'x> Alias<'x> { fn touch(&self) {} } fn f<'z>(p: Item<&'z a::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'x,X> Alias<'x,X> { fn make(&self) -> X { loop {} } }
            fn f<'z>(p: Item<&'z b::Doc>) { p.make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'x,X> Alias<'x,X> { fn make(&self) -> &'x X { loop {} } }
            fn f<'z>(p: Item<&'z b::Doc>) { p.make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'x,X> Alias<'x,X> { fn make(&self) -> Self { loop {} } }
            fn f<'z>(p: Item<&'z b::Doc>) { p.make().inner.touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Pair<A,B> { a: A, b: B } type Swap<'a,'b,T,U> = Pair<&'b U,&'a T>;
            impl<'x,'y,X,Y> Swap<'y,'x,Y,X> { fn make(&self) -> &'y Y { loop {} } }
            fn f<'p,'q>(p: Pair<&'p a::Doc,&'q b::Doc>) { p.make().touch(); }", &["Swap.make", "b.Doc.touch"], true),
        ("struct Pair<A,B> { a: A, b: B } type Same<'a> = Pair<&'a b::Doc,&'a b::Doc>;
            impl<'x> Same<'x> { fn touch(&self) {} }
            fn f<'z>(p: Pair<&'z b::Doc,&'z b::Doc>) { p.touch(); }", &["Same.touch"], true),
        ("struct Item<T> { inner: T } type Inner<'a,T> = Item<&'a T>; type Outer<'b,U> = Inner<'b,U>;
            impl<'x,X> Outer<'x,X> { fn make(&self) -> Self { loop {} } }
            fn f<'z>(p: Outer<'z,b::Doc>) { p.make().inner.touch(); }", &["Outer.make", "b.Doc.touch"], true),
        ("struct Item<'a,T> { inner: &'a T } impl<'x,X> Item<'x,X> { fn make(&self) -> &'x X { loop {} } }
            fn f<'z>(p: Item<'z,b::Doc>) { p.make().touch(); p.inner.touch(); }", &["Item.make", "b.Doc.touch", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'x> Alias<'x> { fn touch(&self) {} }
            fn f<'a>(p: Alias<'a>) { p.touch(); } fn g<'a>(p: Alias<'a>) { p.touch(); }", &["Alias.touch", "Alias.touch"], true),
    ]
}

#[test]
fn named_lifetimes_preserve_member_field_and_yield_ids_fresh_and_cold() {
    for (source, expected, _) in cases() {
        check_locals(source, expected);
    }
}

#[test]
fn unsupported_lifetime_evidence_does_not_select_an_alias_impl() {
    for source in [
        "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: Alias<b::Doc>) { p.touch(); }",
        "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<X> Alias<X> { fn touch(&self) {} } fn f(p: Item<&'static b::Doc>) { p.touch(); }",
        "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a: 'static> Alias<'a> { fn touch(&self) {} } fn f(p: Alias<'static>) { p.touch(); }",
    ] { check_locals(source, &[]); }
}

// Compiler-legal regression retained as a positive target expectation; do not
// convert this into an abstention success when reporting resolution progress.
#[test]
fn alias_owner_elided_lifetime_probe_requires_region_inference() {
    check_locals(ELIDED_ALIAS, &["Alias.touch"]);
}

pub(super) const ELIDED_OUTPUT: &str =
    "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
    impl<'a> Alias<'a> { fn touch(&self) {} } fn make(p: &b::Doc) -> Alias<'_> { loop {} }
    fn f(p: &b::Doc) { make(p).touch(); }";

#[test]
fn alias_owner_elided_output_probe_requires_input_output_region_relationship() {
    check_locals(ELIDED_OUTPUT, &["make", "Alias.touch"]);
}

pub(super) fn elided_cases() -> Vec<(&'static str, &'static [&'static str], bool)> {
    vec![
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: Alias<'_>) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: Item<&b::Doc>) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: Alias) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'a,T> Alias<'a,T> { fn make(&self) -> T { loop {} } }
            fn f(p: Alias<b::Doc>) { p.make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'a,T> Alias<'a,T> { fn make(&self) -> Self { loop {} } }
            fn f(p: Alias<'_,b::Doc>) { p.make().inner.touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
            impl<'a,T> Alias<'a,T> { fn make(&self) -> T { loop {} } }
            fn f(p: Item<&'_ b::Doc>) { p.make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Pair<A,B> { a: A, b: B } type Alias<'a,'b> = Pair<&'a a::Doc,&'b b::Doc>;
            impl<'x,'y> Alias<'x,'y> { fn touch(&self) {} }
            fn f(p: Alias<'_,'_>, q: Alias) { p.touch(); q.touch(); }", &["Alias.touch", "Alias.touch"], true),
        ("struct Item<'a,T> { inner: &'a T } impl<'a,T> Item<'a,T> { fn make(&self) -> &'a T { loop {} } }
            fn f(p: Item<b::Doc>) { p.make().touch(); p.inner.touch(); }", &["Item.make", "b.Doc.touch", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a>(p: &'a b::Doc) -> Alias<'a> { loop {} } fn f(p: &b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<T>(p: T) -> T { p } fn f(p: Alias) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("fn make<T>(p: &T) -> T { loop {} } fn f(p: &b::Doc) { make(p).touch(); }", &["make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} }
            fn f(p: Alias) { p.touch(); fn g(p: Alias) { p.touch(); } }", &["Alias.touch", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
            impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: Item<&a::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; type i32<'a> = Alias<'a>;
            impl<'a> Alias<'a> { fn touch(&self) {} } fn f(p: i32) { p.touch(); }", &["Alias.touch"], true),
    ]
}

#[test]
fn elided_inputs_and_omitted_slots_preserve_exact_member_and_yield_ids_fresh_and_cold() {
    for (source, expected, _) in elided_cases() {
        check_locals(source, expected);
    }
}
