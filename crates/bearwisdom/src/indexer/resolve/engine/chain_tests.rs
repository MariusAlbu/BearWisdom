use super::*;
use crate::indexer::resolve::engine::testkit::{call_ref, ref_ctx, source_symbol, sym, Lookup};
use crate::types::{ChainSegment, MemberChain};

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
    let leaf = segs.last().unwrap().name.clone();
    let mut r = call_ref(&leaf);
    r.chain = Some(MemberChain { segments: segs });
    let mut s = source_symbol("caller");
    s.qualified_name = src_qname.to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(&rc, lookup).map(|res| res.target_symbol_id)
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
