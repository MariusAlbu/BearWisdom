// =============================================================================
// type_checker/chain_tests.rs — sibling tests for the unified chain walker.
//
// These exercise the TypeScript-specific deltas that QUAL-2b-ts folded into
// `resolve_via_chain` as `ChainExtensions` data: alias expansion, inheritance
// climbing, external-qname promotion, `new X().m()` construction roots,
// call-root inference (import + tsconfig alias + ambient-globals fallback).
// The `..._NONE_...` cases confirm a config with `ChainExtensions::NONE` (the
// 5 already-migrated languages) does NOT trip the new fallback hops.
// =============================================================================

use super::{
    identity_normalize, resolve_via_chain, ChainConfig, ChainExtensions, NamespaceLookup,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolInfo, SymbolLookup,
};
use crate::languages::c_lang::hooks::C_LANG_CHAIN_CONFIG;
use crate::languages::csharp::hooks::CSHARP_CHAIN_CONFIG;
use crate::languages::go::hooks::GO_CHAIN_CONFIG;
use crate::languages::java::hooks::JAVA_CHAIN_CONFIG;
use crate::languages::php::hooks::PHP_CHAIN_CONFIG;
use crate::languages::python::hooks::PYTHON_CHAIN_CONFIG;
use crate::languages::ruby::hooks::RUBY_CHAIN_CONFIG;
use crate::languages::rust_lang::hooks::RUST_CHAIN_CONFIG;
use crate::languages::typescript::hooks::TS_CHAIN_CONFIG;
use crate::types::{
    AliasTarget, ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind,
    SymbolKind, Visibility,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Synthetic lookup: a flat symbol table plus per-qname field/return/parent/
// alias maps. Just enough to drive `resolve_via_chain`.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeLookup {
    by_qname: Vec<SymbolInfo>,
    field_types: Vec<(String, String)>,
    return_types: Vec<(String, String)>,
    parents: Vec<(String, String)>,
    aliases: Vec<(String, AliasTarget)>,
    path_aliases: Vec<(String, String)>,
    locals: Vec<(String, String)>,
    by_name_store: Vec<(String, Vec<SymbolInfo>)>,
    members_store: Vec<(String, Vec<SymbolInfo>)>,
    empty_syms: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl FakeLookup {
    fn sym(mut self, id: i64, qname: &str, kind: &str) -> Self {
        let name = qname.rsplit('.').next().unwrap().to_string();
        self.by_qname.push(SymbolInfo {
            id,
            name,
            qualified_name: qname.to_string(),
            kind: kind.to_string(),
            visibility: Some("public".to_string()),
            file_path: Arc::from("ext:ts:fixture/__bw_synthetic__.d.ts"),
            scope_path: None,
            package_id: None,
            signature: None,
        });
        self
    }
    fn ret(mut self, qname: &str, ty: &str) -> Self {
        self.return_types.push((qname.to_string(), ty.to_string()));
        self
    }
    fn field(mut self, qname: &str, ty: &str) -> Self {
        self.field_types.push((qname.to_string(), ty.to_string()));
        self
    }
    fn parent(mut self, child: &str, parent: &str) -> Self {
        self.parents.push((child.to_string(), parent.to_string()));
        self
    }
    fn alias(mut self, name: &str, target: AliasTarget) -> Self {
        self.aliases.push((name.to_string(), target));
        self
    }
    fn path_alias(mut self, from: &str, to: &str) -> Self {
        self.path_aliases.push((from.to_string(), to.to_string()));
        self
    }
    fn local(mut self, name: &str, ty: &str) -> Self {
        self.locals.push((name.to_string(), ty.to_string()));
        self
    }
    /// Register a method symbol whose simple name `by_name` returns and whose
    /// `signature` drives the C# extension-method probe.
    fn ext_method(mut self, id: i64, qname: &str, kind: &str, signature: &str) -> Self {
        let name = qname.rsplit('.').next().unwrap().to_string();
        let sym = SymbolInfo {
            id,
            name: name.clone(),
            qualified_name: qname.to_string(),
            kind: kind.to_string(),
            visibility: Some("public".to_string()),
            file_path: Arc::from("ext:fixture/__bw_synthetic__.cs"),
            scope_path: None,
            package_id: None,
            signature: Some(signature.to_string()),
        };
        self.by_name_store.push((name, vec![sym]));
        self
    }
    /// Register a direct child symbol under `parent_qname` so `members_of`
    /// returns it.
    fn member(mut self, parent_qname: &str, id: i64, qname: &str, kind: &str) -> Self {
        let name = qname.rsplit('.').next().unwrap().to_string();
        let sym = SymbolInfo {
            id,
            name,
            qualified_name: qname.to_string(),
            kind: kind.to_string(),
            visibility: Some("public".to_string()),
            file_path: Arc::from("ext:fixture/__bw_synthetic__.cs"),
            scope_path: None,
            package_id: None,
            signature: None,
        };
        self.members_store.push((parent_qname.to_string(), vec![sym]));
        self
    }
}

impl SymbolLookup for FakeLookup {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        // Backed by `by_name_store` for the C# extension-method probe; the TS
        // delta tests don't register entries and fall through to empty.
        self.by_name_store
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&self.empty_syms)
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.iter().find(|s| s.qualified_name == qname)
    }
    fn members_of(&self, parent_qname: &str) -> &[SymbolInfo] {
        self.members_store
            .iter()
            .find(|(p, _)| p == parent_qname)
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&self.empty_syms)
    }
    fn types_by_name(&self, name: &str) -> &[SymbolInfo] {
        // Return the first type-kind match wrapped in a 1-slice via a cached
        // position. Simpler: scan and return a sub-slice when the unique match
        // sits at a stable index. For the tests we only need the is_type check
        // to be true for known type names — find the index and return that
        // single element.
        if let Some(pos) = self.by_qname.iter().position(|s| {
            s.name == name
                && matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
                )
        }) {
            std::slice::from_ref(&self.by_qname[pos])
        } else {
            &self.empty_syms
        }
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.empty_syms
    }
    fn field_type_name(&self, qname: &str) -> Option<&str> {
        self.field_types
            .iter()
            .find(|(q, _)| q == qname)
            .map(|(_, t)| t.as_str())
    }
    fn return_type_name(&self, qname: &str) -> Option<&str> {
        self.return_types
            .iter()
            .find(|(q, _)| q == qname)
            .map(|(_, t)| t.as_str())
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.aliases.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }
    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        self.parents
            .iter()
            .find(|(c, _)| c == class_qname)
            .map(|(_, p)| p.as_str())
    }
    fn resolve_path_alias(&self, _: Option<i64>, spec: &str) -> Option<String> {
        self.path_aliases
            .iter()
            .find(|(f, _)| f == spec)
            .map(|(_, t)| t.clone())
    }
    fn local_type(&self, name: &str) -> Option<String> {
        self.locals
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
    }
}

// ---------------------------------------------------------------------------
// Chain / ref construction helpers
// ---------------------------------------------------------------------------

