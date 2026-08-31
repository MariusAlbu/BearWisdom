use crate::indexer::resolve::engine::contract::{
    FileContext, Symbol, SymbolLookup, SymbolSet,
};
use crate::indexer::resolve::engine::testkit::{accept_any, ref_ctx, source_symbol, sym};
use crate::indexer::resolve::engine::rules::same_file::SameFileRule;
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{EdgeKind, ExtractedRef};

// ---------------------------------------------------------------------------
// Minimal SymbolLookup double that supports in_file — the testkit Lookup
// always returns empty from in_file.
// ---------------------------------------------------------------------------

struct FileLookup {
    by_file: std::collections::HashMap<String, Vec<Symbol>>,
    by_name: std::collections::HashMap<String, Vec<Symbol>>,
    empty: Vec<Symbol>,
    empty_pairs: Vec<(String, String)>,
}

impl FileLookup {
    fn new() -> Self {
        Self {
            by_file: Default::default(),
            by_name: Default::default(),
            empty: Vec::new(),
            empty_pairs: Vec::new(),
        }
    }

    fn with(mut self, s: Symbol) -> Self {
        self.by_name.entry(s.name.clone()).or_default().push(s.clone());
        self.by_file.entry(s.file_path.to_string()).or_default().push(s);
        self
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for FileLookup {}

impl SymbolLookup for FileLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]))
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> { None }
    fn members_of(&self, _: &str) -> SymbolSet<'_> { SymbolSet::Borrowed(&self.empty) }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> { SymbolSet::Borrowed(&self.empty) }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> { Vec::new() }
    fn has_in_namespace(&self, _: &str) -> bool { false }
    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_file.get(file_path).map(|v| v.as_slice()).unwrap_or(&[]),
        )
    }
    fn field_type_name(&self, _: &str) -> Option<&str> { None }
    fn return_type_name(&self, _: &str) -> Option<&str> { None }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> { None }
    fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_pairs }
    fn is_external_name(&self, _: &str, _: &str) -> bool { false }
}

// ---------------------------------------------------------------------------
// Helper: build a type-position ExtractedRef for `target` with `kind`.
// ---------------------------------------------------------------------------

fn type_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 1,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// BinderContext::target() unit tests
// ---------------------------------------------------------------------------

/// For type-position edge kinds, target() strips the generic application to
/// the bare head.
#[test]
fn target_strips_generic_args_for_type_position_kinds() {
    let lookup = FileLookup::new();
    let fc = FileContext {
        file_path: "src/types.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let s = source_symbol("QueryObserverPendingResult");
    let kind = accept_any;

    for edge_kind in [
        EdgeKind::Inherits,
        EdgeKind::Implements,
        EdgeKind::TypeRef,
        EdgeKind::Instantiates,
    ] {
        let r = type_ref("QueryObserverBaseResult<TData, TError>", edge_kind);
        let rc = ref_ctx(&r, &s, vec![]);
        let ctx = BinderContext {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind: &kind,
            profile: &DEFAULT_PROFILE,
        };
        assert_eq!(
            ctx.target(),
            "QueryObserverBaseResult",
            "edge_kind {edge_kind:?}: expected bare head, got full application string"
        );
    }
}

/// For non-type-position edge kinds (e.g. Calls), target() returns the raw
/// target_name unchanged.
#[test]
fn target_unchanged_for_non_type_position_kinds() {
    let lookup = FileLookup::new();
    let fc = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let s = source_symbol("caller");
    let kind = accept_any;

    for edge_kind in [EdgeKind::Calls, EdgeKind::Reads, EdgeKind::Writes] {
        let raw = "someFunction<T>";
        let r = type_ref(raw, edge_kind);
        let rc = ref_ctx(&r, &s, vec![]);
        let ctx = BinderContext {
            file_ctx: &fc,
            ref_ctx: &rc,
            lookup: &lookup,
            kind: &kind,
            profile: &DEFAULT_PROFILE,
        };
        assert_eq!(
            ctx.target(),
            raw,
            "edge_kind {edge_kind:?}: non-type-position target must be returned unchanged"
        );
    }
}

/// Stripping is a no-op when the target carries no generic args.
#[test]
fn target_bare_name_unchanged_for_type_position_kinds() {
    let lookup = FileLookup::new();
    let fc = FileContext {
        file_path: "src/types.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let s = source_symbol("QueryCache");
    let kind = accept_any;
    let r = type_ref("Subscribable", EdgeKind::Inherits);
    let rc = ref_ctx(&r, &s, vec![]);
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert_eq!(ctx.target(), "Subscribable");
}

// ---------------------------------------------------------------------------
// End-to-end: SameFileRule binds an Inherits target carrying generic args.
// ---------------------------------------------------------------------------

/// An `extends QueryObserverBaseResult<TData, TError>` ref in the same file
/// as the base interface — SameFileRule resolves it by comparing the stripped
/// head against the indexed symbol's name.
#[test]
fn same_file_rule_binds_inherits_with_generic_application_target() {
    let lookup = FileLookup::new().with(sym(
        624,
        "QueryObserverBaseResult",
        "QueryObserverBaseResult",
        "interface",
        "src/types.ts",
    ));
    let r = type_ref("QueryObserverBaseResult<TData, TError>", EdgeKind::Inherits);
    let s = source_symbol("QueryObserverPendingResult");
    let fc = FileContext {
        file_path: "src/types.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match SameFileRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 624),
        other => panic!(
            "expected Resolved(624), got {other:?} — \
             SameFileRule must match using the stripped head"
        ),
    }
}

/// Guard: a bare (non-generic) Inherits target still binds through SameFileRule
/// without regression.
#[test]
fn same_file_rule_binds_non_generic_inherits_target_unchanged() {
    let lookup = FileLookup::new().with(sym(
        700,
        "Subscribable",
        "Subscribable",
        "class",
        "src/types.ts",
    ));
    let r = type_ref("Subscribable", EdgeKind::Inherits);
    let s = source_symbol("QueryCache");
    let fc = FileContext {
        file_path: "src/types.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match SameFileRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 700),
        other => panic!("non-generic target must bind unchanged, got {other:?}"),
    }
}
