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
    infer_expression_type, MembersIndex, SupertypeGraph, SymbolIdMap, SymbolTypeMap,
    Type, TypeArena,
};
use crate::type_checker::profile::language_profile::{
    LanguageProfile, SupertypeDiscovery, DEFAULT_PROFILE,
};
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
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &symbol_types,
        &lookup,
    );

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
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &symbol_types,
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
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &symbol_types,
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
fn members_and_symbol_types_key_consistently_for_same_parsed_file() {
    // Phase 4 will look up a SymbolInfo via MembersIndex then dereference
    // its declared/return TypeId through SymbolTypeMap. Both maps must be
    // built off the same arena and the same sym_id_map; this gate verifies
    // the two halves agree on TypeIds for every type-defining symbol in a
    // real extraction.
    let source = r#"
export class Repository<T> {
    items: T[] = [];
    push(item: T): void { this.items.push(item); }
}
export interface Identified {
    id: string;
}
"#;
    let pf = wrap_as_parsed_file("src/repo.ts", source);
    let sym_ids = deterministic_ids(&pf);

    let mut arena = crate::type_checker::core::TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );
    let types = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );

    // For every type-defining symbol, the TypeId in SymbolTypeMap.return_type
    // must equal arena.class(qname) — the same TypeId MembersIndex would have
    // keyed for that type.
    for (idx, sym) in pf.symbols.iter().enumerate() {
        if !matches!(
            sym.kind,
            crate::types::SymbolKind::Class
                | crate::types::SymbolKind::Interface
                | crate::types::SymbolKind::Struct
                | crate::types::SymbolKind::Trait
                | crate::types::SymbolKind::Enum
                | crate::types::SymbolKind::TypeAlias
                | crate::types::SymbolKind::Delegate
        ) {
            continue;
        }
        let id = sym_ids[&(pf.path.clone(), idx)];
        let data = types
            .get(id)
            .unwrap_or_else(|| panic!("SymbolTypeMap missing entry for {}", sym.qualified_name));
        let return_ty = data
            .return_type
            .expect("type-defining symbol should self-yield");
        let class_ty = arena.class(&sym.qualified_name);
        assert_eq!(
            return_ty, class_ty,
            "SymbolTypeMap and MembersIndex must key the same TypeId for {}",
            sym.qualified_name
        );
        // The member-set keyed under that TypeId is what the chain walker
        // will hit when it derefs the self-yield. For Repository / Identified
        // there must be at least one direct member registered.
        assert!(
            !members.direct_of(class_ty).is_empty(),
            "expected direct members under {} (TypeId from SymbolTypeMap)",
            sym.qualified_name
        );
    }
}

#[test]
fn infer_expression_type_recognises_real_instantiates_ref() {
    // Build a tiny TS program with a `new Foo()` site; locate that ref
    // and feed it through infer_expression_type. The inference fallback
    // should produce arena.class("Foo") even without a Resolution.
    let source = r#"
export class Foo { id: number = 0; }
export function makeFoo(): Foo { return new Foo(); }
"#;
    let pf = wrap_as_parsed_file("src/foo.ts", source);

    let inst_ref = pf
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Instantiates && r.target_name == "Foo")
        .expect("extractor must emit Instantiates ref for `new Foo()`");

    let mut arena = crate::type_checker::core::TypeArena::new();
    let out = infer_expression_type(inst_ref, None, &mut arena, &DEFAULT_PROFILE)
        .expect("Instantiates fallback yields the class TypeId");
    let foo = arena.class("Foo");
    assert_eq!(out, foo);
    // And dereferencing the TypeId yields the canonical Class shape.
    assert_eq!(arena.get(out), Type::Class("Foo".into()));
}

#[test]
fn structural_typing_gates_real_go_satisfaction_on_method_type_data() {
    // Go's structural typing: a struct satisfies an interface implicitly when
    // its method set is a superset AND the matched method signatures are
    // type-compatible. The supertype builder runs the sound INFER-5 check, so
    // a structural edge forms ONLY when both matched methods carry recorded
    // param/return TypeIds. The Go extractor records the method signature as a
    // string but does not yet intern param/return TypeIds onto the
    // ExtractedSymbol, so the sound check stays Unknown and no structural edge
    // forms here. The live member-resolution direction (calling `Write` on the
    // struct, resolved on the struct's own body) is unaffected.
    use crate::languages::go::extract;

    let source = r#"
package main

type Writer interface {
    Write(p []byte) (int, error)
}

type FileBuffer struct {
    data []byte
}

func (f *FileBuffer) Write(p []byte) (int, error) {
    f.data = append(f.data, p...)
    return len(p), nil
}

func (f *FileBuffer) Close() error {
    return nil
}
"#;
    let extraction = extract::extract(source);
    let pf = ParsedFile {
        path: "main.go".to_string(),
        language: "go".to_string(),
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
    };
    let sym_ids = deterministic_ids(&pf);
    let lookup = ParsedFileLookup::from(&pf, &sym_ids);

    let mut arena = TypeArena::new();
    let members = MembersIndex::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
    );

    let symbol_types = SymbolTypeMap::build_from_parsed_files(
        std::slice::from_ref(&pf),
        &sym_ids,
        &mut arena,
        &DEFAULT_PROFILE,
    );

    let go_structural_profile = LanguageProfile {
        supertype_discovery: SupertypeDiscovery::Structural,
        ..DEFAULT_PROFILE
    };
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &go_structural_profile,
        &members,
        &symbol_types,
        &lookup,
    );

    // Find the interface and struct qnames produced by the extractor.
    // Go extractor qualifies with package name (`main`).
    let writer_qname = pf
        .symbols
        .iter()
        .find(|s| s.name == "Writer" && s.kind == crate::types::SymbolKind::Interface)
        .map(|s| s.qualified_name.clone())
        .expect("Writer interface must be extracted");
    let buf_qname = pf
        .symbols
        .iter()
        .find(|s| s.name == "FileBuffer" && s.kind == crate::types::SymbolKind::Struct)
        .map(|s| s.qualified_name.clone())
        .expect("FileBuffer struct must be extracted");

    let writer_ty = arena.class(&writer_qname);
    let buf_ty = arena.class(&buf_qname);

    // No structural edge: the Go extractor records no param/return TypeIds on
    // the Write methods, so the sound INFER-5 check returns Unknown. When the
    // extractor interns method param/return types this edge re-forms — the
    // soundness gate is on type-data hydration, not the structural algorithm.
    assert!(
        !graph.parents_of(buf_ty).contains(&writer_ty),
        "FileBuffer must NOT gain a structural edge while Write carries no recorded type data (parents = {:?})",
        graph.parents_of(buf_ty)
    );

    // The live member-resolution direction is preserved: a call to `Write` on
    // FileBuffer resolves on FileBuffer's own Write body, independent of the
    // structural edge.
    let write_member = members
        .lookup(
            buf_ty,
            "Write",
            EdgeKind::Calls,
            &graph,
            &arena,
            &go_structural_profile,
        )
        .expect("Write must resolve on FileBuffer");
    assert_eq!(write_member.name, "Write");
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
    let symbol_types = SymbolTypeMap::new();
    let graph = SupertypeGraph::build(
        std::slice::from_ref(&pf),
        &mut arena,
        &DEFAULT_PROFILE,
        &members,
        &symbol_types,
        &lookup,
    );

    let user = arena.class("User");
    assert!(members
        .lookup(user, "doesNotExist", EdgeKind::Calls, &graph, &arena, &DEFAULT_PROFILE)
        .is_none());
}
