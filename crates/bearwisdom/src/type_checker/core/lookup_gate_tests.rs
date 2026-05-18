//! Gate-test for phase 3: the lookup layer must answer "find member X on
//! type T" correctly when MembersIndex + SupertypeGraph are built from a
//! real ParsedFile produced by an actual extractor.
//!
//! Mirrors phase 1's `foundation_gate_tests.rs` shape — extract once,
//! build the index from the resulting symbols, then exercise the public
//! lookup API on the resulting TypeIds.

use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::languages::typescript::extract;
use crate::type_checker::core::{
    MembersIndex, SymbolIdMap, SupertypeGraph,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{AliasTarget, EdgeKind, ParsedFile};
use std::sync::Arc;

fn wrap_as_parsed_file(path: &str, source: &str) -> ParsedFile {
    let extraction = extract::extract(source, false);
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: source.len() as u64,
        line_count: source.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

/// Build a sym_id_map that assigns deterministic ids `1..=N` to a file's
/// symbols. The id space is identical to what the indexer would assign on
/// fresh insert, so the gate test is representative.
fn deterministic_ids(pf: &ParsedFile) -> SymbolIdMap {
    let mut map = SymbolIdMap::default();
    for (idx, _) in pf.symbols.iter().enumerate() {
        map.insert((pf.path.clone(), idx), idx as i64 + 1);
    }
    map
}

/// Minimal SymbolLookup that resolves `types_by_name` against the parsed
/// file's symbol set. The supertype builder consults this to map a ref's
/// simple `target_name` to the matching type's qualified name.
struct ParsedFileLookup {
    types: Vec<SymbolInfo>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl ParsedFileLookup {
    fn from(pf: &ParsedFile, sym_ids: &SymbolIdMap) -> Self {
        let mut types = Vec::new();
        let file_path: Arc<str> = Arc::from(pf.path.as_str());
        for (idx, sym) in pf.symbols.iter().enumerate() {
            let id = sym_ids
                .get(&(pf.path.clone(), idx))
                .copied()
                .unwrap_or(idx as i64 + 1);
            types.push(SymbolInfo {
                id,
                name: sym.name.clone(),
                qualified_name: sym.qualified_name.clone(),
                kind: sym.kind.as_str().to_string(),
                visibility: sym.visibility.map(|v| v.as_str().to_string()),
                file_path: file_path.clone(),
                scope_path: sym.scope_path.clone(),
                package_id: pf.package_id,
                signature: sym.signature.clone(),
            });
        }
        Self {
            types,
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }
}

impl SymbolLookup for ParsedFileLookup {
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
        // Linear scan — fine for unit-test fixture sizes; not used in prod.
        let matches: Vec<&SymbolInfo> = self
            .types
            .iter()
            .filter(|s| s.name == name && matches!(s.kind.as_str(), "class" | "interface" | "trait" | "struct" | "enum" | "type_alias"))
            .collect();
        if matches.len() == 1 {
            std::slice::from_ref(matches[0])
        } else {
            &self.empty
        }
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

#[test]
fn lookup_finds_direct_method_on_class_from_real_ts_extraction() {
    let source = r#"
export class User {
    name: string;
    age: number;
    greet(prefix: string): string {
        return prefix + this.name;
    }
}
"#;
    let pf = wrap_as_parsed_file("src/user.ts", source);
    let sym_ids = deterministic_ids(&pf);
    let lookup = ParsedFileLookup::from(&pf, &sym_ids);

    let mut arena = crate::type_checker::core::TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );
    let graph =
        SupertypeGraph::build(std::slice::from_ref(&pf), &mut arena, &DEFAULT_PROFILE, &members, &lookup);

    let user_ty = arena.class("User");
    let greet = members
        .lookup(user_ty, "greet", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("greet method resolves on User");
    assert_eq!(greet.name, "greet");
    assert_eq!(greet.qualified_name, "User.greet");
    assert_eq!(greet.kind, "method");
}

#[test]
fn lookup_walks_inheritance_from_real_ts_extends() {
    let source = r#"
export class User {
    name: string;
    greet(): string { return this.name; }
}
export class Admin extends User {
    level: number;
}
"#;
    let pf = wrap_as_parsed_file("src/auth.ts", source);
    let sym_ids = deterministic_ids(&pf);
    let lookup = ParsedFileLookup::from(&pf, &sym_ids);

    let mut arena = crate::type_checker::core::TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &lookup,
    );

    let admin_ty = arena.class("Admin");
    let greet = members
        .lookup(admin_ty, "greet", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("greet inherited from User resolves on Admin");
    assert_eq!(greet.qualified_name, "User.greet");
}

#[test]
fn lookup_finds_interface_member_via_implements() {
    let source = r#"
export interface Greeter {
    greet(name: string): string;
}
export class HelloGreeter implements Greeter {
    greet(name: string): string { return "Hello " + name; }
}
"#;
    let pf = wrap_as_parsed_file("src/greet.ts", source);
    let sym_ids = deterministic_ids(&pf);
    let lookup = ParsedFileLookup::from(&pf, &sym_ids);

    let mut arena = crate::type_checker::core::TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &lookup,
    );

    // Direct hit on HelloGreeter.greet.
    let hg_ty = arena.class("HelloGreeter");
    let m = members
        .lookup(hg_ty, "greet", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("greet on HelloGreeter");
    assert_eq!(m.qualified_name, "HelloGreeter.greet");

    // Via Greeter interface alone — the structural method declaration on
    // the interface is itself a direct member of the interface symbol.
    let g_ty = arena.class("Greeter");
    let gm = members
        .lookup(g_ty, "greet", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .expect("greet on Greeter interface");
    assert_eq!(gm.qualified_name, "Greeter.greet");
}

#[test]
fn lookup_misses_when_member_absent() {
    let source = r#"
export class User { name: string; }
"#;
    let pf = wrap_as_parsed_file("src/u.ts", source);
    let sym_ids = deterministic_ids(&pf);
    let lookup = ParsedFileLookup::from(&pf, &sym_ids);

    let mut arena = crate::type_checker::core::TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &lookup,
    );

    let user = arena.class("User");
    assert!(members
        .lookup(user, "doesNotExist", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .is_none());
}
