// =============================================================================
// type_checker/core/supertype_tests.rs — Unit tests for SupertypeGraph.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::SymbolInfo;
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
    ExtractedRef {
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
    ExtractedRef {
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
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &lookup);

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
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &lookup);

    let admin_id = arena.class("Admin");
    let external_id = arena.class("ExternalBase");
    assert_eq!(graph.parents_of(admin_id), &[external_id]);
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
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &lookup);

    let admin = arena.class("Admin");
    let parents: Vec<&str> = graph
        .parents_of(admin)
        .iter()
        .map(|t| match arena.get(*t) {
            crate::type_checker::core::types::Type::Class(q) => q.as_str(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(parents.len(), 3);
    assert!(parents.contains(&"User"));
    assert!(parents.contains(&"Role"));
    assert!(parents.contains(&"Auditable"));
}

#[test]
fn build_explicit_ignores_non_inheritance_refs() {
    let mut arena = TypeArena::new();
    let lookup = TypeLookup::new();

    let parsed = vec![parsed_with_refs(
        "x.rs",
        vec![class_sym("Admin", "Admin")],
        vec![ExtractedRef {
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
    let profile = crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &lookup);

    let admin = arena.class("Admin");
    assert!(graph.parents_of(admin).is_empty());
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

#[test]
fn build_structural_links_class_whose_members_superset_interface() {
    let mut arena = TypeArena::new();
    let writer = arena.class("Writer");
    let file = arena.class("File");
    let socket = arena.class("Socket");

    let mut members = MembersIndex::new();
    members.add_direct(writer, method(1, "Write", "Writer"));
    members.add_direct(file, method(2, "Write", "File"));
    members.add_direct(file, method(3, "Close", "File"));
    members.add_direct(socket, method(4, "Close", "Socket"));

    let lookup = TypeLookup::new();
    let profile = crate::type_checker::profile::language_profile::LanguageProfile {
        supertype_discovery: SupertypeDiscovery::Structural,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(&[], &mut arena, &profile, &members, &lookup);

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
fn build_both_combines_explicit_and_structural() {
    let mut arena = TypeArena::new();
    let writer = arena.class("myapp.Writer");
    let file = arena.class("myapp.File");

    let lookup = TypeLookup::new().with_type("Writer", "myapp.Writer");

    let mut members = MembersIndex::new();
    members.add_direct(writer, method(1, "Write", "myapp.Writer"));
    members.add_direct(file, method(2, "Write", "myapp.File"));

    let parsed = vec![parsed_with_refs(
        "y.ts",
        vec![class_sym("Admin", "myapp.Admin")],
        vec![implements_ref(0, "Writer")],
    )];

    let profile = crate::type_checker::profile::language_profile::LanguageProfile {
        supertype_discovery: SupertypeDiscovery::Both,
        ..crate::type_checker::profile::language_profile::DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(&parsed, &mut arena, &profile, &members, &lookup);

    let admin_id = arena.class("myapp.Admin");
    assert!(graph.parents_of(admin_id).contains(&writer));
    assert!(graph.parents_of(file).contains(&writer));
}
