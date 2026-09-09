use super::*;

fn cases() -> Vec<String> {
    let prelude = "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} } struct C;";
    let mut cases = Vec::new();
    for (receiver, arg_type) in [
        ("&self", "&C"),
        ("&mut self", "&mut C"),
        ("self: &Self", "&C"),
        ("self: &mut Self", "&mut C"),
        ("&'_ self", "&C"),
        ("self: &'_ Self", "&C"),
        ("&'a self", "&C"),
        ("self: &'a Self", "&C"),
        ("&'static self", "&'static C"),
        ("mut self: &Self", "&C"),
    ] {
        cases.push(format!("{prelude} impl C {{ fn make<'a>({receiver}, other: &b::Doc) -> Alias<'_> {{ loop {{}} }} }}
            fn f(p: {arg_type}, other: &b::Doc) {{ p.make(other).touch(); }}"));
    }
    cases.push(format!("{prelude} impl C {{ fn make(&self) -> Alias {{ loop {{}} }} }} fn f(p: &C) {{ p.make().touch(); }}"));
    cases.push(format!("{prelude} impl C {{ fn make(&self) -> Alias<'_> {{ loop {{}} }} }} fn f(p: &C) {{ let q = p.make(); q.touch(); }}"));
    cases.push(format!("{prelude} impl C {{ fn make(&self) -> Alias<'_> {{ loop {{}} }} }} fn f(p: &&C) {{ p.make().touch(); }}"));
    cases.push(format!("{prelude} type Borrow<'a> = &'a C; impl C {{ fn make(&self) -> Alias<'_> {{ loop {{}} }} }} fn f(p: Borrow<'_>) {{ p.make().touch(); }}"));
    cases.push("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>; impl<'a,T> Alias<'a,T> { fn touch(&self) {} }
        struct C<T> { inner:T } impl<T> C<T> { fn make(&self, other: &b::Doc) -> Alias<'_,T> { loop {} } }
        fn f(p: &C<b::Doc>, other: &b::Doc) { p.make(other).touch(); }".into());
    cases.push("struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
        struct C<'b> { inner: &'b b::Doc } impl<'b> C<'b> { fn make(&self) -> Alias<'_> { loop {} } }
        fn f(p: &C<'_>) { p.make().touch(); }".into());
    cases
}

#[test]
fn receiver_outputs_keep_exact_targets_across_zero_arguments_locals_and_projection_fresh_and_cold()
{
    for source in cases() {
        check_locals(&source, &["C.make", "Alias.touch"]);
    }
    check_locals("struct Item<T> { inner: T } type Alias<'a,T> = Item<&'a T>;
        struct C<T> { inner:T } impl<T> C<T> { fn make(&self, other: &b::Doc) -> Alias<'_,T> { loop {} } }
        fn f(p: &C<b::Doc>, other: &b::Doc) { p.make(other).inner.touch(); }", &["C.make", "b.Doc.touch"]);
}

const OWNED: &str = "struct Item<T> { inner: T } type Alias<'a> = Item<&'a b::Doc>; impl<'a> Alias<'a> { fn touch(&self) {} }
    struct C; impl C { fn make(&self, other: &b::Doc) -> Alias<'_> { loop {} } }
    fn f(p: C, other: &b::Doc) { p.make(other).touch(); }";

#[test]
fn owned_receiver_output_probe_requires_call_site_borrow_identity() {
    check_locals(OWNED, &["C.make", "Alias.touch"]);
}

#[test]
#[ignore = "independent receiver-output legality checks; requires rustc"]
fn receiver_output_fixtures_agree_with_rustc() {
    for source in cases().into_iter().chain([OWNED.to_owned()]) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(&input, format!("{LOCAL_TYPES} {source}")).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "receiver_fixture",
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
        assert!(
            output.status.success(),
            "{source}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
