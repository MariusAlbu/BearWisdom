use super::*;
use crate::indexer::resolve::engine::testkit::{
    call_ref, file_ctx, import, ref_ctx, source_symbol, sym, Lookup,
};
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain};

fn seg(name: &str, is_call: bool, kind: SegmentKind) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

/// A root segment carrying a split declared annotation: head + type args, the
/// form the extractor emits for `repo: Repository<User>`.
fn seg_declared(name: &str, declared: &str, type_args: &[&str]) -> ChainSegment {
    let mut s = seg(name, false, SegmentKind::Identifier);
    s.declared_type = Some(declared.to_string());
    s.type_args = type_args.iter().map(|t| t.to_string()).collect();
    s
}

fn resolve(lookup: &Lookup, segs: Vec<ChainSegment>, src_qname: &str) -> Option<i64> {
    resolve_with_fc(lookup, segs, src_qname, &file_ctx(vec![], None))
}

/// Resolve under a caller-supplied `FileContext`, so a test can carry imports
/// that steer the import-scoped declaration pick.
fn resolve_with_fc(
    lookup: &Lookup,
    segs: Vec<ChainSegment>,
    src_qname: &str,
    fc: &FileContext,
) -> Option<i64> {
    let leaf = segs.last().unwrap().name.clone();
    let mut r = call_ref(&leaf);
    r.chain = Some(MemberChain { segments: segs });
    let mut s = source_symbol("caller");
    s.qualified_name = src_qname.to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(&rc, fc, lookup).map(|res| res.target_symbol_id)
}

