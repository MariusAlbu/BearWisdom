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
use crate::languages::csharp::hooks::CSHARP_CHAIN_CONFIG;
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
    ExtractedRef {
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
