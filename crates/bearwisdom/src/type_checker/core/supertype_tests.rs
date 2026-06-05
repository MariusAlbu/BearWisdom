// =============================================================================
// type_checker/core/supertype_tests.rs — Unit tests for SupertypeGraph.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::SymbolInfo;
use crate::type_checker::core::symbol_types::{SymbolTypeData, SymbolTypeMap};
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Minimal SymbolLookup fixture — only types_by_name is consulted by the
// supertype builder for name resolution. Every other method returns empty.
// ---------------------------------------------------------------------------

struct TypeLookup {
    types: rustc_hash::FxHashMap<String, Vec<SymbolInfo>>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl TypeLookup {
    fn new() -> Self {
        Self {
            types: Default::default(),
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }

    fn with_type(mut self, name: &str, qname: &str) -> Self {
        let info = SymbolInfo {
            id: 1,
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: "class".to_string(),
            visibility: None,
            file_path: Arc::from("x.rs"),
            scope_path: None,
            package_id: None,
            signature: None,
        };
        self.types.entry(name.to_string()).or_default().push(info);
        self
    }
}

impl SymbolLookup for TypeLookup {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, name: &str) -> &[SymbolInfo] {
        self.types.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn alias_target(&self, _: &str) -> Option<&AliasTarget> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn class_sym(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn parsed_with_refs(
    path: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn inherits_ref(source_idx: usize, target: &str) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::Inherits,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn implements_ref(source_idx: usize, target: &str) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind: EdgeKind::Implements,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

/// The self-`TypeRef` an impl-container emits to its implementing type,
/// mirroring `extract_impl` (`languages/rust_lang/calls.rs`). `target_name` is
/// the raw implementing-type node text (`C` or `C<T>`), the structural carrier
/// the supertype builder reads to reroute the inheritance edge to `C`.
fn impl_typeref(source_idx: usize, impl_type: &str) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: source_idx,
        target_name: impl_type.to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

#[test]
fn add_edge_is_idempotent() {
    let mut arena = TypeArena::new();
    let admin = arena.class("Admin");
    let user = arena.class("User");

    let mut graph = SupertypeGraph::new();
    graph.add_edge(admin, user);
    graph.add_edge(admin, user);
    graph.add_edge(admin, user);
    assert_eq!(graph.parents_of(admin).len(), 1);
    assert_eq!(graph.parents_of(admin)[0], user);
}

#[test]
fn walk_up_emits_self_first_then_ancestors_bfs() {
    let mut arena = TypeArena::new();
    let a = arena.class("A");
    let b = arena.class("B");
    let c = arena.class("C");
    let d = arena.class("D");

    let mut graph = SupertypeGraph::new();
    graph.add_edge(a, b);
    graph.add_edge(a, c);
    graph.add_edge(b, d);
    graph.add_edge(c, d);

    let walk: Vec<TypeId> = graph.walk_up(a).collect();
    assert_eq!(walk[0], a, "self yielded first");
    assert!(walk.contains(&b));
    assert!(walk.contains(&c));
    assert_eq!(
        walk.iter().filter(|t| **t == d).count(),
        1,
        "d yielded only once even though reachable through two paths"
    );
}

#[test]
fn walk_up_handles_cycles() {
    let mut arena = TypeArena::new();
    let a = arena.class("A");
    let b = arena.class("B");

    let mut graph = SupertypeGraph::new();
    graph.add_edge(a, b);
    graph.add_edge(b, a);

    let walk: Vec<TypeId> = graph.walk_up(a).collect();
    assert_eq!(walk, vec![a, b]);
}

#[test]
fn build_explicit_creates_edges_from_inherits_refs() {
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new().with_type("User", "myapp.User");

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![class_sym("Admin", "myapp.Admin")],
        vec![inherits_ref(0, "User")],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let admin_id = arena.class("myapp.Admin");
    let user_id = arena.class("myapp.User");
    assert_eq!(graph.parents_of(admin_id), &[user_id]);
}

#[test]
fn build_explicit_falls_back_to_target_name_when_lookup_misses() {
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new();

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![class_sym("Admin", "Admin")],
        vec![inherits_ref(0, "ExternalBase")],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let admin_id = arena.class("Admin");
    let external_id = arena.class("ExternalBase");
    assert_eq!(graph.parents_of(admin_id), &[external_id]);
}

#[test]
fn build_explicit_includes_pulled_external_base_edges() {
    // EXT-3: a pulled external file's own inheritance must enter the graph so a
    // chain through an external base can climb the external's own hierarchy
    // (and `walk_up_with_args` compose generic args across those hops). Before
    // EXT-3, `ext:` files were skipped and the external base's edge was absent,
    // so `walk_up` stopped at the first external hop.
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new()
        .with_type("Repository", "pkg.Repository")
        .with_type("BaseRepo", "pkg.BaseRepo");

    let parsed = vec![
        // internal: UserRepo extends Repository
        parsed_with_refs(
            "src/repo.rs",
            vec![class_sym("UserRepo", "myapp.UserRepo")],
            vec![inherits_ref(0, "Repository")],
        ),
        // external (pulled by the demand loop): Repository extends BaseRepo
        parsed_with_refs(
            "ext:pkg/repository.d.ts",
            vec![class_sym("Repository", "pkg.Repository")],
            vec![inherits_ref(0, "BaseRepo")],
        ),
    ];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    // The external base's OWN supertype edge is now in the graph.
    let repo_id = arena.class("pkg.Repository");
    let base_id = arena.class("pkg.BaseRepo");
    assert_eq!(
        graph.parents_of(repo_id),
        &[base_id],
        "external Repository → BaseRepo edge must be present"
    );

    // And the chain climbs the whole way: UserRepo → Repository → BaseRepo.
    let userrepo_id = arena.class("myapp.UserRepo");
    let walk: Vec<TypeId> = graph.walk_up(userrepo_id).collect();
    assert!(
        walk.contains(&base_id),
        "walk_up from UserRepo must reach the deep external base BaseRepo"
    );
}

#[test]
fn build_explicit_handles_multi_inherit_and_implements() {
    // Admin extends User, Admin implements Role, Admin implements Auditable.
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new();

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![class_sym("Admin", "Admin")],
        vec![
            inherits_ref(0, "User"),
            implements_ref(0, "Role"),
            implements_ref(0, "Auditable"),
        ],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let admin = arena.class("Admin");
    let parents: Vec<String> = graph
        .parents_of(admin)
        .iter()
        .map(|t| match arena.get(*t) {
            crate::type_checker::core::types::Type::Class(q) => q,
            _ => panic!(),
        })
        .collect();
    assert_eq!(parents.len(), 3);
    assert!(parents.iter().any(|p| p == "User"));
    assert!(parents.iter().any(|p| p == "Role"));
    assert!(parents.iter().any(|p| p == "Auditable"));
}

#[test]
fn build_explicit_ignores_non_inheritance_refs() {
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new();

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![class_sym("Admin", "Admin")],
        vec![ExtractedRef { is_import_binding: false, is_reexport: false,
            source_symbol_index: 0,
            target_name: "doStuff".to_string(),
            kind: EdgeKind::Calls,
            line: 0,
            col: 0,
            module: None,
            namespace_segments: Vec::new(),
            chain: None,
            byte_offset: 0,
            call_args: Vec::new(),
        }],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let admin = arena.class("Admin");
    assert!(graph.parents_of(admin).is_empty());
}

/// A Rust `impl Trait for Type` container symbol, mirroring `extract_impl`:
/// a `Namespace`-kind symbol whose name/qname carry the `<impl Type@line>`
/// marker (suffixed to avoid colliding with `Type`'s real definition). The
/// implementing type is carried structurally by a sibling `TypeRef` edge
/// (`impl_typeref`), exactly as the extractor emits it.
fn impl_container_sym(impl_type: &str, line: u32) -> ExtractedSymbol {
    let short = format!("<impl {impl_type}@{line}>");
    ExtractedSymbol {
        name: short.clone(),
        qualified_name: short,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: Some(format!("impl {impl_type}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[test]
fn build_explicit_reroutes_impl_container_edge_to_implementing_type() {
    // Case B: `trait Greet { fn hello(&self){} }  struct Dog;  impl Greet for Dog {}`.
    // The Implements edge's source symbol is the `<impl Dog@N>` impl-container
    // (a Namespace), not `Dog` itself. The supertype edge must attach to the
    // IMPLEMENTING type `Dog`, so `Dog`'s ancestor walk reaches `Greet` and a
    // default method filed under `Greet` becomes reachable from `Dog`.
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new()
        .with_type("Dog", "Dog")
        .with_type("Greet", "Greet");

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![impl_container_sym("Dog", 3)],
        vec![implements_ref(0, "Greet"), impl_typeref(0, "Dog")],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let dog = arena.class("Dog");
    let greet = arena.class("Greet");
    let impl_container = arena.class("<impl Dog@3>");

    assert!(
        graph.parents_of(dog).contains(&greet),
        "the C->Trait edge must attach to the implementing type Dog"
    );
    assert!(
        graph.parents_of(impl_container).is_empty(),
        "no dead edge keyed on the impl-container namespace"
    );
}

#[test]
fn impl_container_reroute_makes_trait_default_method_reachable() {
    // End-to-end Case B: the trait default method `hello` is filed under
    // `class("Greet")` (its scope is the trait body). After the impl-container
    // edge reroutes `Dog -> Greet`, `MembersIndex::lookup` from the concrete
    // receiver `Dog` walks the ancestor `Greet` and resolves `hello`.
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new()
        .with_type("Dog", "Dog")
        .with_type("Greet", "Greet");

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![impl_container_sym("Dog", 3)],
        vec![implements_ref(0, "Greet"), impl_typeref(0, "Dog")],
    )];

    // `hello` is a direct member of the trait Greet (default method body).
    let mut members = MembersIndex::new();
    let greet = arena.class("Greet");
    members.add_direct(greet, method(42, "hello", "Greet"));

    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let dog = arena.class("Dog");
    let hit = members
        .lookup(dog, "hello", EdgeKind::Calls, &graph, &arena, &profile)
        .expect("dog.hello() resolves to the trait default method through the rerouted edge");
    assert_eq!(hit.id, 42, "resolved symbol is the trait's default `hello`");
}

#[test]
fn build_explicit_reroutes_generic_impl_container_to_base_type() {
    // `impl<T: Display> Foo for Wrapper<T>`-style container: the self-TypeRef
    // carries the full implementing-type node text `Wrapper<T>`; the rerouted
    // child must be the bare base `Wrapper`. A bound in the impl param list
    // (`<T: Display>`) is irrelevant here because the implementing type is read
    // from the TypeRef target, not by scanning the impl signature — the prior
    // angle-bracket-string scan mishandled exactly this bounded-generic shape.
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new()
        .with_type("Wrapper", "Wrapper")
        .with_type("Display", "Display");

    let mut container = impl_container_sym("Wrapper", 7);
    container.name = "<impl Wrapper<T>@7>".to_string();
    container.qualified_name = "<impl Wrapper<T>@7>".to_string();
    container.signature = Some("impl<T: Display> Wrapper<T>".to_string());

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![container],
        vec![implements_ref(0, "Display"), impl_typeref(0, "Wrapper<T>")],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let wrapper = arena.class("Wrapper");
    let display = arena.class("Display");
    assert!(
        graph.parents_of(wrapper).contains(&display),
        "generic impl container must reroute to the bare base type Wrapper"
    );
}

#[test]
fn build_explicit_keeps_non_namespace_source_unchanged() {
    // Regression guard: a normal class `Inherits` edge (source is a real type,
    // not an impl-container Namespace) still keys the child on the source's
    // own qualified_name — the reroute must not touch it. This mirrors
    // `build_explicit_creates_edges_from_inherits_refs` but asserts the
    // impl-container path is inert for ordinary inheritance.
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new().with_type("User", "myapp.User");

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![class_sym("Admin", "myapp.Admin")],
        vec![inherits_ref(0, "User")],
    )];

    let members = MembersIndex::new();
    let symbol_types = SymbolTypeMap::new();
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let admin = arena.class("myapp.Admin");
    let user = arena.class("myapp.User");
    assert_eq!(graph.parents_of(admin), &[user]);
}

fn method(id: i64, name: &str, scope: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: format!("{scope}.{name}"),
        kind: "method".to_string(),
        visibility: None,
        file_path: Arc::from("x.go"),
        scope_path: Some(scope.to_string()),
        package_id: None,
        signature: None,
    }
}

// ---------------------------------------------------------------------------
// LANG-PY-1: ancestor-order axis (BFS vs C3 linearization).
//
// Asymmetric diamond where the two orders diverge:
//   O ; A(O) ; B(A) ; C(O) ; D(B, C)
// BFS(D)  = D, B, C, A, O   — C reached at depth 1 before A (A is B's parent)
// C3(D)   = D, B, A, C, O   — A precedes C: B's full linearization (B,A,O)
//                             contributes A ahead of C in the merge.
// A member overridden on BOTH A and C is therefore resolved on C under BFS
// and on A under C3 — order decides the winning override.
// ---------------------------------------------------------------------------

fn diamond_graph(arena: &mut TypeArena) -> (SupertypeGraph, [TypeId; 5]) {
    let o = arena.class("O");
    let a = arena.class("A");
    let b = arena.class("B");
    let c = arena.class("C");
    let d = arena.class("D");
    let mut g = SupertypeGraph::new();
    g.add_edge(a, o);
    g.add_edge(b, a);
    g.add_edge(c, o);
    g.add_edge(d, b);
    g.add_edge(d, c);
    (g, [o, a, b, c, d])
}

#[test]
fn linearize_bfs_matches_walk_up_with_args() {
    let mut arena = TypeArena::new();
    let (g, [_o, _a, _b, _c, d]) = diamond_graph(&mut arena);

    let bfs = g.linearize_with_args(d, &arena, AncestorOrder::Bfs);
    let walk = g.walk_up_with_args(d, &arena);
    assert_eq!(
        bfs, walk,
        "Bfs arm must be byte-identical to walk_up_with_args"
    );
}

#[test]
fn linearize_c3_orders_diamond_differently_than_bfs() {
    let mut arena = TypeArena::new();
    let (g, [o, a, b, c, d]) = diamond_graph(&mut arena);

    let bfs: Vec<TypeId> = g
        .linearize_with_args(d, &arena, AncestorOrder::Bfs)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let c3: Vec<TypeId> = g
        .linearize_with_args(d, &arena, AncestorOrder::C3)
        .into_iter()
        .map(|(t, _)| t)
        .collect();

    assert_eq!(bfs, vec![d, b, c, a, o], "BFS order");
    assert_eq!(c3, vec![d, b, a, c, o], "C3 MRO order");
    assert_ne!(bfs, c3, "the two orders must diverge on this diamond");
}

#[test]
fn linearize_c3_is_total_same_node_set_as_bfs() {
    let mut arena = TypeArena::new();
    let (g, _) = diamond_graph(&mut arena);
    let d = arena.class("D");

    let mut bfs_set: Vec<TypeId> = g
        .linearize_with_args(d, &arena, AncestorOrder::Bfs)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let mut c3_set: Vec<TypeId> = g
        .linearize_with_args(d, &arena, AncestorOrder::C3)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    bfs_set.sort();
    c3_set.sort();
    assert_eq!(bfs_set, c3_set, "C3 must yield the same node SET as BFS");
}

#[test]
fn linearize_c3_composes_generic_args_across_hops() {
    // A: B<X> ; B<T>: C<T>. Under both orders C must be reached with the
    // concrete [X], not the unbound parameter — C3 reuses the same
    // node_params / edge_args composition the BFS arm performs.
    use crate::type_checker::core::types::{GenericParamData, Type};

    let mut arena = TypeArena::new();
    let a = arena.class("A");
    let b = arena.class("B");
    let c = arena.class("C");
    let x = arena.class("X");

    // B<T>'s declared param.
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let t_ty = arena.intern(Type::Generic { param: t_param });

    let mut g = SupertypeGraph::new();
    g.record_node_params(b, vec![t_param]);
    g.add_edge_generic(a, b, vec![x]); // A: B<X>
    g.add_edge_generic(b, c, vec![t_ty]); // B<T>: C<T>

    let c3 = g.linearize_with_args(a, &arena, AncestorOrder::C3);
    let (_, c_args) = c3
        .iter()
        .find(|(node, _)| *node == c)
        .expect("C reachable under C3");
    assert_eq!(
        c_args.as_slice(),
        &[x],
        "C3 must compose B's binding T->X so C carries [X]"
    );
}

#[test]
fn find_on_chain_picks_different_override_under_c3_vs_bfs() {
    use crate::type_checker::profile::language_profile::{AncestorOrder, DEFAULT_PROFILE};

    let mut arena = TypeArena::new();
    let (g, [_o, a, _b, c, d]) = diamond_graph(&mut arena);

    // `m` overridden on both A and C. BFS reaches C first, C3 reaches A first.
    let mut members = MembersIndex::new();
    members.add_direct(a, method(10, "m", "A"));
    members.add_direct(c, method(20, "m", "C"));

    let bfs_profile = LanguageProfile {
        ancestor_order: AncestorOrder::Bfs,
        ..DEFAULT_PROFILE
    };
    let c3_profile = LanguageProfile {
        ancestor_order: AncestorOrder::C3,
        ..DEFAULT_PROFILE
    };

    let bfs_hit = members
        .lookup(d, "m", EdgeKind::Calls, &g, &arena, &bfs_profile)
        .expect("m resolves under BFS");
    let c3_hit = members
        .lookup(d, "m", EdgeKind::Calls, &g, &arena, &c3_profile)
        .expect("m resolves under C3");

    assert_eq!(bfs_hit.id, 20, "BFS resolves m on C (id 20)");
    assert_eq!(c3_hit.id, 10, "C3 resolves m on A (id 10)");
}

/// SymbolTypeData for a method, registered into `types` under `id`: `params`
/// positional parameter TypeIds, `ret` the return TypeId. Lets a structural
/// fixture carry the param/return shapes INFER-5 compares — the synthetic
/// `method()` helper leaves these absent, which under the sound structural
/// check yields Unknown (no edge).
fn record_method_types(types: &mut SymbolTypeMap, id: i64, params: Vec<TypeId>, ret: TypeId) {
    types.insert(
        id,
        SymbolTypeData {
            declared_type: None,
            return_type: Some(ret),
            param_types: params,
            generic_params: Vec::new(),
        },
    );
}

#[test]
fn build_structural_links_class_whose_members_superset_interface() {
    let mut arena = TypeArena::new();
    let writer = arena.class("Writer");
    let file = arena.class("File");
    let socket = arena.class("Socket");

    let byte_slice = arena.class("ByteSlice");
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);

    let mut members = MembersIndex::new();
    members.add_direct(writer, method(1, "Write", "Writer"));
    members.add_direct(file, method(2, "Write", "File"));
    members.add_direct(file, method(3, "Close", "File"));
    members.add_direct(socket, method(4, "Close", "Socket"));

    // Writer.Write and File.Write share the same param/return shape, so the
    // sound structural check accepts File. Socket has no Write member at all.
    let mut symbol_types = SymbolTypeMap::new();
    record_method_types(&mut symbol_types, 1, vec![byte_slice], int_ty);
    record_method_types(&mut symbol_types, 2, vec![byte_slice], int_ty);

    let lookup = TypeLookup::new();
    let profile = crate::type_checker::profile::language_profile::LanguageProfile {
        supertype_discovery: SupertypeDiscovery::Structural,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(&[], &mut arena, &profile, &members, &symbol_types, &lookup);

    assert!(
        graph.parents_of(file).contains(&writer),
        "File satisfies Writer interface structurally"
    );
    assert!(
        !graph.parents_of(socket).contains(&writer),
        "Socket missing Write must not satisfy Writer"
    );
}

#[test]
fn build_structural_rejects_incompatible_signature_via_infer5() {
    // The soundness upgrade: two structs declare a `Write` method with the
    // SAME name+kind as the interface's, but different param/return types.
    // Under the old name+kind matcher both linked; under the sound INFER-5
    // check only the type-compatible struct gets the edge — the incompatible
    // one yields Unknown (no edge).
    let mut arena = TypeArena::new();
    let writer = arena.class("Writer");
    let good_file = arena.class("GoodFile");
    let bad_write = arena.class("BadWrite");

    let byte_slice = arena.class("ByteSlice");
    let string_ty = arena.class("String");
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);

    let mut members = MembersIndex::new();
    members.add_direct(writer, method(1, "Write", "Writer"));
    members.add_direct(good_file, method(2, "Write", "GoodFile"));
    members.add_direct(bad_write, method(3, "Write", "BadWrite"));

    // Writer.Write: (ByteSlice) -> Int.
    // GoodFile.Write: (ByteSlice) -> Int  — type-compatible.
    // BadWrite.Write: (String) -> String  — same name+kind, incompatible types.
    let mut symbol_types = SymbolTypeMap::new();
    record_method_types(&mut symbol_types, 1, vec![byte_slice], int_ty);
    record_method_types(&mut symbol_types, 2, vec![byte_slice], int_ty);
    record_method_types(&mut symbol_types, 3, vec![string_ty], string_ty);

    let lookup = TypeLookup::new();
    let profile = crate::type_checker::profile::language_profile::LanguageProfile {
        supertype_discovery: SupertypeDiscovery::Structural,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(&[], &mut arena, &profile, &members, &symbol_types, &lookup);

    assert!(
        graph.parents_of(good_file).contains(&writer),
        "GoodFile's type-compatible Write must satisfy Writer"
    );
    assert!(
        !graph.parents_of(bad_write).contains(&writer),
        "BadWrite's incompatible-signature Write must NOT satisfy Writer (sound check)"
    );
}

#[test]
fn build_both_combines_explicit_and_structural() {
    let mut arena = TypeArena::new();
    let writer = arena.class("myapp.Writer");
    let file = arena.class("myapp.File");

    let lookup = TypeLookup::new().with_type("Writer", "myapp.Writer");

    let mut members = MembersIndex::new();
    members.add_direct(writer, method(1, "Write", "myapp.Writer"));
    members.add_direct(file, method(2, "Write", "myapp.File"));

    // Both Write methods share the same shape so the structural half links
    // File → Writer under the sound INFER-5 check.
    let byte_slice = arena.class("ByteSlice");
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);
    let mut symbol_types = SymbolTypeMap::new();
    record_method_types(&mut symbol_types, 1, vec![byte_slice], int_ty);
    record_method_types(&mut symbol_types, 2, vec![byte_slice], int_ty);

    let parsed = vec![parsed_with_refs(
        "y.ts",
        vec![class_sym("Admin", "myapp.Admin")],
        vec![implements_ref(0, "Writer")],
    )];

    let profile = crate::type_checker::profile::language_profile::LanguageProfile {
        supertype_discovery: SupertypeDiscovery::Both,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &symbol_types, &lookup);

    let admin_id = arena.class("myapp.Admin");
    assert!(graph.parents_of(admin_id).contains(&writer));
    assert!(graph.parents_of(file).contains(&writer));
}
