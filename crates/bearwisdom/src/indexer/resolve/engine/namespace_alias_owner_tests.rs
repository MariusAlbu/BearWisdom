use super::*;

#[path = "namespace_lifetime_owner_tests.rs"]
mod lifetimes;
#[path = "namespace_output_owner_tests.rs"]
mod outputs;

const REFERENCE_ALIAS: &str = "struct Item<T> { inner: T } type Alias = Item<&'static b::Doc>;
    impl Alias { pub fn touch(&self) {} } fn f(p: Alias) { p.touch(); }";

#[test]
fn alias_owner_reference_argument_probe_requires_lossless_type_identity() {
    check_locals(REFERENCE_ALIAS, &["Alias.touch"]);
}

fn cases() -> Vec<(&'static str, &'static [&'static str], bool)> {
    vec![
        ("fn f(p: &b::Doc) { p.touch(); }", &["b.Doc.touch"], true),
        ("type Alias = &'static b::Doc; fn f(p: Alias) { p.touch(); }", &["b.Doc.touch"], true),
        ("type Inner = &'static b::Doc; type Outer = &'static Inner;
            fn f(p: Outer) { p.touch(); }", &["b.Doc.touch"], true),
        ("struct Holder { inner: &'static b::Doc } fn f(p: &Holder) { p.inner.touch(); }", &["b.Doc.touch"], true),
        ("struct Holder; impl Holder { fn make(&self) -> &'static b::Doc { loop {} } }
            fn f(p: &Holder) { p.make().touch(); }", &["Holder.make", "b.Doc.touch"], true),
        ("fn make() -> &'static b::Doc { loop {} } fn f() { make().touch(); }", &["make", "b.Doc.touch"], true),
        ("fn make<T>(p: T) -> T { p } fn f(p: &'static b::Doc) { make(p).touch(); }", &["make", "b.Doc.touch"], true),
        ("fn make<T>(p: &'static T) -> T { loop {} } fn f(p: &'static b::Doc) { make(p).touch(); }", &["make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Ref = Item<&'static b::Doc>; type Value = Item<b::Doc>;
            impl Ref { fn touch(&self) {} } impl Value { fn touch(&self) {} }
            fn make<T>(p: T) -> Item<T> { loop {} } fn f(p: &'static b::Doc) { make(p).touch(); }", &["make", "Ref.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<&'static b::Doc>;
            impl Alias { fn make(&self) -> Self { loop {} } }
            fn f(p: &Alias) { p.make().inner.touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("fn f(p: *const b::Doc) { p.touch(); }", &[], false),
        ("type Alias = *mut b::Doc; fn f(p: Alias) { p.touch(); }", &[], false),
        ("fn make() -> *const b::Doc { loop {} } fn f() { make().touch(); }", &["make"], false),
        ("struct Item<T> { inner: T } type Alias = Item<&'static b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<&'static b::Doc>) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<&'static mut b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Alias) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<&'static b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<&'static a::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Read = Item<&'static b::Doc>; type Write = Item<&'static mut b::Doc>;
            impl Read { pub fn touch(&self) {} } impl Write { pub fn touch(&self) {} }
            fn f(p: Read, q: Write) { p.touch(); q.touch(); }", &["Read.touch", "Write.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<&'static b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<&'static mut b::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<*const b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Alias) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Read = Item<*const b::Doc>; type Write = Item<*mut b::Doc>;
            impl Read { pub fn touch(&self) {} } impl Write { pub fn touch(&self) {} }
            fn f(p: Read, q: Write) { p.touch(); q.touch(); }", &["Read.touch", "Write.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<*const b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<*mut b::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<*const b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<&'static b::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<*const b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<*const a::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<(&'static b::Doc, *mut b::Doc)>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Alias) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type DocRef = &'static b::Doc; type Alias = Item<DocRef>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<&'static b::Doc>) { p.touch(); }", &["Alias.touch"], true),
        ("struct Item<T> { inner: T } type Alias<T> = Item<&'static T>;
            impl<X> Alias<X> { pub fn make(&self) -> X { loop {} } }
            fn f(p: Item<&'static b::Doc>) { p.make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<T> = Item<*const T>;
            impl<X> Alias<X> { pub fn make(&self) -> X { loop {} } }
            fn f(p: Item<*const b::Doc>) { p.make().touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item; type Alias = Item; impl Alias { pub fn new() -> Self { Self } pub fn touch(&self) {} }
            fn f() { Item::new().touch(); Alias::new().touch(); }",
            &["Alias.new", "Alias.touch", "Alias.new", "Alias.touch"], true),
        ("struct Item; type First = Second; type Second = Item;
            impl First { pub fn touch(&self) {} } fn f(p: Item) { p.touch(); }", &["First.touch"], true),
        ("struct Pair<A,B> { a: A, b: B } type Swap<T,U> = Pair<U,T>;
            impl<X,Y> Swap<X,Y> { pub fn make(&self) -> X { loop {} } }
            fn f(p: Pair<a::Doc,b::Doc>) { p.make().touch(); }", &["Swap.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<T> = Item<T>;
            impl<X> Alias<X> { pub fn make(&self) -> Self { loop {} } }
            fn f(p: Item<b::Doc>) { p.make().inner.touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias<T> = Item<T>;
            impl<X> Alias<X> { pub fn make<Y>(&self, p: Y) -> Y { p } }
            fn f(p: Item<a::Doc>, q: b::Doc) { p.make(q).touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<b::Doc>;
            impl Alias { pub fn make(&self) -> Self { loop {} } }
            fn f(p: Item<b::Doc>) { p.make().inner.touch(); }", &["Alias.make", "b.Doc.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<b::Doc>;
            impl Alias { pub fn make(&self) -> Self { loop {} } }
            fn f(p: Item<a::Doc>) { p.make().inner.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Signed = Item<i32>; type Unsigned = Item<u32>;
            impl Signed { pub fn touch(&self) {} } impl Unsigned { pub fn touch(&self) {} }
            fn f(p: Item<i32>, q: Item<u32>) { p.touch(); q.touch(); }", &["Signed.touch", "Unsigned.touch"], true),
        ("struct Item<T> { inner: T } type Alias = Item<i32>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<u32>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<i32>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<i64>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<f32>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<f64>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<i32>;
            impl Alias { pub fn touch(&self) {} } fn f<T>(p: Item<T>) { p.touch(); }", &[], false),
        ("struct Pair<A,B> { a: A, b: B } type Same<T> = Pair<T,T>;
            impl<T> Same<T> { pub fn make(&self) -> T { loop {} } }
            fn f(p: Pair<b::Doc,b::Doc>) { p.make().touch(); }", &["Same.make", "b.Doc.touch"], true),
        ("struct Pair<A,B> { a: A, b: B } type Same<T> = Pair<T,T>;
            impl<T> Same<T> { pub fn make(&self) -> T { loop {} } }
            fn f(p: Pair<a::Doc,b::Doc>) { p.make().touch(); }", &[], false),
        ("struct Item; type Alias = Alias; impl Alias { pub fn touch(&self) {} }
            fn f(p: Item) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<&'static b::Doc>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<b::Doc>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } type Alias = Item<(b::Doc,)>;
            impl Alias { pub fn touch(&self) {} } fn f(p: Item<(&'static b::Doc,)>) { p.touch(); }", &[], false),
        ("struct Item<T> { inner: T } mod api { type i32 = crate::b::Doc;
            type Alias = crate::Item<i32>; impl Alias { pub fn touch(&self) {} } }
            fn f(p: Item<b::Doc>) { p.touch(); }", &["api.Alias.touch"], true),
        ("struct Item<T> { inner: T } mod api { type i32 = crate::b::Doc;
            type Alias = crate::Item<i32>; impl Alias { pub fn touch(&self) {} } }
            fn f(p: Item<i32>) { p.touch(); }", &[], false),
    ]
}

const LIFETIME_ALIAS: &str = "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>;
    impl<'a> Alias<'a> { pub fn touch(&self) {} } fn f<'a>(p: Alias<'a>) { p.touch(); }";

#[test]
fn alias_owner_named_lifetime_probe_requires_bound_regions() {
    check_locals(LIFETIME_ALIAS, &["Alias.touch"]);
}

#[test]
fn alias_owner_applications_preserve_exact_member_and_yield_ids_fresh_and_cold() {
    for (source, expected, _) in cases() {
        check_locals(source, expected);
    }
}

#[test]
fn alias_owner_terminal_nominal_must_belong_to_the_impl_crate() {
    check_project_mode(&[("Cargo.toml", "[workspace]\nmembers=['app','api']"),
        ("api/Cargo.toml", "[package]\nname='api'"), ("api/src/lib.rs", "pub struct RealDoc;"),
        ("app/Cargo.toml", "[package]\nname='app'\n[dependencies]\napi={path='../api'}"),
        ("app/src/lib.rs", "type Alias = api::RealDoc; impl Alias { pub fn new() -> Self { loop {} } pub fn touch(&self) {} }
            fn f() { api::RealDoc::new().touch(); }")], "app/src/lib.rs", None, true);
}

#[test]
#[ignore = "independent alias/application fixture legality; requires rustc on PATH, not compiler target labels"]
fn alias_owner_fixtures_agree_with_rustc() {
    for (source, _, allowed) in cases()
        .into_iter()
        .chain(lifetimes::cases())
        .chain(lifetimes::elided_cases())
        .chain([
            (REFERENCE_ALIAS, &["Alias.touch"][..], true),
            (LIFETIME_ALIAS, &["Alias.touch"][..], true),
            (lifetimes::ELIDED_ALIAS, &["Alias.touch"][..], true),
            (lifetimes::ELIDED_OUTPUT, &["make", "Alias.touch"][..], true),
        ])
    {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(&input, format!("{LOCAL_TYPES} {source}")).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "alias_fixture",
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
        assert_eq!(output.status.success(), allowed, "{source}\n{diagnostics}");
        if !allowed {
            assert!(
                ["E0599", "E0391"]
                    .iter()
                    .any(|code| diagnostics.contains(code)),
                "{diagnostics}"
            );
        }
    }
}
