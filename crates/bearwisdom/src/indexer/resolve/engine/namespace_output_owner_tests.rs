use super::*;

fn cases() -> Vec<(&'static str, &'static [&'static str], bool)> {
    vec![
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make(p: &b::Doc) -> Alias { loop {} } fn f(p: &b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a>(p: &'a b::Doc) -> Alias<'_> { loop {} } fn f(p: &b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make(p: &'static b::Doc) -> Alias<'_> { loop {} } fn f(p: &'static b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>; impl<'a,T> Alias<'a,T> { fn make(&self) -> T { loop {} } }
            fn build<T>(p: &T) -> Alias<T> { loop {} } fn f(p: &b::Doc) { build(p).make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>; impl<'a,T> Alias<'a,T> { fn make(&self) -> Self { loop {} } }
            fn build<T>(p: &T) -> Alias<'_,T> { loop {} } fn f(p: &b::Doc) { build(p).make().inner.touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make(p: &'_ b::Doc) -> Item<&'_ b::Doc> { loop {} } fn f(p: &b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Pair<A,B> { a: A, b: B } type Alias<'a,'b> = Pair<&'a a::Doc,&'b b::Doc>;
            impl<'a,'b> Alias<'a,'b> { fn touch(&self) {} } fn make(p: &b::Doc) -> Alias { loop {} }
            fn f(p: &b::Doc) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a>(p: &'a b::Doc, q: &'a b::Doc) -> Alias<'_> { loop {} }
            fn f<'a>(p: &'a b::Doc, q: &'a b::Doc) { make(p,q).touch(); }", &["make"], false),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make(p: Alias<'_>) -> Alias<'_> { p } fn f(p: Alias<'_>) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            type Hidden = &'static b::Doc; fn make(p: Hidden) -> Alias<'_> { loop {} }
            fn f(p: Hidden) { make(p).touch(); }", &["make"], false),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make(p: &b::Doc) -> Alias<'_> { loop {} } fn f(p: &b::Doc) { let q = make(p); q.touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            struct C; impl C { fn make(p: &b::Doc) -> Alias<'_> { loop {} } }
            fn f(p: &b::Doc) { C::make(p).touch(); }", &["C.make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make(p: &b::Doc, q: &b::Doc) -> Alias<'_> { loop {} }
            fn f(p: &b::Doc, q: &b::Doc) { make(p,q).touch(); }", &["make"], false),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make() -> Alias<'_> { loop {} } fn f() { make().touch(); }", &["make"], false),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            type Erased<'a> = b::Doc; fn make(p: Erased<'_>) -> Alias<'_> { loop {} }
            fn f(p: Erased<'_>) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a>(p: &'a b::Doc, q: &'a b::Doc) -> Alias<'a> { loop {} }
            fn f<'a>(p: &'a b::Doc, q: &'a b::Doc) { make(p,q).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a>(p: (&'a b::Doc,&'a b::Doc)) -> Alias<'_> { loop {} }
            fn f<'a>(p: (&'a b::Doc,&'a b::Doc)) { make(p).touch(); }", &["make", "Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a,'b>(p: (&'a b::Doc,&'b b::Doc)) -> Alias<'_> { loop {} }
            fn f<'a>(p: (&'a b::Doc,&'a b::Doc)) { make(p).touch(); }", &["make"], false),
        ("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
            fn make<'a,'b>(p: &'a b::Doc, q: &'b b::Doc) -> Alias<'_> { loop {} }
            fn f<'a>(p: &'a b::Doc, q: &'a b::Doc) { make(p,q).touch(); }", &["make"], false),
    ]
}

#[test]
fn output_elision_preserves_exact_cascade_targets_and_ambiguity_barriers_fresh_and_cold() {
    for (source, expected, _) in cases() {
        check_locals(source, expected);
    }
}

const RECEIVER_OUTPUT: &str = "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
    impl<'a> Alias<'a> { fn touch(&self) {} } struct C;
    impl C { fn make(&self, other: &b::Doc) -> Alias<'_> { loop {} } }
    fn f(p: &C, other: &b::Doc) { p.make(other).touch(); }";

#[test]
fn alias_owner_receiver_output_probe_requires_borrow_provenance() {
    check_locals(RECEIVER_OUTPUT, &["C.make", "Alias.touch"]);
}

#[path = "namespace_receiver_owner_tests.rs"]
mod receivers;

#[test]
#[ignore = "independent output-elision acceptance/rejection checks; requires rustc"]
fn output_elision_fixtures_agree_with_rustc() {
    let mut mismatches = Vec::new();
    for (source, _, allowed) in
        cases()
            .into_iter()
            .chain([(RECEIVER_OUTPUT, &["C.make", "Alias.touch"][..], true)])
    {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(&input, format!("{LOCAL_TYPES} {source}")).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "output_fixture",
                "--crate-type",
                "lib",
                "--edition=2021",
                "--emit=metadata",
                "--cap-lints=allow",
            ])
            .arg(&input)
            .arg("-o")
            .arg(dir.path().join("lib.rmeta"))
            .output()
            .expect("rustc required");
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        if output.status.success() != allowed || (!allowed && !diagnostics.contains("E0106")) {
            mismatches.push(format!(
                "expected allowed={allowed}: {source}\n{diagnostics}"
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