fn seg(name: &str, kind: SegmentKind, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: "identifier".to_string(),
        kind,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        type_arg_ids: Vec::new(),
    }
}

fn ref_with_chain(segments: Vec<ChainSegment>, kind: EdgeKind) -> ExtractedRef {
    let leaf = segments.last().unwrap().name.clone();
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: leaf,
        kind,
        line: 1,
        col: 0,
        module: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        chain: Some(MemberChain { segments }),
        call_args: Vec::new(),
    }
}

fn src_symbol() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "caller".to_string(),
        qualified_name: "caller".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn file_ctx_with_imports(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/app.ts".to_string(),
        language: "typescript".to_string(),
        imports,
        file_namespace: None,
    }
}

fn import(name: &str, module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: false,
    }
}

fn run(
    config: &ChainConfig,
    chain_ref: &ExtractedRef,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<i64> {
    let src = src_symbol();
    let ref_ctx = RefContext {
        extracted_ref: chain_ref,
        source_symbol: &src,
        scope_chain: vec!["caller".to_string()],
        file_package_id: None,
    };
    resolve_via_chain(
        config,
        chain_ref.chain.as_ref().unwrap(),
        chain_ref.kind,
        Some(file_ctx),
        &ref_ctx,
        lookup,
    )
    .map(|r| r.target_symbol_id)
}

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: module.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

/// Like `run` but returns the full `Resolution` and accepts a custom scope
/// chain (SelfRef resolution reads the enclosing type from it).
fn run_res(
    config: &ChainConfig,
    chain_ref: &ExtractedRef,
    file_ctx: &FileContext,
    scope_chain: Vec<String>,
    lookup: &dyn SymbolLookup,
) -> Option<crate::indexer::resolve::engine::Resolution> {
    let src = src_symbol();
    let ref_ctx = RefContext {
        extracted_ref: chain_ref,
        source_symbol: &src,
        scope_chain,
        file_package_id: None,
    };
    resolve_via_chain(
        config,
        chain_ref.chain.as_ref().unwrap(),
        chain_ref.kind,
        Some(file_ctx),
        &ref_ctx,
        lookup,
    )
}

// ---------------------------------------------------------------------------
// Tests — TS-specific deltas, via TS_CHAIN_CONFIG.
// ---------------------------------------------------------------------------

#[test]
fn ts_chain_construction_root() {
    // `new Builder().withName()` — Construction root adopts `Builder` as the
    // receiver, then `withName` resolves under it.
    let lookup = FakeLookup::default()
        .sym(1, "Builder", "class")
        .sym(2, "Builder.withName", "method");
    let r = ref_with_chain(
        vec![
            seg("Builder", SegmentKind::Construction, true),
            seg("withName", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn ts_chain_alias_expansion_root() {
    // `m.get()` where `m: UserMap` and `type UserMap = Map`. The alias has no
    // members; expansion walks it to `Map`, where `get` lives.
    let lookup = FakeLookup::default()
        .local("m", "UserMap")
        .alias(
            "UserMap",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: Vec::new(),
            },
        )
        .sym(1, "Map.get", "method");
    let r = ref_with_chain(
        vec![
            seg("m", SegmentKind::Identifier, false),
            seg("get", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(1));
}

#[test]
fn ts_chain_inheritance_final_segment() {
    // `repo.findOne()` where `repo: UserRepo extends BaseRepo` and `findOne`
    // is declared on the parent. The inheritance walk climbs to BaseRepo.
    let lookup = FakeLookup::default()
        .local("repo", "UserRepo")
        .sym(1, "UserRepo", "class")
        .parent("UserRepo", "BaseRepo")
        .sym(2, "BaseRepo.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn ts_chain_external_qname_promotion() {
    // `a.equal()` where the local resolves to the short name `Assertion` but
    // the member lives under the external `chai.Assertion.equal`. The
    // promotion hop rewrites `Assertion` → `chai.Assertion`.
    let lookup = FakeLookup::default()
        .local("a", "Assertion")
        .sym(1, "chai.Assertion", "interface")
        .sym(2, "chai.Assertion.equal", "method");
    let r = ref_with_chain(
        vec![
            seg("a", SegmentKind::Identifier, false),
            seg("equal", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn ts_chain_call_root_via_import() {
    // `dayjs().format()` with `import dayjs from 'dayjs'`. The call root's
    // return type (`Dayjs`) seeds the chain; `format` resolves on it.
    let lookup = FakeLookup::default()
        .ret("dayjs.dayjs", "Dayjs")
        .sym(1, "Dayjs.format", "method");
    let r = ref_with_chain(
        vec![
            seg("dayjs", SegmentKind::Identifier, true),
            seg("format", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![import("dayjs", "dayjs")]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(1));
}

#[test]
fn ts_chain_call_root_via_tsconfig_alias() {
    // `createUser().save()` with `import { createUser } from '@/lib/user'`
    // and a tsconfig path alias `@/lib/user` → `src/lib/user`. The aliased
    // qname's return type (`User`) seeds the chain.
    let lookup = FakeLookup::default()
        .path_alias("@/lib/user", "src/lib/user")
        .ret("src/lib/user.createUser", "User")
        .sym(1, "User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("createUser", SegmentKind::Identifier, true),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![import("createUser", "@/lib/user")]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(1));
}

#[test]
fn ts_chain_call_root_via_npm_globals_fallback() {
    // `expect(x).toBe()` with NO import (vitest globals mode). The root
    // fallback probes `__npm_globals__.expect` → return_type `chai.Assertion`,
    // then `toBe` resolves under it.
    let globals = crate::ecosystem::npm::NPM_GLOBALS_MODULE;
    let lookup = FakeLookup::default()
        .ret(&format!("{globals}.expect"), "chai.Assertion")
        .sym(1, "chai.Assertion.toBe", "method");
    let r = ref_with_chain(
        vec![
            seg("expect", SegmentKind::Identifier, true),
            seg("toBe", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&TS_CHAIN_CONFIG, &r, &fc, &lookup), Some(1));
}

// ---------------------------------------------------------------------------
// Guard: a config with ChainExtensions::NONE (the 5 already-migrated
// languages) must NOT take the new fallback hops.
// ---------------------------------------------------------------------------

fn none_config() -> ChainConfig {
    ChainConfig {
        strategy_prefix: "ts",
        normalize_type: identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "interface"],
        static_type_kinds: &["class", "interface", "enum", "type_alias"],
        use_generics: true,
        namespace_lookup: NamespaceLookup::None,
        kind_compatible: crate::languages::typescript::predicates::kind_compatible,
        extensions: ChainExtensions::NONE,
    }
}

#[test]
fn none_config_skips_construction_root() {
    // Same fixture as ts_chain_construction_root, but with NONE: the
    // Construction arm is gated off, so the root never resolves and the whole
    // chain misses.
    let lookup = FakeLookup::default()
        .sym(1, "Builder", "class")
        .sym(2, "Builder.withName", "method");
    let r = ref_with_chain(
        vec![
            seg("Builder", SegmentKind::Construction, true),
            seg("withName", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

#[test]
fn none_config_skips_inheritance_walk() {
    // Same fixture as ts_chain_inheritance_final_segment, but with NONE: the
    // inheritance walk is gated off, so the inherited member is never found.
    let lookup = FakeLookup::default()
        .local("repo", "UserRepo")
        .sym(1, "UserRepo", "class")
        .parent("UserRepo", "BaseRepo")
        .sym(2, "BaseRepo.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

#[test]
fn none_config_skips_alias_expansion() {
    // Same fixture as ts_chain_alias_expansion_root, but with NONE: the alias
    // is never expanded, so `get` is looked up on the alias name (no members)
    // and the chain misses.
    let lookup = FakeLookup::default()
        .local("m", "UserMap")
        .alias(
            "UserMap",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: Vec::new(),
            },
        )
        .sym(1, "Map.get", "method");
    let r = ref_with_chain(
        vec![
            seg("m", SegmentKind::Identifier, false),
            seg("get", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

// ---------------------------------------------------------------------------
// Tests — C# deltas via CSHARP_CHAIN_CONFIG. These exercise every path the
// deleted `walk_csharp_chain` covered, now through the generic engine:
// SelfRef enclosing-type root (incl. `record`), field-type root, static-type
// root, field-type intermediate hop, wildcard-`using` namespace lookup
// (intermediate + final), by_qualified_name final hit, members_of final
// fallback, inheritance climb, and the extension-method last resort.
// ---------------------------------------------------------------------------

#[test]
fn csharp_chain_self_ref_root_record() {
    // `this.Save()` inside a `record` — SelfRef resolves the enclosing type
    // from the scope chain. `record` is the C#-specific enclosing kind.
    let lookup = FakeLookup::default()
        .sym(1, "App.Order", "record")
        .sym(2, "App.Order.Save", "method");
    let r = ref_with_chain(
        vec![
            seg("this", SegmentKind::SelfRef, false),
            seg("Save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["App.Order".to_string()], &lookup)
            .map(|res| res.target_symbol_id),
        Some(2)
    );
}

#[test]
fn csharp_chain_field_type_root() {
    // `repo.Find()` where `repo: UserRepository` is a field on the enclosing
    // class. Phase 1 resolves the field type, Phase 3 resolves the member.
    let mut lookup = FakeLookup::default()
        .sym(1, "App.Svc", "class")
        .sym(2, "UserRepository.Find", "method");
    lookup
        .field_types
        .push(("App.Svc.repo".to_string(), "UserRepository".to_string()));
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("Find", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["App.Svc".to_string()], &lookup)
            .map(|res| res.target_symbol_id),
        Some(2)
    );
}

#[test]
fn csharp_chain_static_type_root_by_qname_final() {
    // `Math.Abs()` — `Math` is a static type root; `Abs` resolves directly via
    // by_qualified_name at confidence 1.0.
    let lookup = FakeLookup::default()
        .sym(1, "Math", "class")
        .sym(2, "Math.Abs", "method");
    let r = ref_with_chain(
        vec![
            seg("Math", SegmentKind::Identifier, false),
            seg("Abs", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("Math.Abs resolves");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "csharp_chain_resolution");
}

#[test]
fn csharp_chain_namespace_wildcard_final() {
    // `helper.Run()` where `helper: Helper` and `Helper.Run` is keyed under a
    // wildcard `using App.Utils` namespace (`App.Utils.Helper.Run`).
    let lookup = FakeLookup::default()
        .local("helper", "Helper")
        .sym(1, "App.Utils.Helper.Run", "method");
    let r = ref_with_chain(
        vec![
            seg("helper", SegmentKind::Identifier, false),
            seg("Run", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![wildcard_import("App.Utils")]);
    let res = run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("namespace-qualified final resolves");
    assert_eq!(res.target_symbol_id, 1);
    assert_eq!(res.confidence, 0.95);
}

#[test]
fn csharp_chain_members_of_final_fallback() {
    // `svc.Handle()` where `svc: Service` and `Handle` is a member of
    // `Service` reachable only via members_of (no by_qualified_name hit, no
    // namespace). walk_inheritance gates the generic members_of-final fallback.
    let lookup = FakeLookup::default()
        .local("svc", "Service")
        .member("Service", 1, "Service.Handle", "method");
    let r = ref_with_chain(
        vec![
            seg("svc", SegmentKind::Identifier, false),
            seg("Handle", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
            .map(|res| res.target_symbol_id),
        Some(1)
    );
}

#[test]
fn csharp_chain_inheritance_final_segment() {
    // `repo.FindOne()` where `repo: UserRepo extends BaseRepo` and `FindOne`
    // is declared on the parent. walk_inheritance climbs to BaseRepo.
    let lookup = FakeLookup::default()
        .local("repo", "UserRepo")
        .sym(1, "UserRepo", "class")
        .parent("UserRepo", "BaseRepo")
        .sym(2, "BaseRepo.FindOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("FindOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
            .map(|res| res.target_symbol_id),
        Some(2)
    );
}

#[test]
fn csharp_chain_extension_method_resolves_by_receiver() {
    // `s.Truncate(10)` binds to `static string Truncate(this string s, int n)`
    // in a static class — Truncate is not a member of `string`, so it resolves
    // via the extension-method last resort keyed on the `this` receiver.
    let lookup = FakeLookup::default()
        .local("s", "string")
        .ext_method(
            7,
            "App.StringExtensions.Truncate",
            "method",
            "public static string Truncate(this string value, int max)",
        );
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("Truncate", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("extension method resolves");
    assert_eq!(res.target_symbol_id, 7);
    assert_eq!(res.confidence, 0.85);
    assert_eq!(res.strategy, "csharp_extension_method");
}

#[test]
fn csharp_chain_extension_method_rejects_wrong_receiver() {
    // Same shape, but the extension's `this` receiver is `int`, not `string`.
    // The signature probe rejects it and the chain misses.
    let lookup = FakeLookup::default()
        .local("s", "string")
        .ext_method(
            7,
            "App.IntExtensions.Truncate",
            "method",
            "public static int Truncate(this int value, int max)",
        );
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("Truncate", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert!(
        run_res(&CSHARP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup).is_none()
    );
}

#[test]
fn none_config_skips_extension_method() {
    // The extension-method fixture under ChainExtensions::NONE: the probe is
    // gated off, so `Truncate` never binds and the chain misses.
    let lookup = FakeLookup::default()
        .local("s", "string")
        .ext_method(
            7,
            "App.StringExtensions.Truncate",
            "method",
            "public static string Truncate(this string value, int max)",
        );
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("Truncate", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

// ---------------------------------------------------------------------------
// Go differential tests (QUAL-2b-go).
//
// Anchor the GO_CHAIN_CONFIG case-space that the deleted walk_go_chain
// covered: identifier-rooted chains (no SelfRef), field-type progression,
// method return-type yield, static-type roots, scope-chain field roots, and
// the one Go delta — embedded-struct promotion — riding the shared
// walk_inheritance climb. A NONE-gate guard proves the embed promotion
// requires the flag. `enclosing_type_kinds`/SelfRef are absent in Go.
// ---------------------------------------------------------------------------

/// A segment whose `declared_type` seeds the chain root, mirroring the Go
/// extractor's `d: pkg.Derived` annotation on a receiver identifier.
fn seg_typed(name: &str, declared_type: &str) -> ChainSegment {
    let mut s = seg(name, SegmentKind::Identifier, false);
    s.declared_type = Some(declared_type.to_string());
    s
}

#[test]
fn go_chain_local_type_member_access() {
    // `u.Save()` where `u` is a local typed `User`, `Save` a method on `User`.
    let lookup = FakeLookup::default()
        .local("u", "User")
        .sym(1, "User", "struct")
        .sym(2, "User.Save", "method");
    let r = ref_with_chain(
        vec![
            seg("u", SegmentKind::Identifier, false),
            seg("Save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&GO_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("u.Save() resolves");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "go_chain_resolution");
}

#[test]
fn go_chain_field_type_progression() {
    // `s.repo.Find()` — s: Server, Server.repo: Repo, Repo.Find a method.
    let lookup = FakeLookup::default()
        .local("s", "Server")
        .sym(1, "Server", "struct")
        .field("Server.repo", "Repo")
        .sym(2, "Repo", "struct")
        .sym(3, "Repo.Find", "method");
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("repo", SegmentKind::Property, false),
            seg("Find", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run(&GO_CHAIN_CONFIG, &r, &fc, &lookup),
        Some(3),
        "s.repo.Find() walks the field type to Repo"
    );
}

#[test]
fn go_chain_method_return_type_yield() {
    // `c.DB().Begin()` — c: Client, Client.DB() returns Conn, Conn.Begin a method.
    let lookup = FakeLookup::default()
        .local("c", "Client")
        .sym(1, "Client", "struct")
        .ret("Client.DB", "Conn")
        .sym(2, "Conn", "struct")
        .sym(3, "Conn.Begin", "method");
    let r = ref_with_chain(
        vec![
            seg("c", SegmentKind::Identifier, false),
            seg("DB", SegmentKind::Property, true),
            seg("Begin", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run(&GO_CHAIN_CONFIG, &r, &fc, &lookup),
        Some(3),
        "c.DB().Begin() walks the return type to Conn"
    );
}

#[test]
fn go_chain_static_type_root() {
    // `Logger.New()` where `Logger` is itself a type name (no local, no field).
    let lookup = FakeLookup::default()
        .sym(1, "Logger", "struct")
        .sym(2, "Logger.New", "function");
    let r = ref_with_chain(
        vec![
            seg("Logger", SegmentKind::Identifier, false),
            seg("New", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&GO_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn go_chain_scope_field_root() {
    // `handler.Serve()` where `handler` is a field on the enclosing scope's
    // type — the root identifier resolves via scope-chain field lookup, not a
    // local or a type name. Mirrors the Phase-1 enclosing-field branch.
    let lookup = FakeLookup::default()
        .field("server.Server.handler", "Mux")
        .sym(1, "Mux", "struct")
        .sym(2, "Mux.Serve", "method");
    let r = ref_with_chain(
        vec![
            seg("handler", SegmentKind::Identifier, false),
            seg("Serve", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(
            &GO_CHAIN_CONFIG,
            &r,
            &fc,
            vec!["server.Server.Run".to_string(), "server.Server".to_string()],
            &lookup,
        )
        .map(|res| res.target_symbol_id),
        Some(2)
    );
}

#[test]
fn go_chain_embedded_struct_promotion() {
    // `d.Hello()` where d: Derived, Derived embeds Base (Inherits edge →
    // parent_class_qname), and Hello is a method on Base. The Go delta:
    // walk_inheritance climbs the embed and binds the promoted method.
    let lookup = FakeLookup::default()
        .sym(1, "pkg.Derived", "struct")
        .parent("pkg.Derived", "pkg.Base")
        .sym(2, "pkg.Base", "struct")
        .sym(3, "pkg.Base.Hello", "method");
    let r = ref_with_chain(
        vec![
            seg_typed("d", "pkg.Derived"),
            seg("Hello", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&GO_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("d.Hello() resolves via embedded promotion");
    assert_eq!(res.target_symbol_id, 3);
    assert_eq!(res.strategy, "go_chain_inheritance");
}

#[test]
fn go_chain_embedded_promotion_gated_by_none() {
    // The same embedded-promotion fixture under ChainExtensions::NONE: the
    // inheritance climb is gated off, so `Hello` never binds — proves the
    // promotion is the walk_inheritance flag, not the base walk.
    let lookup = FakeLookup::default()
        .sym(1, "pkg.Derived", "struct")
        .parent("pkg.Derived", "pkg.Base")
        .sym(2, "pkg.Base", "struct")
        .sym(3, "pkg.Base.Hello", "method");
    let r = ref_with_chain(
        vec![
            seg_typed("d", "pkg.Derived"),
            seg("Hello", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

#[test]
fn go_chain_empty_chain_returns_none() {
    // A single-segment chain (len < 2) is not a member walk — both the deleted
    // walker and resolve_via_chain return None.
    let lookup = FakeLookup::default().sym(1, "Foo", "struct");
    let r = ref_with_chain(vec![seg("Foo", SegmentKind::Identifier, false)], EdgeKind::Calls);
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&GO_CHAIN_CONFIG, &r, &fc, &lookup), None);
}

// ---------------------------------------------------------------------------
// Java differential tests (QUAL-2b-java).
//
// Anchor the JAVA_CHAIN_CONFIG case-space that the deleted walk_java_chain
// covered, now through the generic engine: SelfRef enclosing-type root
// (implicit `this.method()`), static-type root + by_qualified_name final at
// 1.0, field-type root progression, the wildcard-import namespace final hit at
// 0.95 (`NamespaceLookup::WildcardOnly`), and the inheritance climb for an
// inherited member (the bespoke walker's members_of/0.90 last resort folds
// into the shared walk_inheritance ladder). A NONE-gate guard proves the
// inheritance climb requires the flag.
// ---------------------------------------------------------------------------

#[test]
fn java_chain_self_ref_implicit_this() {
    // `this.create()` inside `com.example.OrderService` — SelfRef resolves the
    // enclosing class from the scope chain, then `create` resolves under it at
    // confidence 1.0.
    let lookup = FakeLookup::default()
        .sym(1, "com.example.OrderService", "class")
        .sym(2, "com.example.OrderService.create", "method");
    let r = ref_with_chain(
        vec![
            seg("this", SegmentKind::SelfRef, false),
            seg("create", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(
        &JAVA_CHAIN_CONFIG,
        &r,
        &fc,
        vec!["com.example.OrderService".to_string()],
        &lookup,
    )
    .expect("this.create() resolves via SelfRef enclosing class");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "java_chain_resolution");
}

#[test]
fn java_chain_static_type_root_by_qname_final() {
    // `Foo.staticMethod()` where `Foo` is a known type — static-type root, then
    // `staticMethod` resolves via by_qualified_name at confidence 1.0.
    let lookup = FakeLookup::default()
        .sym(1, "Foo", "class")
        .sym(2, "Foo.staticMethod", "method");
    let r = ref_with_chain(
        vec![
            seg("Foo", SegmentKind::Identifier, false),
            seg("staticMethod", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&JAVA_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("Foo.staticMethod() resolves");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "java_chain_resolution");
}

#[test]
fn java_chain_field_type_root() {
    // `repo.findOne()` where `repo: UserRepository` is a field on the enclosing
    // class. Phase 1 resolves the field type, Phase 3 resolves the member.
    let lookup = FakeLookup::default()
        .sym(1, "com.example.Svc", "class")
        .field("com.example.Svc.repo", "UserRepository")
        .sym(2, "UserRepository.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(
            &JAVA_CHAIN_CONFIG,
            &r,
            &fc,
            vec!["com.example.Svc".to_string()],
            &lookup,
        )
        .map(|res| res.target_symbol_id),
        Some(2)
    );
}

#[test]
fn java_chain_wildcard_namespace_final() {
    // `Foo.staticMethod()` with `import java.util.*;`. `Foo` is a known type so
    // the root resolves, but `staticMethod` is keyed only under the wildcard
    // namespace (`java.util.Foo.staticMethod`). The namespace-qualified final
    // hit lands at confidence 0.95.
    let lookup = FakeLookup::default()
        .sym(1, "Foo", "class")
        .sym(2, "java.util.Foo.staticMethod", "method");
    let r = ref_with_chain(
        vec![
            seg("Foo", SegmentKind::Identifier, false),
            seg("staticMethod", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![wildcard_import("java.util")]);
    let res = run_res(&JAVA_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("wildcard-import namespace final resolves");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 0.95);
    assert_eq!(res.strategy, "java_chain_resolution");
}

#[test]
fn java_chain_inheritance_final_segment() {
    // `repo.findOne()` where `repo: UserRepo extends BaseRepo` and `findOne` is
    // declared on the parent. walk_inheritance climbs the `extends` chain to
    // BaseRepo — folds in the bespoke walker's inherited-member coverage.
    let lookup = FakeLookup::default()
        .local("repo", "UserRepo")
        .sym(1, "UserRepo", "class")
        .parent("UserRepo", "BaseRepo")
        .sym(2, "BaseRepo.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&JAVA_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("repo.findOne() resolves via inheritance climb");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.strategy, "java_chain_inheritance");
}

#[test]
fn java_chain_inheritance_gated_by_none() {
    // The inheritance fixture under ChainExtensions::NONE: the climb is gated
    // off, so the inherited `findOne` never binds.
    let lookup = FakeLookup::default()
        .local("repo", "UserRepo")
        .sym(1, "UserRepo", "class")
        .parent("UserRepo", "BaseRepo")
        .sym(2, "BaseRepo.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

// ---------------------------------------------------------------------------
// Python differential tests (QUAL-2b-python).
//
// Anchor the PYTHON_CHAIN_CONFIG case-space the deleted walk_python_chain
// covered: `self.`-rooted chains resolving the enclosing class, local-typed
// roots, static-type-name roots, field-type progression, method return-type
// yield, and the by_qualified_name final hit. Python is `use_generics: false`
// with `ChainExtensions::NONE` — no inheritance climb, no namespace lookup.
// ---------------------------------------------------------------------------

#[test]
fn python_chain_self_ref_root() {
    // `self.save()` inside `app.User` — SelfRef resolves the enclosing class
    // from the scope chain, then `save` resolves under it.
    let lookup = FakeLookup::default()
        .sym(1, "app.User", "class")
        .sym(2, "app.User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("self", SegmentKind::SelfRef, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&PYTHON_CHAIN_CONFIG, &r, &fc, vec!["app.User".to_string()], &lookup)
        .expect("self.save() resolves via SelfRef enclosing class");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "python_chain_resolution");
}

#[test]
fn python_chain_local_type_member_access() {
    // `u.save()` where `u` is a local typed `User`, `save` a method on `User`.
    let lookup = FakeLookup::default()
        .local("u", "User")
        .sym(1, "User", "class")
        .sym(2, "User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("u", SegmentKind::Identifier, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&PYTHON_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn python_chain_field_type_progression() {
    // `s.repo.find()` — s: Service, Service.repo: Repo, Repo.find a method.
    let lookup = FakeLookup::default()
        .local("s", "Service")
        .sym(1, "Service", "class")
        .field("Service.repo", "Repo")
        .sym(2, "Repo", "class")
        .sym(3, "Repo.find", "method");
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("repo", SegmentKind::Property, false),
            seg("find", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&PYTHON_CHAIN_CONFIG, &r, &fc, &lookup), Some(3));
}

#[test]
fn python_chain_method_return_type_yield() {
    // `c.db().begin()` — c: Client, Client.db() returns Conn, Conn.begin a method.
    let lookup = FakeLookup::default()
        .local("c", "Client")
        .sym(1, "Client", "class")
        .ret("Client.db", "Conn")
        .sym(2, "Conn", "class")
        .sym(3, "Conn.begin", "method");
    let r = ref_with_chain(
        vec![
            seg("c", SegmentKind::Identifier, false),
            seg("db", SegmentKind::Property, true),
            seg("begin", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&PYTHON_CHAIN_CONFIG, &r, &fc, &lookup), Some(3));
}

#[test]
fn python_chain_static_type_root() {
    // `Logger.get()` where `Logger` is itself a class name (no local, no field).
    let lookup = FakeLookup::default()
        .sym(1, "Logger", "class")
        .sym(2, "Logger.get", "method");
    let r = ref_with_chain(
        vec![
            seg("Logger", SegmentKind::Identifier, false),
            seg("get", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&PYTHON_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

// ---------------------------------------------------------------------------
// Ruby differential tests (QUAL-2b-ruby).
//
// Anchor the RUBY_CHAIN_CONFIG case-space the deleted walk_ruby_chain covered:
// `self`-rooted chains resolving the enclosing class/module (`namespace` kind),
// local-typed roots, field-type progression, and the by_qualified_name final.
// Ruby is `use_generics: false` with `ChainExtensions::NONE`.
// ---------------------------------------------------------------------------

#[test]
fn ruby_chain_self_ref_root_namespace_enclosing() {
    // `self.process` inside an `Admin` module (indexed as `namespace`) — SelfRef
    // resolves the enclosing module, then `process` resolves under it.
    let lookup = FakeLookup::default()
        .sym(1, "Admin", "namespace")
        .sym(2, "Admin.process", "method");
    let r = ref_with_chain(
        vec![
            seg("self", SegmentKind::SelfRef, false),
            seg("process", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&RUBY_CHAIN_CONFIG, &r, &fc, vec!["Admin".to_string()], &lookup)
        .expect("self.process resolves via SelfRef enclosing module");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.strategy, "ruby_chain_resolution");
}

#[test]
fn ruby_chain_local_type_member_access() {
    // `u.save` where `u` is a local typed `User`, `save` a method on `User`.
    let lookup = FakeLookup::default()
        .local("u", "User")
        .sym(1, "User", "class")
        .sym(2, "User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("u", SegmentKind::Identifier, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&RUBY_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn ruby_chain_field_type_progression() {
    // `s.client.call` — s: Service, Service.client: Client, Client.call a method.
    let lookup = FakeLookup::default()
        .local("s", "Service")
        .sym(1, "Service", "class")
        .field("Service.client", "Client")
        .sym(2, "Client", "class")
        .sym(3, "Client.call", "method");
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("client", SegmentKind::Property, false),
            seg("call", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&RUBY_CHAIN_CONFIG, &r, &fc, &lookup), Some(3));
}

// ---------------------------------------------------------------------------
// PHP differential tests (QUAL-2b-php).
//
// Anchor the PHP_CHAIN_CONFIG case-space the deleted walk_php_chain covered:
// `$this->`-rooted chains, the `ClassName::method()` TypeAccess static root
// (`root_type_access`), local-typed field/return progression, the
// `use`-statement namespace final hit (`NamespaceLookup::AllImports`), the
// external-qname promotion hop (`promote_external_qname`), and the inheritance
// climb for `__callStatic`-forwarded members (`walk_inheritance`). PHP is
// `use_generics: false`.
// ---------------------------------------------------------------------------

#[test]
fn php_chain_self_ref_root() {
    // `$this->save()` inside `App.Models.User` — SelfRef resolves the enclosing
    // class, then `save` resolves under it.
    let lookup = FakeLookup::default()
        .sym(1, "App.Models.User", "class")
        .sym(2, "App.Models.User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("this", SegmentKind::SelfRef, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(
            &PHP_CHAIN_CONFIG,
            &r,
            &fc,
            vec!["App.Models.User".to_string()],
            &lookup,
        )
        .map(|res| res.target_symbol_id),
        Some(2)
    );
}

#[test]
fn php_chain_type_access_static_root() {
    // `User::find()` — the TypeAccess root resolves to the type's qualified name
    // (`App.Models.User`), then `find` resolves under it via by_qualified_name.
    let lookup = FakeLookup::default()
        .sym(1, "App.Models.User", "class")
        .sym(2, "App.Models.User.find", "method");
    let mut type_access = seg("User", SegmentKind::TypeAccess, false);
    type_access.node_kind = "name".to_string();
    let r = ref_with_chain(
        vec![type_access, seg("find", SegmentKind::Property, true)],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&PHP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("User::find() resolves via TypeAccess static root");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
}

#[test]
fn php_chain_type_access_root_gated_by_none() {
    // The same TypeAccess fixture under ChainExtensions::NONE: the root arm is
    // gated off, so the static call never resolves through the chain walk.
    let lookup = FakeLookup::default()
        .sym(1, "App.Models.User", "class")
        .sym(2, "App.Models.User.find", "method");
    let mut type_access = seg("User", SegmentKind::TypeAccess, false);
    type_access.node_kind = "name".to_string();
    let r = ref_with_chain(
        vec![type_access, seg("find", SegmentKind::Property, true)],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

#[test]
fn php_chain_field_type_progression() {
    // `$this->repo->find()` — root is `$this` (Repo field on the enclosing
    // class), then walk to Repo and resolve `find`.
    let lookup = FakeLookup::default()
        .sym(1, "App.Svc", "class")
        .field("App.Svc.repo", "Repo")
        .sym(2, "Repo", "class")
        .sym(3, "Repo.find", "method");
    let r = ref_with_chain(
        vec![
            seg("this", SegmentKind::SelfRef, false),
            seg("repo", SegmentKind::Property, false),
            seg("find", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run_res(&PHP_CHAIN_CONFIG, &r, &fc, vec!["App.Svc".to_string()], &lookup)
            .map(|res| res.target_symbol_id),
        Some(3)
    );
}

#[test]
fn php_chain_namespace_use_statement_final() {
    // `helper->run()` where `helper: Helper` and `Helper.run` is keyed under a
    // `use App\Utils` import (`App.Utils.Helper.run`). AllImports namespace
    // final hit lands at confidence 0.95.
    let lookup = FakeLookup::default()
        .local("helper", "Helper")
        .sym(1, "App.Utils.Helper.run", "method");
    let r = ref_with_chain(
        vec![
            seg("helper", SegmentKind::Identifier, false),
            seg("run", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![import("Helper", "App.Utils")]);
    let res = run_res(&PHP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("use-statement namespace final resolves");
    assert_eq!(res.target_symbol_id, 1);
    assert_eq!(res.confidence, 0.95);
}

#[test]
fn php_chain_external_qname_promotion() {
    // `m->where()` where the local resolves to the short `Builder` but the
    // member lives under the external `Illuminate.Builder.where`. The promotion
    // hop rewrites `Builder` → `Illuminate.Builder`.
    let lookup = FakeLookup::default()
        .local("m", "Builder")
        .sym(1, "Illuminate.Builder", "class")
        .sym(2, "Illuminate.Builder.where", "method");
    let r = ref_with_chain(
        vec![
            seg("m", SegmentKind::Identifier, false),
            seg("where", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&PHP_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn php_chain_inheritance_final_segment() {
    // `m->save()` where `m: Post extends Model` and `save` is declared on the
    // parent. walk_inheritance climbs the `extends` chain to Model — the
    // bespoke walker's `__callStatic`-forwarding coverage.
    let lookup = FakeLookup::default()
        .local("m", "Post")
        .sym(1, "Post", "class")
        .parent("Post", "Model")
        .sym(2, "Model.save", "method");
    let r = ref_with_chain(
        vec![
            seg("m", SegmentKind::Identifier, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&PHP_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("m->save() resolves via inheritance climb");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.strategy, "php_chain_inheritance");
}

// ---------------------------------------------------------------------------
// C/C++ differential tests (QUAL-2b-c_lang).
//
// Anchor the C_LANG_CHAIN_CONFIG case-space the deleted walk_c_lang_chain
// covered: local-typed roots, `::`→`.` type-name normalization on a C++
// qualified root, field-type progression, the by_qualified_name final hit, and
// the typedef-dereference post-hop that collapses a project-defined pointer
// typedef to its aliased struct mid-walk. C is `use_generics: false`.
// ---------------------------------------------------------------------------

#[test]
fn c_chain_local_type_member_access() {
    // `s.open()` where `s` is a local typed `Socket`, `open` a method on `Socket`.
    let lookup = FakeLookup::default()
        .local("s", "Socket")
        .sym(1, "Socket", "struct")
        .sym(2, "Socket.open", "method");
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("open", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&C_LANG_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("s.open() resolves");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.strategy, "c_chain_resolution");
}

#[test]
fn c_chain_cpp_namespace_normalization_root() {
    // `s.handle()` where `s` is a local typed with a C++ `::`-qualified name
    // `net::Server`; normalize_type rewrites it to `net.Server` so the member
    // keys correctly.
    let lookup = FakeLookup::default()
        .local("s", "net::Server")
        .sym(1, "net.Server", "class")
        .sym(2, "net.Server.handle", "method");
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("handle", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&C_LANG_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn c_chain_field_type_progression() {
    // `c.conn.send()` — c: Client, Client.conn: Conn, Conn.send a method.
    let lookup = FakeLookup::default()
        .local("c", "Client")
        .sym(1, "Client", "struct")
        .field("Client.conn", "Conn")
        .sym(2, "Conn", "struct")
        .sym(3, "Conn.send", "method");
    let r = ref_with_chain(
        vec![
            seg("c", SegmentKind::Identifier, false),
            seg("conn", SegmentKind::Property, false),
            seg("send", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&C_LANG_CHAIN_CONFIG, &r, &fc, &lookup), Some(3));
}

#[test]
fn c_chain_typedef_via_alias_expansion() {
    // `c.channel().write()` — Client.channel() returns the pointer typedef
    // `TChannelPtr` (a `type_alias` whose AliasTarget is `Channel`).
    // The generic alias-expansion path (`expand_aliases: true` in
    // C_LANG_CHAIN_CONFIG) rewrites `TChannelPtr` → `Channel` at each hop so
    // `write` resolves on the struct. No separate fn-pointer is needed.
    let lookup = FakeLookup::default()
        .local("c", "Client")
        .sym(1, "Client", "struct")
        .ret("Client.channel", "TChannelPtr")
        .sym(2, "TChannelPtr", "type_alias")
        .alias(
            "TChannelPtr",
            AliasTarget::Application {
                root: "Channel".to_string(),
                args: Vec::new(),
            },
        )
        .sym(3, "Channel", "struct")
        .sym(4, "Channel.write", "method");
    let r = ref_with_chain(
        vec![
            seg("c", SegmentKind::Identifier, false),
            seg("channel", SegmentKind::Property, true),
            seg("write", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run(&C_LANG_CHAIN_CONFIG, &r, &fc, &lookup),
        Some(4),
        "typedef pointer collapses to Channel via alias expansion mid-walk"
    );
}

// ---------------------------------------------------------------------------
// Rust differential tests (QUAL-2b-rust).
//
// Anchor the RUST_CHAIN_CONFIG case-space the deleted walk_rust_lang_chain
// covered, now through the generic engine: SelfRef enclosing-type root
// (`Self::method()` rooting on the enclosing struct/enum/trait), `::`→`.`
// type-name normalization on a qualified root, local-typed roots, field-type
// progression, method return-type yield, static-type roots, the
// by_qualified_name final hit (1.0), the members_of final fallback (0.95), and
// the trait-inheritance climb on the final segment (`walk_inheritance` →
// `rust_chain_inheritance`). A NONE-gate guard proves the inheritance climb
// requires the flag. Rust is `use_generics: false`.
// ---------------------------------------------------------------------------

#[test]
fn rust_chain_self_ref_root() {
    // `Self::save()` inside `crate.User` — SelfRef resolves the enclosing
    // struct from the scope chain, then `save` resolves under it at 1.0.
    let lookup = FakeLookup::default()
        .sym(1, "crate.User", "struct")
        .sym(2, "crate.User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("Self", SegmentKind::SelfRef, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&RUST_CHAIN_CONFIG, &r, &fc, vec!["crate.User".to_string()], &lookup)
        .expect("Self::save() resolves via SelfRef enclosing struct");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "rust_chain_resolution");
}

#[test]
fn rust_chain_local_type_member_access() {
    // `u.save()` where `u` is a local typed `User`, `save` a method on `User`.
    let lookup = FakeLookup::default()
        .local("u", "User")
        .sym(1, "User", "struct")
        .sym(2, "User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("u", SegmentKind::Identifier, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn rust_chain_path_separator_normalization_root() {
    // `p.query()` where `p` is a local typed with a `::`-qualified path
    // `crate::db::Pool`; normalize_path rewrites it to `crate.db.Pool` so the
    // member keys correctly against the index's `.`-separated qnames.
    let lookup = FakeLookup::default()
        .local("p", "crate::db::Pool")
        .sym(1, "crate.db.Pool", "struct")
        .sym(2, "crate.db.Pool.query", "method");
    let r = ref_with_chain(
        vec![
            seg("p", SegmentKind::Identifier, false),
            seg("query", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn rust_chain_field_type_progression() {
    // `s.repo.find()` — s: Server, Server.repo: Repository, Repository.find a method.
    let lookup = FakeLookup::default()
        .local("s", "Server")
        .sym(1, "Server", "struct")
        .field("Server.repo", "Repository")
        .sym(2, "Repository", "struct")
        .sym(3, "Repository.find", "method");
    let r = ref_with_chain(
        vec![
            seg("s", SegmentKind::Identifier, false),
            seg("repo", SegmentKind::Property, false),
            seg("find", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup),
        Some(3),
        "s.repo.find() walks the field type to Repository"
    );
}

#[test]
fn rust_chain_method_return_type_yield() {
    // `c.db().begin()` — c: Client, Client.db() returns Conn, Conn.begin a method.
    let lookup = FakeLookup::default()
        .local("c", "Client")
        .sym(1, "Client", "struct")
        .ret("Client.db", "Conn")
        .sym(2, "Conn", "struct")
        .sym(3, "Conn.begin", "method");
    let r = ref_with_chain(
        vec![
            seg("c", SegmentKind::Identifier, false),
            seg("db", SegmentKind::Property, true),
            seg("begin", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(
        run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup),
        Some(3),
        "c.db().begin() walks the return type to Conn"
    );
}

#[test]
fn rust_chain_static_type_root_by_qname_final() {
    // `DbPool::new()` where `DbPool` is itself a struct name (no local, no
    // field). The static-type root resolves to the type, then `new` resolves
    // via by_qualified_name at confidence 1.0.
    let lookup = FakeLookup::default()
        .sym(1, "DbPool", "struct")
        .sym(2, "DbPool.new", "function");
    let r = ref_with_chain(
        vec![
            seg("DbPool", SegmentKind::Identifier, false),
            seg("new", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&RUST_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("DbPool::new() resolves via static-type root");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.confidence, 1.0);
    assert_eq!(res.strategy, "rust_chain_resolution");
}

#[test]
fn rust_chain_members_of_final_fallback() {
    // `svc.handle()` where `svc: Service` and `handle` is a member of `Service`
    // reachable only via members_of (no by_qualified_name hit). The 0.95
    // members_of-final fallback fires (gated by walk_inheritance).
    let lookup = FakeLookup::default()
        .local("svc", "Service")
        .member("Service", 1, "Service.handle", "method");
    let r = ref_with_chain(
        vec![
            seg("svc", SegmentKind::Identifier, false),
            seg("handle", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&RUST_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("svc.handle() resolves via members_of fallback");
    assert_eq!(res.target_symbol_id, 1);
    assert_eq!(res.confidence, 0.95);
}

#[test]
fn rust_chain_trait_inheritance_final_segment() {
    // `r.into_response()` where `r: MyResponse` implements/extends `IntoResponse`
    // (inherits_map → parent_class_qname) and `into_response` is a trait-default
    // method on the parent. walk_inheritance climbs to the trait and binds it.
    let lookup = FakeLookup::default()
        .local("r", "MyResponse")
        .sym(1, "MyResponse", "struct")
        .parent("MyResponse", "IntoResponse")
        .sym(2, "IntoResponse.into_response", "method");
    let r = ref_with_chain(
        vec![
            seg("r", SegmentKind::Identifier, false),
            seg("into_response", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    let res = run_res(&RUST_CHAIN_CONFIG, &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("r.into_response() resolves via trait-inheritance climb");
    assert_eq!(res.target_symbol_id, 2);
    assert_eq!(res.strategy, "rust_chain_inheritance");
}

#[test]
fn rust_chain_inheritance_gated_by_none() {
    // The trait-inheritance fixture under ChainExtensions::NONE: the climb is
    // gated off, so the inherited `into_response` never binds — proves the
    // trait-default coverage is the walk_inheritance flag, not the base walk.
    let lookup = FakeLookup::default()
        .local("r", "MyResponse")
        .sym(1, "MyResponse", "struct")
        .parent("MyResponse", "IntoResponse")
        .sym(2, "IntoResponse.into_response", "method");
    let r = ref_with_chain(
        vec![
            seg("r", SegmentKind::Identifier, false),
            seg("into_response", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&none_config(), &r, &fc, &lookup), None);
}

#[test]
fn rust_chain_empty_chain_returns_none() {
    // A single-segment chain (len < 2) is not a member walk — both the deleted
    // walker and resolve_via_chain return None.
    let lookup = FakeLookup::default().sym(1, "Foo", "struct");
    let r = ref_with_chain(vec![seg("Foo", SegmentKind::Identifier, false)], EdgeKind::Calls);
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup), None);
}

// ---------------------------------------------------------------------------
// Generic type-alias expansion across the languages that carry type aliases
// AND a populated `alias_target` (INFER-10). Each flips `expand_aliases: true`
// on its `<LANG>_CHAIN_CONFIG`; a value typed as an alias name walks to the
// alias's concrete target before member lookup. Scala builds its config inline
// (no exported static) so the test mirrors it with the same flag set.
//
// A no-op guard proves expansion never rewrites a non-alias type: with the
// flag on, a value typed as a class still resolves its member against the
// class, not a guessed target.
// ---------------------------------------------------------------------------

#[test]
fn go_chain_true_alias_via_alias_expansion() {
    // `um.Get()` where `um: UserMap` and `type UserMap = MapImpl` (a Go TRUE
    // alias, the `=` form). It shares MapImpl's members; expansion rewrites
    // `UserMap` → `MapImpl`, where `Get` lives. A Go defined type (`type X Y`,
    // no `=`) would synthesize no AliasTarget and stay a no-op here.
    let lookup = FakeLookup::default()
        .local("um", "UserMap")
        .sym(1, "UserMap", "type_alias")
        .alias(
            "UserMap",
            AliasTarget::Application {
                root: "MapImpl".to_string(),
                args: Vec::new(),
            },
        )
        .sym(2, "MapImpl.Get", "method");
    let r = ref_with_chain(
        vec![
            seg("um", SegmentKind::Identifier, false),
            seg("Get", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&GO_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn rust_chain_type_alias_via_alias_expansion() {
    // `repo.get()` where `repo: Repo` and `type Repo = HashMapStore`. The alias
    // is members-less; expansion walks `Repo` → `HashMapStore` where `get` lives.
    let lookup = FakeLookup::default()
        .local("repo", "Repo")
        .sym(1, "Repo", "type_alias")
        .alias(
            "Repo",
            AliasTarget::Application {
                root: "HashMapStore".to_string(),
                args: Vec::new(),
            },
        )
        .sym(2, "HashMapStore.get", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("get", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

#[test]
fn python_chain_type_alias_via_alias_expansion() {
    // `users.append()` where `users: UserList` and `type UserList = ListImpl`.
    // The alias has no members; expansion walks `UserList` → `ListImpl`.
    let lookup = FakeLookup::default()
        .local("users", "UserList")
        .sym(1, "UserList", "type_alias")
        .alias(
            "UserList",
            AliasTarget::Application {
                root: "ListImpl".to_string(),
                args: Vec::new(),
            },
        )
        .sym(2, "ListImpl.append", "method");
    let r = ref_with_chain(
        vec![
            seg("users", SegmentKind::Identifier, false),
            seg("append", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&PYTHON_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}

/// Scala's `ChainConfig` is built inline in its resolver (no exported static),
/// so mirror it here with `expand_aliases: true` to exercise the same flip.
/// The `kind_compatible` predicate is harness-only (Scala's own is `pub(super)`,
/// like the `none_config` helper) — it does not affect alias expansion.
fn scala_alias_config() -> ChainConfig {
    ChainConfig {
        strategy_prefix: "scala",
        normalize_type: identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "trait", "object"],
        static_type_kinds: &["class", "trait", "object", "enum", "type_alias"],
        use_generics: true,
        namespace_lookup: NamespaceLookup::WildcardOnly,
        kind_compatible: crate::languages::typescript::predicates::kind_compatible,
        extensions: ChainExtensions {
            expand_aliases: true,
            ..ChainExtensions::NONE
        },
    }
}

#[test]
fn scala_chain_type_member_via_alias_expansion() {
    // `assoc.get()` where `assoc: Assoc` and `type Assoc = MapImpl` (a Scala
    // type member). Expansion walks `Assoc` → `MapImpl` where `get` lives.
    let lookup = FakeLookup::default()
        .local("assoc", "Assoc")
        .sym(1, "Assoc", "type_alias")
        .alias(
            "Assoc",
            AliasTarget::Application {
                root: "MapImpl".to_string(),
                args: Vec::new(),
            },
        )
        .sym(2, "MapImpl.get", "method");
    let r = ref_with_chain(
        vec![
            seg("assoc", SegmentKind::Identifier, false),
            seg("get", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&scala_alias_config(), &r, &fc, &lookup), Some(2));
}

#[test]
fn alias_expansion_noop_on_non_alias_type() {
    // With `expand_aliases: true`, a value typed as a plain class (NOT an alias)
    // must NOT be rewritten: `alias_target("User")` is None, so expansion is a
    // no-op and the member resolves against `User` directly. This guards the
    // conservatism invariant — flipping the flag widens alias chains only and
    // never mis-resolves a non-alias receiver to a guessed target.
    let lookup = FakeLookup::default()
        .local("u", "User")
        .sym(1, "User", "struct")
        .sym(2, "User.save", "method");
    let r = ref_with_chain(
        vec![
            seg("u", SegmentKind::Identifier, false),
            seg("save", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    // RUST_CHAIN_CONFIG now carries expand_aliases: true. The non-alias receiver
    // still binds `User.save` (id 2) — the bare member walk, unchanged. No alias
    // entry for "User" means expand_alias returns None and current_type stays.
    assert_eq!(run(&RUST_CHAIN_CONFIG, &r, &fc, &lookup), Some(2));
}
