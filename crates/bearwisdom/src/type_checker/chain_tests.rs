// =============================================================================
// type_checker/chain_tests.rs — sibling tests for the unified chain walker.
//
// These exercise the TypeScript-specific deltas carried in `resolve_via_chain`
// as `ChainExtensions` data: alias expansion, inheritance climbing,
// external-qname promotion, `new X().m()` construction roots, call-root
// inference (import + tsconfig alias + ambient-globals fallback). The
// `none_config()` cases confirm a config with `ChainExtensions::NONE` does NOT
// trip the fallback hops.
// =============================================================================

use super::{
    identity_normalize, resolve_via_chain, ChainConfig, ChainExtensions, NamespaceLookup,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolInfo, SymbolLookup,
};
use crate::languages::c_lang::hooks::C_LANG_CHAIN_CONFIG;
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
    type_args_store: Vec<(String, Vec<String>)>,
    return_type_args_store: Vec<(String, Vec<String>)>,
    generic_params_store: Vec<(String, Vec<String>)>,
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
    fn type_args(mut self, qname: &str, args: &[&str]) -> Self {
        self.type_args_store
            .push((qname.to_string(), args.iter().map(|s| s.to_string()).collect()));
        self
    }
    fn return_type_args(mut self, qname: &str, args: &[&str]) -> Self {
        self.return_type_args_store
            .push((qname.to_string(), args.iter().map(|s| s.to_string()).collect()));
        self
    }
    fn generic_params(mut self, qname: &str, params: &[&str]) -> Self {
        self.generic_params_store
            .push((qname.to_string(), params.iter().map(|s| s.to_string()).collect()));
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
    fn field_type_args(&self, qname: &str) -> Option<&[String]> {
        self.type_args_store
            .iter()
            .find(|(q, _)| q == qname)
            .map(|(_, v)| v.as_slice())
    }
    fn return_type_args(&self, qname: &str) -> Option<&[String]> {
        self.return_type_args_store
            .iter()
            .find(|(q, _)| q == qname)
            .map(|(_, v)| v.as_slice())
    }
    fn generic_params(&self, name: &str) -> Option<&[String]> {
        self.generic_params_store
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
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
// Java differential tests.
//
// Anchor the legacy `resolve_via_chain` ladder driven by the Java-shaped
// `ChainExtensions` (`java_config()`): SelfRef enclosing-type root (implicit
// `this.method()`), static-type root + by_qualified_name final at 1.0,
// field-type root progression, the wildcard-import namespace final hit at
// 0.95 (`NamespaceLookup::WildcardOnly`), and the inheritance climb for an
// inherited member via `walk_inheritance`. A NONE-gate guard proves the
// inheritance climb requires the flag. The legacy `resolve_via_chain` ladder
// these flags drive still backs the languages that ship a `<LANG>_CHAIN_CONFIG`.
// ---------------------------------------------------------------------------

/// Legacy-walker fixture mirroring the Java `ChainExtensions` deltas:
/// `walk_inheritance` + `qualify_via_imports`, `NamespaceLookup::WildcardOnly`.
/// Anchors the generic `resolve_via_chain` ladder still driven by the languages
/// that ship a `<LANG>_CHAIN_CONFIG`.
fn java_config() -> ChainConfig {
    ChainConfig {
        strategy_prefix: "java",
        normalize_type: identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "interface", "enum"],
        static_type_kinds: &["class", "interface", "enum", "type_alias"],
        use_generics: true,
        namespace_lookup: NamespaceLookup::WildcardOnly,
        kind_compatible: crate::languages::typescript::predicates::kind_compatible,
        extensions: ChainExtensions {
            walk_inheritance: true,
            qualify_via_imports: true,
            ..ChainExtensions::NONE
        },
    }
}

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
        &java_config(),
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
    let res = run_res(&java_config(), &r, &fc, vec!["caller".to_string()], &lookup)
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
            &java_config(),
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
    let res = run_res(&java_config(), &r, &fc, vec!["caller".to_string()], &lookup)
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
    let res = run_res(&java_config(), &r, &fc, vec!["caller".to_string()], &lookup)
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

#[test]
fn java_chain_qualifies_bare_receiver_via_import() {
    // `repo.findOne()` where `repo: Repository` and `Repository` is brought in
    // by `import com.fakeext.data.Repository`. The bare receiver type doesn't
    // own `findOne`; qualify_via_imports promotes `Repository` to its imported
    // qname so the package-keyed member binds.
    let lookup = FakeLookup::default()
        .local("repo", "Repository")
        .sym(1, "com.fakeext.data.Repository", "class")
        .sym(2, "com.fakeext.data.Repository.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![import("Repository", "com.fakeext.data.Repository")]);
    let res = run_res(&java_config(), &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("import-qualified receiver resolves findOne");
    assert_eq!(res.target_symbol_id, 2);
}

#[test]
fn java_chain_qualifies_return_type_same_package() {
    // `repo.findOne().getEmail()` where `findOne` returns the bare `Entity`,
    // which is never imported (only a return type). The same-package
    // qualification promotes `Entity` to `com.fakeext.data.Entity` using the
    // receiver's package, so the second hop `getEmail` binds.
    let lookup = FakeLookup::default()
        .local("repo", "Repository")
        .sym(1, "com.fakeext.data.Repository", "class")
        .sym(2, "com.fakeext.data.Repository.findOne", "method")
        .ret("com.fakeext.data.Repository.findOne", "Entity")
        .sym(3, "com.fakeext.data.Entity", "class")
        .sym(4, "com.fakeext.data.Entity.getEmail", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
            seg("getEmail", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![import("Repository", "com.fakeext.data.Repository")]);
    let res = run_res(&java_config(), &r, &fc, vec!["caller".to_string()], &lookup)
        .expect("same-package return type qualifies so getEmail binds");
    assert_eq!(res.target_symbol_id, 4);
}

#[test]
fn java_chain_qualify_via_imports_gated_by_none() {
    // The import-qualification fixture under ChainExtensions::NONE: with the
    // flag off, the bare `Repository` receiver never promotes, so `findOne`
    // keyed under the package qname stays unreachable.
    let lookup = FakeLookup::default()
        .local("repo", "Repository")
        .sym(1, "com.fakeext.data.Repository", "class")
        .sym(2, "com.fakeext.data.Repository.findOne", "method");
    let r = ref_with_chain(
        vec![
            seg("repo", SegmentKind::Identifier, false),
            seg("findOne", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![import("Repository", "com.fakeext.data.Repository")]);
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

// ---------------------------------------------------------------------------
// Generic method-return arg binding: CODEGEN-1
// ---------------------------------------------------------------------------

/// `obj.getItems().get(0)` where `getItems()` returns `List<User>`.
///
/// Symbols:
///   * `Repo.getItems` — return_type="List", type_args=["User"]
///   * `List` — class with generic_params=["E"]
///   * `List.get` — return_type="E"
///   * `User.someMethod` — the final target (id=4)
///
/// Without arg binding `E` stays unbound, and the chain can't find
/// `User.someMethod`.  With binding `E→User`, `List.get()` yields `User`
/// and the next hop resolves.
#[test]
fn method_return_generic_arg_binds_element_type() {
    let lookup = FakeLookup::default()
        .local("obj", "Repo")
        .sym(1, "Repo", "class")
        .sym(2, "Repo.getItems", "method")
        .ret("Repo.getItems", "List")
        .return_type_args("Repo.getItems", &["User"])
        .sym(3, "List", "class")
        .generic_params("List", &["E"])
        .sym(4, "List.get", "method")
        .ret("List.get", "E")
        .sym(5, "User", "class")
        .sym(6, "User.someMethod", "method");

    let r = ref_with_chain(
        vec![
            seg("obj", SegmentKind::Identifier, false),
            seg("getItems", SegmentKind::Property, true),
            seg("get", SegmentKind::Property, true),
            seg("someMethod", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    assert_eq!(run(&java_config(), &r, &fc, &lookup), Some(6));
}

/// Control: without type_args registered for the method, the element type
/// parameter stays unbound and the chain yields the unresolved type variable.
/// The call to `User.someMethod` is NOT resolved.
#[test]
fn method_return_without_args_does_not_bind_element_type() {
    let lookup = FakeLookup::default()
        .local("obj", "Repo")
        .sym(1, "Repo", "class")
        .sym(2, "Repo.getItems", "method")
        .ret("Repo.getItems", "List")
        // No return_type_args for Repo.getItems → E stays unbound.
        .sym(3, "List", "class")
        .generic_params("List", &["E"])
        .sym(4, "List.get", "method")
        .ret("List.get", "E")
        .sym(5, "User", "class")
        .sym(6, "User.someMethod", "method");

    let r = ref_with_chain(
        vec![
            seg("obj", SegmentKind::Identifier, false),
            seg("getItems", SegmentKind::Property, true),
            seg("get", SegmentKind::Property, true),
            seg("someMethod", SegmentKind::Property, true),
        ],
        EdgeKind::Calls,
    );
    let fc = file_ctx_with_imports(vec![]);
    // Without the arg, E stays as "E" which has no members, so resolution fails.
    assert_eq!(run(&java_config(), &r, &fc, &lookup), None);
}