#[test]
fn binds_member_on_local_variable_type() {
    let lookup = Lookup::new()
        .with_local_type("repo", "Repo")
        .with_member("Repo", sym(10, "find", "Repo.find", "method", "a.ts"));
    let segs = vec![
        seg("repo", false, SegmentKind::Identifier),
        seg("find", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(10));
}

#[test]
fn walks_two_hops_advancing_through_return_type() {
    let lookup = Lookup::new()
        .with_local_type("repo", "Repo")
        .with_member("Repo", sym(10, "find", "Repo.find", "method", "a.ts"))
        .with_return_type("Repo.find", "User")
        .with_member("User", sym(20, "name", "User.name", "field", "a.ts"));
    let segs = vec![
        seg("repo", false, SegmentKind::Identifier),
        seg("find", true, SegmentKind::Property),
        seg("name", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

#[test]
fn probe_typeof_callable_property_yields_referenced_fn_return() {
    // vi.fn().mockImplementation(): VitestUtils.fn is typed `typeof fn` where
    // fn is a function returning Mock. Calling vi.fn() must yield Mock so the
    // next hop (.mockImplementation) resolves on Mock.
    let lookup = Lookup::new()
        .with_local_type("vi", "VitestUtils")
        .with(sym(1, "fn", "fn", "function", "a.ts"))
        .with_return_type("fn", "Mock")
        .with_member("VitestUtils", sym(2, "fn", "VitestUtils.fn", "property", "a.ts"))
        .with_field_type("VitestUtils.fn", "fn")
        .with(sym(5, "Mock", "Mock", "interface", "a.ts"))
        .with_member(
            "Mock",
            sym(3, "mockImplementation", "Mock.mockImplementation", "method", "a.ts"),
        );
    let segs = vec![
        seg("vi", false, SegmentKind::Identifier),
        seg("fn", true, SegmentKind::Property),
        seg("mockImplementation", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(3));
}

#[test]
fn probe_vitest_fn_mock_chain_end_to_end() {
    // vi.fn().mockImplementation(): vi: VitestUtils; VitestUtils.fn typed as the
    // spy.fn function (typeof fn); spy.fn returns Mock; Mock is a transparent
    // alias to MockInstance; MockInstance.mockImplementation is the leaf.
    let lookup = Lookup::new()
        .with_local_type("vi", "VitestUtils")
        .with_member("VitestUtils", sym(1, "fn", "VitestUtils.fn", "property", "a.ts"))
        .with_field_type("VitestUtils.fn", "spy.fn")
        .with(sym(2, "fn", "spy.fn", "function", "a.ts"))
        .with_return_type("spy.fn", "Mock")
        .with(sym(3, "Mock", "Mock", "type_alias", "a.ts"))
        .with_field_type("Mock", "MockInstance")
        .with(sym(4, "MockInstance", "MockInstance", "interface", "a.ts"))
        .with_member(
            "MockInstance",
            sym(5, "mockImplementation", "MockInstance.mockImplementation", "method", "a.ts"),
        );
    let segs = vec![
        seg("vi", false, SegmentKind::Identifier),
        seg("fn", true, SegmentKind::Property),
        seg("mockImplementation", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(5));
}

#[test]
fn probe_intersection_alias_with_own_members_finds_branch_member() {
    // screen.getByText: screen: Screen, Screen = BoundFunctions & { debug },
    // getByText lives on the BoundFunctions branch. Screen having its OWN
    // member (debug) must not block resolving getByText on the branch.
    let lookup = Lookup::new()
        .with_local_type("screen", "Screen")
        .with(sym(1, "Screen", "Screen", "type", "a.ts"))
        .with_alias(
            "Screen",
            crate::types::AliasTarget::Intersection(vec!["BoundFunctions".to_string()]),
        )
        .with_member("Screen", sym(2, "debug", "Screen.debug", "property", "a.ts"))
        .with(sym(3, "BoundFunctions", "BoundFunctions", "type", "a.ts"))
        .with_member(
            "BoundFunctions",
            sym(4, "getByText", "BoundFunctions.getByText", "method", "a.ts"),
        );
    let segs = vec![
        seg("screen", false, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(4));
}

#[test]
fn roots_at_bare_type_name_for_static_access() {
    let lookup = Lookup::new()
        .with(sym(1, "Math", "Math", "class", "a.ts"))
        .with_member("Math", sym(30, "max", "Math.max", "method", "a.ts"));
    let segs = vec![
        seg("Math", false, SegmentKind::TypeAccess),
        seg("max", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(30));
}

#[test]
fn roots_member_on_array_typed_receiver() {
    // `items: User[]` — the receiver's array type normalizes to the lib Array,
    // so `items.map(...)` resolves against Array's members.
    let lookup = Lookup::new()
        .with_local_type("items", "User[]")
        .with_member(
            "Array",
            sym(60, "map", "Array.map", "method", "ext:ts:__ts_lib__/lib.es5.d.ts"),
        );
    let segs = vec![
        seg("items", false, SegmentKind::Identifier),
        seg("map", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(60));
}

#[test]
fn roots_member_on_array_annotated_local() {
    // `const queries: Array<unknown> = []` then `queries.forEach(...)`.
    // The annotation seeds the local type as Array; forEach must resolve
    // against Array's members. Regression guard for the annotation-capture +
    // flow_binding_decl_type seeding path.
    let lookup = Lookup::new()
        .with_local_type("queries", "Array<unknown>")
        .with_member(
            "Array",
            sym(
                61,
                "forEach",
                "Array.forEach",
                "method",
                "ext:ts:__ts_lib__/lib.es5.d.ts",
            ),
        );
    let segs = vec![
        seg("queries", false, SegmentKind::Identifier),
        seg("forEach", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(61));
}

#[test]
fn binds_member_through_named_intersection_branch() {
    // `const r = render(...)` types `r` as `Result`, an intersection alias
    // `BoundFunctions<typeof queries> & { container; ... }`. `getByText` is NOT a
    // member of `Result`; it lives on the NAMED branch `BoundFunctions`. The walk
    // must follow the named branch to bind it. (solid-testing-library shape.)
    let lookup = Lookup::new()
        .with_local_type("r", "@solidjs/testing-library.Result")
        .with_alias(
            "@solidjs/testing-library.Result",
            AliasTarget::Intersection(vec!["BoundFunctions".to_string()]),
        )
        .with(sym(
            2,
            "BoundFunctions",
            "@testing-library/dom.BoundFunctions",
            "interface",
            "ext:ts:@testing-library/dom/get-queries-for-element.d.ts",
        ))
        .with_member_id(
            2,
            sym(
                90,
                "getByText",
                "@testing-library/dom.BoundFunctions.getByText",
                "method",
                "ext:ts:@testing-library/dom/get-queries-for-element.d.ts",
            ),
        );
    let segs = vec![
        seg("r", false, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(90));
}

#[test]
fn roots_on_imported_value_declared_type() {
    // `initTRPC.create()` — the root is an imported VALUE (a `declare const`)
    // whose declaration carries a type. Rooting on that type lets the member
    // walk continue, even though `initTRPC` is not itself a type name.
    let lookup = Lookup::new()
        .with(sym(
            1,
            "initTRPC",
            "@trpc/server.initTRPC",
            "variable",
            "ext:ts:@trpc/server/index.d.ts",
        ))
        .with_field_type("@trpc/server.initTRPC", "TRPCBuilder")
        .with_member(
            "TRPCBuilder",
            sym(50, "create", "TRPCBuilder.create", "method", "ext:ts:@trpc/server/index.d.ts"),
        );
    let segs = vec![
        seg("initTRPC", false, SegmentKind::Identifier),
        seg("create", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(50));
}

#[test]
fn roots_self_at_enclosing_type() {
    let lookup = Lookup::new()
        .with_enclosing("Svc.run", "Svc")
        .with_member("Svc", sym(40, "helper", "Svc.helper", "method", "a.ts"));
    let segs = vec![
        seg("this", false, SegmentKind::SelfRef),
        seg("helper", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "Svc.run"), Some(40));
}

#[test]
fn declines_when_member_absent() {
    let lookup = Lookup::new()
        .with_local_type("repo", "Repo")
        .with_member("Repo", sym(10, "find", "Repo.find", "method", "a.ts"));
    let segs = vec![
        seg("repo", false, SegmentKind::Identifier),
        seg("missing", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), None);
}

#[test]
fn declines_when_root_untyped() {
    let lookup = Lookup::new().with_member("Repo", sym(10, "find", "Repo.find", "method", "a.ts"));
    let segs = vec![
        seg("mystery", false, SegmentKind::Identifier),
        seg("find", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), None);
}

// --- lookup_member / member_yield_type (the chain-walk primitives) ----------

fn any(_: &str) -> bool {
    true
}

#[test]
fn lookup_member_finds_direct_member() {
    let lookup = Lookup::new().with_member("Repo", sym(5, "find", "Repo.find", "method", "a.ts"));
    let m = lookup_member(&lookup, "Repo", "find", &any).expect("member found");
    assert_eq!(m.id, 5);
    assert_eq!(m.qualified_name, "Repo.find");
}

#[test]
fn lookup_member_climbs_supertype() {
    let lookup = Lookup::new()
        .with_member("Base", sym(7, "save", "Base.save", "method", "a.ts"))
        .with_parent("Repo", "Base");
    let m = lookup_member(&lookup, "Repo", "save", &any).expect("inherited member found");
    assert_eq!(m.id, 7);
}

#[test]
fn lookup_member_declines_unknown() {
    let lookup = Lookup::new().with_member("Repo", sym(5, "find", "Repo.find", "method", "a.ts"));
    assert!(lookup_member(&lookup, "Repo", "missing", &any).is_none());
}

#[test]
fn lookup_member_kind_predicate_filters_candidates() {
    let lookup = Lookup::new().with_member("Repo", sym(5, "find", "Repo.find", "field", "a.ts"));
    // Accept only methods → the field `find` is rejected.
    assert!(lookup_member(&lookup, "Repo", "find", &|k| k == "method").is_none());
}

#[test]
fn member_yield_type_reads_return_for_calls_and_field_otherwise() {
    let lookup = Lookup::new()
        .with_return_type("Repo.find", "User")
        .with_field_type("Repo.db", "Database");
    let arena = lookup.type_arena().unwrap();
    assert_eq!(
        member_yield_type(&lookup, arena, "Repo.find", true).map(|id| arena.format_type(id)),
        Some("User".to_string())
    );
    assert_eq!(
        member_yield_type(&lookup, arena, "Repo.db", false).map(|id| arena.format_type(id)),
        Some("Database".to_string())
    );
    assert_eq!(member_yield_type(&lookup, arena, "Repo.unknown", true), None);
}

#[test]
fn binds_generic_method_return_substituting_type_arg() {
    // interface Repository<T> { find(): T }
    // const repo: Repository<User> = ...; repo.find().name  →  User.name (id 20)
    let lookup = Lookup::new()
        .with_local_type("repo", "Repository<User>")
        .with_generics("Repository", &["T"])
        .with_member("Repository", sym(10, "find", "Repository.find", "method", "a.ts"))
        .with_return_type("Repository.find", "T")
        .with_member("User", sym(20, "name", "User.name", "field", "a.ts"));
    let segs = vec![
        seg("repo", false, SegmentKind::Identifier),
        seg("find", true, SegmentKind::Property),
        seg("name", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

#[test]
fn binds_through_a_type_alias() {
    // type UserRepo = Repository<User>;  interface Repository<T> { find(): T }
    // const r: UserRepo = ...; r.find().name  →  User.name (id 20)
    let lookup = Lookup::new()
        .with_local_type("r", "UserRepo")
        .with_alias(
            "UserRepo",
            crate::types::AliasTarget::Application {
                root: "Repository".to_string(),
                args: vec!["User".to_string()],
            },
        )
        .with_generics("Repository", &["T"])
        .with_member("Repository", sym(10, "find", "Repository.find", "method", "a.ts"))
        .with_return_type("Repository.find", "T")
        .with_member("User", sym(20, "name", "User.name", "field", "a.ts"));
    let segs = vec![
        seg("r", false, SegmentKind::Identifier),
        seg("find", true, SegmentKind::Property),
        seg("name", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

#[test]
fn binds_member_through_a_mapped_type_to_its_source() {
    // type Override<A> = { [K in keyof A]: ... };  const m: Override<Result>;
    // m.mutate  →  Result.mutate (id 30). A mapped type's keys ARE its source's
    // keys, so the member resolves on the bound source object.
    let lookup = Lookup::new()
        .with_local_type("m", "Override<Result>")
        .with_alias(
            "Override",
            crate::types::AliasTarget::Mapped {
                source: "A".to_string(),
                value_template: "A[K]".to_string(),
            },
        )
        .with_generics("Override", &["A"])
        .with_member("Result", sym(30, "mutate", "Result.mutate", "property", "a.ts"));
    let segs = vec![
        seg("m", false, SegmentKind::Identifier),
        seg("mutate", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(30));
}

#[test]
fn member_on_generic_supertype_binds_args_from_extends_edge() {
    // class Child extends Base<User>;  interface Base<T> { m: T }   const c: Child;
    // c.m yields T, which the `extends Base<User>` edge binds to User, so c.m.firstName
    // resolves to User.firstName. The args live on the supertype edge, not on the
    // receiver (Child has none), so substitute_through alone leaves m's yield as
    // unbound T and .firstName misses — the supertype-edge bind is what types it.
    let lookup = Lookup::new()
        .with_local_type("c", "Child")
        .with(sym(1, "Child", "Child", "class", "a.ts"))
        .with(sym(2, "Base", "Base", "interface", "a.ts"))
        .with_parent_id(1, 2)
        .with_parent_args("Child", "Base", &["User"])
        .with_generics("Base", &["T"])
        .with_member_id(2, sym(30, "m", "Base.m", "property", "a.ts"))
        .with_field_type("Base.m", "T")
        .with(sym(3, "User", "User", "class", "a.ts"))
        .with_member("User", sym(40, "firstName", "User.firstName", "property", "a.ts"));
    let segs = vec![
        seg("c", false, SegmentKind::Identifier),
        seg("m", false, SegmentKind::Property),
        seg("firstName", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(40));
}

#[test]
fn binds_member_through_unbound_mapped_source_via_reexport_closure() {
    // testing-library `RenderResult`:
    //   type RenderResult<Q extends Queries = typeof queries> =
    //     { ... } & { [P in keyof Q]: BoundFunction<Q[P]> };
    //   const rendered: RenderResult;  rendered.getByText(...)
    // `Q` is an UNBOUND parameter (the receiver supplies no type argument), so it
    // defaults to `typeof queries` — a value namespace whose keys (`getByText`)
    // live in `@testing-library/dom`, which `@testing-library/react` re-exports
    // wholesale. The member resolves through that wildcard re-export closure.
    let barrel = "ext:ts:@testing-library/react/types/index.d.ts";
    let lookup = Lookup::new()
        .with_local_type("rendered", "RenderResult")
        .with(sym(1, "RenderResult", "RenderResult", "type_alias", barrel))
        .with_alias(
            "RenderResult",
            crate::types::AliasTarget::Mapped {
                source: "Q".to_string(),
                value_template: "BoundFunction<Q[P]>".to_string(),
            },
        )
        .with_generics("RenderResult", &["Q"])
        .with_reexport(barrel, "*", "@testing-library/dom")
        .with(sym(
            42,
            "getByText",
            "@testing-library/dom.getByText",
            "function",
            "ext:ts:@testing-library/dom/types/queries.d.ts",
        ));
    let segs = vec![
        seg("rendered", false, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(42));
}

#[test]
fn roots_binding_typed_return_type_of_typeof_fn() {
    // const rendered: ReturnType<typeof render>;  rendered.getByText()
    // `ReturnType<T> = T extends (...) => infer R ? R : any` — a return-type
    // extraction. Applied to `typeof render`, it roots `rendered` on `render`'s
    // return type (RenderResult), so the member resolves there.
    let lookup = Lookup::new()
        .with_local_type("rendered", "ReturnType<typeof render>")
        .with_alias(
            "ReturnType",
            crate::types::AliasTarget::Conditional {
                check: "T".to_string(),
                extends: "(...args: any) => infer R".to_string(),
                true_branch: "R".to_string(),
                false_branch: "any".to_string(),
                infer_binding: None,
            },
        )
        .with(sym(1, "render", "render", "function", "ext:ts:tl.d.ts"))
        .with_return_type("render", "RenderResult")
        .with_member(
            "RenderResult",
            sym(42, "getByText", "RenderResult.getByText", "method", "ext:ts:tl.d.ts"),
        );
    let segs = vec![
        seg("rendered", false, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(42));
}

#[test]
fn return_type_of_typeof_fn_scopes_to_the_imported_overload() {
    // Two same-named `render` in different packages. `typeof render` must bind to
    // the one the file IMPORTS (react), not a first-winner (vue) — so
    // `rendered: ReturnType<typeof render>` roots on react's RenderResult.
    let lookup = Lookup::new()
        .with_local_type("rendered", "ReturnType<typeof render>")
        .with_alias(
            "ReturnType",
            crate::types::AliasTarget::Conditional {
                check: "T".to_string(),
                extends: "(...args: any) => infer R".to_string(),
                true_branch: "R".to_string(),
                false_branch: "any".to_string(),
                infer_binding: None,
            },
        )
        .with(sym(1, "render", "@testing-library/vue.render", "function", "ext:ts:vue.d.ts"))
        .with_return_type("@testing-library/vue.render", "Vue")
        .with(sym(2, "render", "@testing-library/react.render", "function", "ext:ts:react.d.ts"))
        .with_return_type("@testing-library/react.render", "RenderResult")
        .with_member(
            "RenderResult",
            sym(42, "getByText", "RenderResult.getByText", "method", "ext:ts:react.d.ts"),
        );
    let fc = file_ctx(vec![import("render", Some("@testing-library/react"))], None);
    let segs = vec![
        seg("rendered", false, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve_with_fc(&lookup, segs, "caller", &fc), Some(42));
}

#[test]
fn unbound_mapped_source_declines_when_receiver_is_internal() {
    // The same unbound-mapped shape but the receiver type is declared in an
    // INTERNAL file: the namespace + wholesale-re-export shape only occurs in
    // library `.d.ts`, so the re-export-closure fallback must NOT fire for
    // project code (guards against false resolves on internal utility mapped
    // types used without a type argument).
    let internal = "src/types.ts";
    let lookup = Lookup::new()
        .with_local_type("x", "Local")
        .with(sym(1, "Local", "Local", "type_alias", internal))
        .with_alias(
            "Local",
            crate::types::AliasTarget::Mapped {
                source: "Q".to_string(),
                value_template: "Q[P]".to_string(),
            },
        )
        .with_generics("Local", &["Q"])
        .with_reexport(internal, "*", "other")
        .with(sym(42, "getByText", "other.getByText", "function", internal));
    let segs = vec![
        seg("x", false, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), None);
}

#[test]
fn roots_a_call_at_the_callee_return_type() {
    // function makeRepo(): Repo {...};  makeRepo().save()  →  Repo.save (id 30)
    let lookup = Lookup::new()
        .with(sym(1, "makeRepo", "makeRepo", "function", "a.ts"))
        .with_return_type("makeRepo", "Repo")
        .with_member("Repo", sym(30, "save", "Repo.save", "method", "a.ts"));
    let segs = vec![
        seg("makeRepo", true, SegmentKind::Identifier),
        seg("save", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(30));
}

#[test]
fn roots_a_call_at_a_callable_interface_values_call_signature_return() {
    // const expect: ExpectStatic;  interface ExpectStatic { (x): Assertion }
    // expect(x).toBe(...)  →  Assertion.toBe (id 70)
    // The callee `expect` is a const, not a function, so its declared interface's
    // synthesised `call` member supplies the call result type (Assertion).
    let lookup = Lookup::new()
        .with(sym(1, "expect", "expect", "const", "ext:ts:vitest/index.d.ts"))
        .with_field_type("expect", "ExpectStatic")
        .with_member(
            "ExpectStatic",
            sym(50, "call", "ExpectStatic.call", "method", "ext:ts:vitest/index.d.ts"),
        )
        .with_return_type("ExpectStatic.call", "Assertion")
        .with_member(
            "Assertion",
            sym(70, "toBe", "Assertion.toBe", "method", "ext:ts:vitest/index.d.ts"),
        );
    let segs = vec![
        seg("expect", true, SegmentKind::Identifier),
        seg("toBe", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(70));
}

#[test]
fn call_root_through_callable_interface_substitutes_receiver_type_arg() {
    // const wrap: Wrapper<User>;  interface Wrapper<T> { (): T }
    // wrap().name  →  User.name (id 20): the `call` member returns T, which the
    // receiver's applied arg binds to User exactly as any generic member hop does.
    let lookup = Lookup::new()
        .with(sym(1, "wrap", "wrap", "const", "a.ts"))
        .with_field_type("wrap", "Wrapper<User>")
        .with_generics("Wrapper", &["T"])
        .with_member("Wrapper", sym(50, "call", "Wrapper.call", "method", "a.ts"))
        .with_return_type("Wrapper.call", "T")
        .with_member("User", sym(20, "name", "User.name", "field", "a.ts"));
    let segs = vec![
        seg("wrap", true, SegmentKind::Identifier),
        seg("name", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

#[test]
fn function_callee_return_path_unaffected_by_callable_value_fallback() {
    // function render(...): RenderResult {...};  render(c).getByText(...)
    //   →  RenderResult.getByText (id 90)
    // render is a real function whose return type roots via callee_return_type
    // (kind=function); the callable-value fallback must not perturb it.
    let lookup = Lookup::new()
        .with(sym(1, "render", "render", "function", "ext:ts:@testing-library/react/index.d.ts"))
        .with_return_type("render", "RenderResult")
        .with_member(
            "RenderResult",
            sym(90, "getByText", "RenderResult.getByText", "method", "ext:ts:@testing-library/react/index.d.ts"),
        );
    let segs = vec![
        seg("render", true, SegmentKind::Identifier),
        seg("getByText", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(90));
}

#[test]
fn callee_return_root_prefers_import_scoped_declaration_for_overloaded_name() {
    // Two `render` callables: one in a UI-component package (pkg 7) returning a
    // node type with no `getByText`, one in the DOM-testing package (pkg 8)
    // returning RenderResult. The use site imports `render` from pkg 8's
    // specifier, so `render(c).getByText(...)` must root on pkg 8's declaration
    // (-> RenderResult -> getByText id 90), NOT the first-callable-wins pkg-7
    // declaration that has no usable member. Generic overload disambiguation by
    // import scope; no `render` special-case.
    let lookup = Lookup::new()
        .with_workspace_pkg("ui", 7)
        .with_workspace_pkg("dom-testing", 8)
        .with_in_package(
            7,
            sym(1, "render", "ui.render", "function", "packages/ui/render.ts"),
        )
        .with_return_type("ui.render", "ReactNode")
        .with_in_package(
            8,
            sym(2, "render", "domTesting.render", "function", "packages/dom-testing/render.ts"),
        )
        .with_return_type("domTesting.render", "RenderResult")
        .with_member(
            "RenderResult",
            sym(90, "getByText", "RenderResult.getByText", "method", "packages/dom-testing/render.ts"),
        );
    let segs = || {
        vec![
            seg("render", true, SegmentKind::Identifier),
            seg("getByText", true, SegmentKind::Property),
        ]
    };
    // Import `render` from pkg 8 -> root on domTesting.render -> RenderResult.
    let fc_dom = file_ctx(vec![import("render", Some("dom-testing"))], None);
    assert_eq!(resolve_with_fc(&lookup, segs(), "caller", &fc_dom), Some(90));
    // No import attribution: first-callable (pkg 7, ReactNode) wins and has no
    // getByText -> unresolved. Proves the scope filter, not a global reorder.
    assert_eq!(resolve(&lookup, segs(), "caller"), None);
}

#[test]
fn union_member_forwards_when_every_arm_carries_it() {
    // `type QOR = ArmA | ArmB`, both `extends Base`; Base declares `data`.
    //   useQ(): QOR ; useQ().data  → Base.data (id 99)
    // A tagged result union (e.g. TanStack's QueryObserverResult) resolves a
    // shared-base member because every arm carries it via inheritance.
    let lookup = Lookup::new()
        .with(sym(1, "useQ", "useQ", "function", "pkg/q.ts"))
        .with_return_type("useQ", "QOR")
        .with(sym(2, "QOR", "QOR", "type_alias", "pkg/q.ts"))
        .with_alias(
            "QOR",
            crate::types::AliasTarget::Union(vec!["ArmA".into(), "ArmB".into()]),
        )
        .with(sym(3, "ArmA", "ArmA", "interface", "pkg/q.ts"))
        .with_parent("ArmA", "Base")
        .with(sym(4, "ArmB", "ArmB", "interface", "pkg/q.ts"))
        .with_parent("ArmB", "Base")
        .with(sym(5, "Base", "Base", "interface", "pkg/q.ts"))
        .with_member("Base", sym(99, "data", "Base.data", "property", "pkg/q.ts"));
    let segs = vec![
        seg("useQ", true, SegmentKind::Identifier),
        seg("data", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(99));
}

#[test]
fn union_member_declines_when_an_arm_lacks_it() {
    // Only ArmA carries `data`; union member access requires it on EVERY arm,
    // so `useQ().data` does not resolve.
    let lookup = Lookup::new()
        .with(sym(1, "useQ", "useQ", "function", "pkg/q.ts"))
        .with_return_type("useQ", "QOR")
        .with(sym(2, "QOR", "QOR", "type_alias", "pkg/q.ts"))
        .with_alias(
            "QOR",
            crate::types::AliasTarget::Union(vec!["ArmA".into(), "ArmB".into()]),
        )
        .with(sym(3, "ArmA", "ArmA", "interface", "pkg/q.ts"))
        .with_member("ArmA", sym(99, "data", "ArmA.data", "property", "pkg/q.ts"))
        .with(sym(4, "ArmB", "ArmB", "interface", "pkg/q.ts"));
    let segs = vec![
        seg("useQ", true, SegmentKind::Identifier),
        seg("data", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), None);
}

#[test]
fn function_and_callable_value_roots_cascade_to_further_members() {
    // The same generic root-typing mechanism that binds `toBe` / `getByText`
    // binds any further member on the rooted interface without per-member code:
    //   expect(x).toEqualTypeOf(y)  → ExpectStatic call sig -> Assertion.toEqualTypeOf
    //   render(c).getByRole(r)      → render(): RenderResult -> RenderResult.getByRole
    let lookup = Lookup::new()
        .with(sym(1, "expect", "expect", "const", "ext:ts:vitest/index.d.ts"))
        .with_field_type("expect", "ExpectStatic")
        .with_member(
            "ExpectStatic",
            sym(50, "call", "ExpectStatic.call", "method", "ext:ts:vitest/index.d.ts"),
        )
        .with_return_type("ExpectStatic.call", "Assertion")
        .with_member(
            "Assertion",
            sym(71, "toEqualTypeOf", "Assertion.toEqualTypeOf", "method", "ext:ts:vitest/index.d.ts"),
        )
        .with(sym(2, "render", "render", "function", "ext:ts:@testing-library/react/index.d.ts"))
        .with_return_type("render", "RenderResult")
        .with_member(
            "RenderResult",
            sym(91, "getByRole", "RenderResult.getByRole", "method", "ext:ts:@testing-library/react/index.d.ts"),
        );
    let expect_segs = vec![
        seg("expect", true, SegmentKind::Identifier),
        seg("toEqualTypeOf", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, expect_segs, "caller"), Some(71));
    let render_segs = vec![
        seg("render", true, SegmentKind::Identifier),
        seg("getByRole", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, render_segs, "caller"), Some(91));
}

#[test]
fn import_typed_value_resolves_through_to_referenced_export_type() {
    // The extractor lowers `const expect: typeof import('vitest')['expect']` to a
    // field_type whose head is the bare export name `expect` (the cross-module
    // ref target). The exporting module's own `expect` value carries the real
    // type `ExpectStatic`. The chain root `expect(x).toBe(...)` must chase the
    // import-typed shim through to `ExpectStatic`'s call-signature return,
    // `Assertion`, and bind `Assertion.toBe`.
    let lookup = Lookup::new()
        // The use-site shim: its declared type is the bare export name.
        .with(sym(1, "expect", "expect", "const", "ext:ts:globals.d.ts"))
        .with_field_type("expect", "expect")
        // The exporting module's `expect` value, declared `: ExpectStatic`.
        .with(sym(2, "expect", "vitest.expect", "const", "ext:ts:vitest/index.d.ts"))
        .with_field_type("vitest.expect", "ExpectStatic")
        .with_member(
            "ExpectStatic",
            sym(50, "call", "ExpectStatic.call", "method", "ext:ts:vitest/index.d.ts"),
        )
        .with_return_type("ExpectStatic.call", "Assertion")
        .with_member(
            "Assertion",
            sym(70, "toBe", "Assertion.toBe", "method", "ext:ts:vitest/index.d.ts"),
        );
    let segs = vec![
        seg("expect", true, SegmentKind::Identifier),
        seg("toBe", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(70));
}

#[test]
fn re_roots_member_less_value_shim_onto_same_simple_name_type() {
    // The callee value's declared type is a QUALIFIED head (`vitest.ExpectStatic`)
    // whose own declaration is a member-less value re-export shim. The interface
    // that actually declares the call signature has the SAME simple name under a
    // DIFFERENT qname (`@vitest/expect.ExpectStatic`). Rooting `expect(x)` must
    // bridge the shim to that interface, yield `Assertion` from its `call` member,
    // and bind `Assertion.toBe`. Without the bridge the walk dead-ends on the
    // member-less value and the chain is unresolved.
    let lookup = Lookup::new()
        .with(sym(1, "expect", "vitest.expect", "const", "ext:ts:vitest/index.d.ts"))
        .with_field_type("vitest.expect", "vitest.ExpectStatic")
        // The shim: a member-less value of the qualified type-name.
        .with(sym(
            2,
            "ExpectStatic",
            "vitest.ExpectStatic",
            "variable",
            "ext:ts:vitest/index.d.ts",
        ))
        // The interface that declares the call signature, same simple name.
        .with(sym(
            3,
            "ExpectStatic",
            "@vitest/expect.ExpectStatic",
            "interface",
            "ext:ts:@vitest/expect/index.d.ts",
        ))
        .with_member(
            "@vitest/expect.ExpectStatic",
            sym(50, "call", "@vitest/expect.ExpectStatic.call", "method", "ext:ts:@vitest/expect/index.d.ts"),
        )
        .with_return_type("@vitest/expect.ExpectStatic.call", "Assertion")
        .with_member(
            "Assertion",
            sym(70, "toBe", "Assertion.toBe", "method", "ext:ts:@vitest/expect/index.d.ts"),
        );
    let segs = vec![
        seg("expect", true, SegmentKind::Identifier),
        seg("toBe", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(70));
}

#[test]
fn member_bearing_type_head_is_not_re_rooted() {
    // A value whose declared type head already names a members-bearing type must
    // root on THAT type unchanged — the shim-bridge must not divert it to a
    // same-simple-name decl elsewhere. Here `box: Holder` and `Holder` is a real
    // interface with `get`; no second `Holder` exists, so the receiver stays
    // `Holder` and binds `Holder.get`.
    let lookup = Lookup::new()
        .with(sym(1, "box", "box", "const", "a.ts"))
        .with_field_type("box", "Holder")
        .with(sym(2, "Holder", "Holder", "interface", "a.ts"))
        .with_member("Holder", sym(40, "get", "Holder.get", "method", "a.ts"));
    let segs = vec![
        seg("box", false, SegmentKind::Identifier),
        seg("get", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(40));
}

#[test]
fn roots_a_call_at_a_generic_return_type() {
    // function makeRepo(): Repository<User> {...};  makeRepo().find().name  →  User.name (id 20)
    let lookup = Lookup::new()
        .with(sym(1, "makeRepo", "makeRepo", "function", "a.ts"))
        .with_return_type("makeRepo", "Repository<User>")
        .with_generics("Repository", &["T"])
        .with_member("Repository", sym(10, "find", "Repository.find", "method", "a.ts"))
        .with_return_type("Repository.find", "T")
        .with_member("User", sym(20, "name", "User.name", "field", "a.ts"));
    let segs = vec![
        seg("makeRepo", true, SegmentKind::Identifier),
        seg("find", true, SegmentKind::Property),
        seg("name", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

#[test]
fn roots_declared_annotation_with_split_type_args() {
    // const repo: Repository<User> — the annotation arrives split: head + type args
    let lookup = Lookup::new()
        .with_generics("Repository", &["T"])
        .with_member("Repository", sym(10, "find", "Repository.find", "method", "a.ts"))
        .with_return_type("Repository.find", "T")
        .with_member("User", sym(20, "name", "User.name", "field", "a.ts"));
    let segs = vec![
        seg_declared("repo", "Repository", &["User"]),
        seg("find", true, SegmentKind::Property),
        seg("name", false, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

#[test]
fn root_binds_in_scope_var_over_same_named_value_elsewhere() {
    // Two `x` locals in different scopes: HostA.x: FooA (has `m`), HostB.x: FooB
    // (no `m`). The root must bind to the use site's in-scope declaration, not
    // the first same-named value in the by-name index. Mirrors the `devtools` /
    // `client` / `result` collisions across a monorepo.
    let lookup = Lookup::new()
        .with(sym(1, "x", "HostA.x", "variable", "a.ts"))
        .with(sym(2, "x", "HostB.x", "variable", "a.ts"))
        .with_field_type("HostA.x", "FooA")
        .with_field_type("HostB.x", "FooB")
        .with_member("FooA", sym(10, "m", "FooA.m", "method", "a.ts"));
    let segs = || {
        vec![
            seg("x", false, SegmentKind::Identifier),
            seg("m", true, SegmentKind::Property),
        ]
    };
    // From HostA: x is HostA.x -> FooA -> FooA.m.
    assert_eq!(resolve(&lookup, segs(), "HostA"), Some(10));
    // From HostB: x is HostB.x -> FooB, which has no `m` -> unresolved (NOT FooA.m).
    assert_eq!(resolve(&lookup, segs(), "HostB"), None);
}

// --- id-keyed member walk / inheritance climb (cross-package identity) -------

/// Member lookup follows the receiver's SYMBOL ID, not its qname string. Two
/// `Client` types in different packages share the qname `Client` but have
/// distinct ids and distinct `query` methods. A chain rooted on package A's
/// `Client` binds A's `query` (id 110); package B's binds B's (id 210) — even
/// though `by_qname("Client")` first-wins to one of them. This is the
/// member-walk half of the `devtools` cross-package collision.
#[test]
fn member_walk_keys_on_receiver_symbol_id_across_same_qname_types() {
    // Package A's Client (id 100) with query (id 110); package B's Client
    // (id 200) with query (id 210). Only A is registered under the qname index
    // (first-wins), so a qname-string member lookup would always pick A.
    let a_query = sym(110, "query", "Client.query", "method", "a.ts");
    let b_query = sym(210, "query", "Client.query", "method", "b.ts");
    let lookup = Lookup::new()
        .with(sym(100, "ClientA", "ClientA", "class", "a.ts"))
        .with(sym(200, "ClientB", "ClientB", "class", "b.ts"))
        .with_member_id(100, a_query)
        .with_member_id(200, b_query);

    // Root on package A's class by name -> id 100 -> query id 110.
    let segs_a = vec![
        seg("ClientA", false, SegmentKind::TypeAccess),
        seg("query", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs_a, "caller"), Some(110));

    // Root on package B's class by name -> id 200 -> query id 210, NOT 110.
    let segs_b = vec![
        seg("ClientB", false, SegmentKind::TypeAccess),
        seg("query", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs_b, "caller"), Some(210));
}

/// The supertype climb follows `parent_class_id`, not a parent qname string.
/// Package A's `Repo` (id 100) extends `Base` id 7 (which has `save`); an
/// unrelated `Base` in package B (id 8) has no `save`. `aRepo.save()` climbs to
/// id 7 and binds its `save` — never B's same-named base.
#[test]
fn inheritance_climb_keys_on_parent_symbol_id() {
    let lookup = Lookup::new()
        .with(sym(100, "Repo", "Repo", "class", "a.ts"))
        .with_parent_id(100, 7)
        .with_member_id(7, sym(70, "save", "Base.save", "method", "a.ts"));
    let segs = vec![
        seg("Repo", false, SegmentKind::TypeAccess),
        seg("save", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(70));
}

// --- structural interning of a generic inferred-local root ------------------

/// A generic inferred local — `const q = cache.build()` whose forward-inferred
/// type is `Query<string>` — interns structurally at the root, so member lookup
/// keys on the bare head `Query`, not on a flat class literally named
/// `Query<string>` (which would have no members). `resolve_root` runs the local
/// type through `arena.intern_type_str`, producing `Apply { Query, [string] }`;
/// `head_qname` looks through the application to `Query`, where `isStaleByTime`
/// is found.
#[test]
fn identifier_root_decomposes_generic_local_type() {
    let lookup = Lookup::new()
        .with_local_type("q", "Query<string>")
        .with_member(
            "Query",
            sym(80, "isStaleByTime", "Query.isStaleByTime", "method", "a.ts"),
        );
    let segs = vec![
        seg("q", false, SegmentKind::Identifier),
        seg("isStaleByTime", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(80));
}

// --- cross-package import-scoped binding (bare type_ref / 1-seg instantiates) -

use crate::indexer::resolve::engine::contract::FileContext;
use crate::indexer::resolve::engine::semantic_model::SemanticModel;
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};

static WS_PROFILE: LanguageProfile = LanguageProfile {
    workspace_packages: true,
    ..DEFAULT_PROFILE
};

/// A lookup with the same `QueryClient` class declared in two sibling workspace
/// packages — query-core (pkg 10, id 13168) and solid-query (pkg 19, id 15723) —
/// with query-core registered as a declared workspace package. The first-wins
/// `by_name`/`types_by_name` order puts solid-query (id 15723) first.
fn two_query_clients() -> Lookup {
    Lookup::new()
        .with_workspace_pkg("@tanstack/query-core", 10)
        .with_workspace_pkg("@tanstack/solid-query", 19)
        .with_in_package(
            19,
            sym(15723, "QueryClient", "QueryClient", "class", "packages/solid-query/src/QueryClient.ts"),
        )
        .with_in_package(
            10,
            sym(13168, "QueryClient", "QueryClient", "class", "packages/query-core/src/queryClient.ts"),
        )
}

/// The file imports `QueryClient` from `@tanstack/query-core`.
fn imports_query_client_from_core() -> FileContext {
    file_ctx(
        vec![import("QueryClient", Some("@tanstack/query-core"))],
        None,
    )
}

/// Drive one ref through the full engine (chain-less ladder + chain walk) under
/// the workspace-enabled profile, returning the bound symbol id.
fn engine_resolve(lookup: &Lookup, r: &ExtractedRef, fc: &FileContext) -> Option<i64> {
    let s = source_symbol("caller");
    let rc = ref_ctx(r, &s, vec![]);
    SemanticModel::production()
        .get_symbol_info(&rc, fc, lookup, &WS_PROFILE)
        .map(|res| res.target_symbol_id)
}

#[test]
fn bare_type_ref_binds_to_imported_package_not_by_name_first() {
    let lookup = two_query_clients();
    let fc = imports_query_client_from_core();
    let mut r = call_ref("QueryClient");
    r.kind = EdgeKind::TypeRef;
    // The use site imports QueryClient from query-core (pkg 10); the bind must
    // pick pkg-10's def (13168), NOT solid-query's first-wins def (15723).
    assert_eq!(engine_resolve(&lookup, &r, &fc), Some(13168));
}

#[test]
fn one_seg_instantiates_binds_to_imported_package_not_by_name_first() {
    let lookup = two_query_clients();
    let fc = imports_query_client_from_core();
    let mut r = call_ref("QueryClient");
    r.kind = EdgeKind::Instantiates;
    // `new QueryClient()` emits a single-segment chain with module=None; it falls
    // to the bare ladder and must bind the imported package's def.
    r.chain = Some(MemberChain {
        segments: vec![seg("QueryClient", false, SegmentKind::Identifier)],
    });
    assert_eq!(engine_resolve(&lookup, &r, &fc), Some(13168));
}

#[test]
fn multi_seg_chain_roots_on_imported_packages_class() {
    // `QueryClient.setQueryData(...)` static-access chain. Each package's
    // QueryClient has its OWN setQueryData (keyed by symbol id); the root must
    // pick the imported package's class so the member walk binds ITS method.
    let lookup = two_query_clients()
        .with_member_id(13168, sym(13200, "setQueryData", "QueryClient.setQueryData", "method", "packages/query-core/src/queryClient.ts"))
        .with_member_id(15723, sym(15800, "setQueryData", "QueryClient.setQueryData", "method", "packages/solid-query/src/QueryClient.ts"));
    let fc = imports_query_client_from_core();
    let segs = vec![
        seg("QueryClient", false, SegmentKind::TypeAccess),
        seg("setQueryData", true, SegmentKind::Property),
    ];
    let leaf = "setQueryData";
    let mut r = call_ref(leaf);
    r.chain = Some(MemberChain { segments: segs });
    let s = source_symbol("caller");
    let rc = ref_ctx(&r, &s, vec![]);
    // query-core (pkg 10) def is registered AFTER solid-query (pkg 19), so
    // first-wins would pick solid-query's method (15800); the import scope must
    // steer the root to pkg-10's class -> its setQueryData (13200).
    assert_eq!(
        bind_member_access(&rc, &fc, &lookup).map(|res| res.target_symbol_id),
        Some(13200)
    );
}

#[test]
fn member_walk_climbs_to_a_non_first_of_several_supertypes() {
    // `interface Assertion extends VitestAssertion, JestAssertion, Matchers` —
    // `toBe` is declared on JestAssertion, the SECOND supertype. The id-keyed
    // climb must visit EVERY direct parent (a DAG, breadth-first), not follow a
    // single linear chain that would keep only the first parent and miss toBe.
    let lookup = Lookup::new()
        .with(sym(100, "Assertion", "pkg.Assertion", "interface", "a.ts"))
        .with(sym(101, "VitestAssertion", "pkg.VitestAssertion", "interface", "a.ts"))
        .with(sym(102, "JestAssertion", "pkg.JestAssertion", "interface", "a.ts"))
        .with(sym(103, "Matchers", "pkg.Matchers", "interface", "a.ts"))
        .with_parent_id(100, 101)
        .with_parent_id(100, 102)
        .with_parent_id(100, 103)
        .with_member_id(102, sym(200, "toBe", "pkg.JestAssertion.toBe", "method", "a.ts"))
        .with_local_type("a", "pkg.Assertion");
    let segs = vec![
        seg("a", false, SegmentKind::Identifier),
        seg("toBe", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(200));
}

// --- use-site SymbolId threading across hops (determinism fix) ---------------

/// When a chain has two hops and the intermediate type's qname is shared by two
/// declarations in different packages, the walk must bind the final member on the
/// declaration that the USE SITE's import scope established at the root — not the
/// first-winner returned by `by_qualified_name`.
///
/// Setup:
///   - `Client` (qname "Client") in pkg 10 (id 100), `execute` (id 105, returns Client), `status` (id 110).
///   - `Client` (qname "Client") in pkg 20 (id 200), `execute` (id 205, returns Client), `status` (id 210).
///   - `by_qualified_name("Client")` first-winner = id 200 (registered last).
///   - The use-site imports `Client` from pkg 10 → root is established as id 100.
///
/// `Client.execute().status` must resolve to id 110 (pkg 10), NOT 210 (pkg 20's
/// first-winner that the re-search at the intermediate hop would pick without the id fix).
#[test]
fn intermediate_hop_threads_use_site_id_not_first_winner() {
    // Members for pkg A's Client (id 100).
    let mut exec_a = sym(105, "execute", "Client.execute", "method", "pkg_a/client.ts");
    exec_a.package_id = Some(10);
    let mut status_a = sym(110, "status", "Client.status", "field", "pkg_a/client.ts");
    status_a.package_id = Some(10);
    // Members for pkg B's Client (id 200).
    let mut exec_b = sym(205, "execute", "Client.execute", "method", "pkg_b/client.ts");
    exec_b.package_id = Some(20);
    let mut status_b = sym(210, "status", "Client.status", "field", "pkg_b/client.ts");
    status_b.package_id = Some(20);

    // Register pkg A's Client first (id 100), then pkg B's (id 200).
    // `by_qname["Client"]` overwrites to id 200 — first-winner for by_qualified_name.
    let lookup = Lookup::new()
        .with_workspace_pkg("pkg-a", 10)
        .with_workspace_pkg("pkg-b", 20)
        .with_in_package(10, sym(100, "Client", "Client", "class", "pkg_a/client.ts"))
        .with_in_package(20, sym(200, "Client", "Client", "class", "pkg_b/client.ts"))
        .with_member_id(100, exec_a)
        .with_member_id(100, status_a)
        .with_member_id(200, exec_b)
        .with_member_id(200, status_b)
        .with_return_type("Client.execute", "Client")
        .with_field_type("Client.status", "string");

    // The chain: Client.execute().status
    let segs = vec![
        seg("Client", false, SegmentKind::TypeAccess),
        seg("execute", true, SegmentKind::Property),
        seg("status", false, SegmentKind::Property),
    ];

    // Import Client from pkg-a: root must pick id 100, and the walk must stay on
    // pkg-a's declarations through every hop.
    let fc = file_ctx(vec![import("Client", Some("pkg-a"))], None);
    assert_eq!(
        resolve_with_fc(&lookup, segs, "caller", &fc),
        Some(110),
        "expected pkg-a's status (110) but got pkg-b's first-winner (210)"
    );
}

// --- Sub-fix 1: this/Self yield rebind (fluent / builder chains) -------------

/// A member whose declared return type is the string `"this"` is a fluent
/// builder step. `builder.set(x).build()` must resolve `build` on the
/// RECEIVER type (`Builder`), not on a non-existent class literally named
/// `"this"`. `yield_through` must substitute the receiver type when the
/// yielded type's head is `"this"` or `"Self"`.
#[test]
fn this_return_rebinds_to_receiver_for_fluent_chain() {
    let lookup = Lookup::new()
        .with_local_type("builder", "Builder")
        .with_member("Builder", sym(10, "set", "Builder.set", "method", "a.ts"))
        .with_return_type("Builder.set", "this")
        .with_member("Builder", sym(20, "build", "Builder.build", "method", "a.ts"));
    let segs = vec![
        seg("builder", false, SegmentKind::Identifier),
        seg("set", true, SegmentKind::Property),
        seg("build", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

/// The `Self` keyword (Rust/Swift) follows the same rebind rule as `this`.
#[test]
fn self_return_rebinds_to_receiver_for_fluent_chain() {
    let lookup = Lookup::new()
        .with_local_type("qb", "QueryBuilder")
        .with_member("QueryBuilder", sym(10, "where_", "QueryBuilder.where_", "method", "a.ts"))
        .with_return_type("QueryBuilder.where_", "Self")
        .with_member("QueryBuilder", sym(20, "execute", "QueryBuilder.execute", "method", "a.ts"));
    let segs = vec![
        seg("qb", false, SegmentKind::Identifier),
        seg("where_", true, SegmentKind::Property),
        seg("execute", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(20));
}

// --- Sub-fix 2: callable-property root (vi.fn case) -------------------------

/// `vi.fn()` where `fn` is a PROPERTY whose field type is a function type
/// `() => Mock<T>`. Calling `vi.fn()` must unwrap the call signature's
/// return type (`Mock<T>`), not stop at the property's declared type
/// `() => Mock<T>` (which has no members). The member is a property
/// (kind="property"), but `is_call=true` at the segment — `yield_through`
/// must detect the function-type field and return its return type.
#[test]
fn callable_property_field_type_yields_call_signature_return() {
    let lookup = Lookup::new()
        .with_local_type("vi", "Vi")
        // `fn` property on Vi whose field type is a function type
        .with_member("Vi", sym(5, "fn", "Vi.fn", "property", "ext:ts:vitest/index.d.ts"))
        .with_field_type("Vi.fn", "() => MockInstance")
        .with_member(
            "MockInstance",
            sym(80, "mockReturnValue", "MockInstance.mockReturnValue", "method", "ext:ts:vitest/index.d.ts"),
        );
    let segs = vec![
        seg("vi", false, SegmentKind::Identifier),
        seg("fn", true, SegmentKind::Property),
        seg("mockReturnValue", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(80));
}

/// A generic callable property: `fn: <T>() => Mock<T>`. The call-site's
/// type arg threads through to resolve the next member on the bound type.
#[test]
fn callable_property_field_type_with_generic_arg_substitutes_type() {
    // `vi.fn<User>()` — the call yields `Mock<User>`, and `mock.results[0].value`
    // would type as User. Simplified: fn is `() => Mock<T>`, fn<User>() → Mock<User>.
    let lookup = Lookup::new()
        .with_local_type("vi", "Vi")
        .with_generics("Vi.fn", &["T"])
        .with_member("Vi", sym(5, "fn", "Vi.fn", "property", "ext:ts:vitest/index.d.ts"))
        .with_field_type("Vi.fn", "() => MockInstance")
        .with_member(
            "MockInstance",
            sym(80, "mockResolvedValue", "MockInstance.mockResolvedValue", "method", "ext:ts:vitest/index.d.ts"),
        );
    let segs = vec![
        seg("vi", false, SegmentKind::Identifier),
        seg("fn", true, SegmentKind::Property),
        seg("mockResolvedValue", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(80));
}

// --- Sub-fix 3: mid-chain stdlib receiver typing (Array/String) --------------

/// `getItems()` returns `User[]`; the chain walker must advance through the
/// yielded array type to `Array.map`. This validates that `return_type = "User[]"`
/// interns correctly as `Apply(Array, [User])` and `head_qname` returns `Array`
/// for the next member lookup. If this already works, it confirms the path; if
/// it was broken, the test finds it.
#[test]
fn mid_chain_array_return_type_reaches_array_members() {
    let lookup = Lookup::new()
        .with(sym(1, "getItems", "getItems", "function", "a.ts"))
        .with_return_type("getItems", "User[]")
        .with_member(
            "Array",
            sym(60, "map", "Array.map", "method", "ext:ts:__ts_lib__/lib.es5.d.ts"),
        );
    let segs = vec![
        seg("getItems", true, SegmentKind::Identifier),
        seg("map", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(60));
}

/// `str.split(".")` where `str: string` — the receiver typed as stdlib
/// `string` (primitive) reaches `String.split`. The flow cache records a
/// declared param type `string`; the chain walker must route the primitive
/// through to `String`'s member index. Tests the primitive-to-nominal bridge
/// for string receivers.
#[test]
fn string_typed_receiver_reaches_string_members() {
    // The receiver is a local typed as `string`. The member lookup must
    // find `split` on the `String` class (the nominal form of the primitive).
    let lookup = Lookup::new()
        .with_local_type("str", "string")
        .with_member(
            "String",
            sym(70, "split", "String.split", "method", "ext:ts:__ts_lib__/lib.es5.d.ts"),
        );
    let segs = vec![
        seg("str", false, SegmentKind::Identifier),
        seg("split", true, SegmentKind::Property),
    ];
    assert_eq!(resolve(&lookup, segs, "caller"), Some(70));
}
