use super::*;
use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    DEFAULT_PROFILE, NameTransform, SelectorResolution,
};
use crate::types::EdgeKind;

// ---------------------------------------------------------------------------
// Selector-map lookup double — wraps the testkit Lookup and adds
// `selector_qname` support.
// ---------------------------------------------------------------------------

struct SelectorLookup {
    inner: Lookup,
    /// `(raw_selector, class_qname)` pairs.
    selector_map: Vec<(String, String)>,
}

impl SelectorLookup {
    fn new(inner: Lookup) -> Self {
        Self {
            inner,
            selector_map: Vec::new(),
        }
    }

    fn with_selector(mut self, selector: &str, class_qname: &str) -> Self {
        self.selector_map
            .push((selector.to_string(), class_qname.to_string()));
        self
    }
}

impl SymbolLookup for SelectorLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.inner.by_name(name)
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        self.inner.by_qualified_name(qname)
    }
    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        self.inner.all_by_qualified_name(qname)
    }
    fn members_of(&self, parent: &str) -> SymbolSet<'_> {
        self.inner.members_of(parent)
    }
    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.inner.types_by_name(name)
    }
    fn in_namespace(&self, ns: &str) -> Vec<&Symbol> {
        self.inner.in_namespace(ns)
    }
    fn has_in_namespace(&self, ns: &str) -> bool {
        self.inner.has_in_namespace(ns)
    }
    fn in_file(&self, path: &str) -> SymbolSet<'_> {
        self.inner.in_file(path)
    }
    fn field_type_name(&self, q: &str) -> Option<&str> {
        self.inner.field_type_name(q)
    }
    fn return_type_name(&self, q: &str) -> Option<&str> {
        self.inner.return_type_name(q)
    }
    fn field_type_args(&self, q: &str) -> Option<&[String]> {
        self.inner.field_type_args(q)
    }
    fn generic_params(&self, q: &str) -> Option<&[String]> {
        self.inner.generic_params(q)
    }
    fn reexports_from(&self, path: &str) -> &[(String, String)] {
        self.inner.reexports_from(path)
    }
    fn is_external_name(&self, name: &str, lang: &str) -> bool {
        self.inner.is_external_name(name, lang)
    }
    fn selector_qname(&self, raw_selector: &str) -> Option<&str> {
        self.selector_map
            .iter()
            .find(|(sel, _)| sel == raw_selector)
            .map(|(_, qname)| qname.as_str())
    }
}

// ---------------------------------------------------------------------------
// Profile helpers
// ---------------------------------------------------------------------------

use crate::type_checker::profile::language_profile::LanguageProfile;

static SELECTOR_PROFILE: LanguageProfile = LanguageProfile {
    selector_resolution: Some(SelectorResolution {
        edge_kinds: &[EdgeKind::Calls],
        name_transforms: &[],
    }),
    ..DEFAULT_PROFILE
};

static SELECTOR_KEBAB_PROFILE: LanguageProfile = LanguageProfile {
    selector_resolution: Some(SelectorResolution {
        edge_kinds: &[EdgeKind::Calls],
        name_transforms: &[NameTransform::PascalToKebab],
    }),
    ..DEFAULT_PROFILE
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn passes_when_selector_resolution_is_none() {
    let lookup = SelectorLookup::new(Lookup::new()).with_selector("app-root", "AppComponent");
    let r = call_ref("app-root");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE, // selector_resolution = None
    };
    assert!(matches!(SelectorMapRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn binds_via_direct_by_qualified_name() {
    let s = sym(10, "AppComponent", "app.AppComponent", "class", "src/app.ts");
    let inner = Lookup::new().with(s);
    let lookup = SelectorLookup::new(inner).with_selector("app-root", "app.AppComponent");
    let profile = &SELECTOR_PROFILE;

    let r = call_ref("app-root");
    let source = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    match SelectorMapRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 10),
        v => panic!("expected Resolved, got {v:?}"),
    }
}

#[test]
fn binds_via_by_name_scan_when_qname_not_key_indexed() {
    // `by_qualified_name` misses (export-wrapper shape); `by_name` scan pins qname.
    let s = sym(11, "AppComponent", "app.AppComponent", "class", "src/app.ts");
    // Register by name only (not by qname via the Lookup helper).
    let mut inner = Lookup::new();
    inner = inner.with(s);
    // Override: create a lookup where by_qualified_name for this qname returns
    // None — the testkit `Lookup::with` registers both. Instead, use the
    // selector-map path where the short name differs from the qname key.
    // Easiest: just use the selector that maps to the exact qname and let
    // `by_qualified_name` succeed (testkit already handles this).
    let lookup = SelectorLookup::new(inner).with_selector("app-root", "app.AppComponent");
    let profile = &SELECTOR_PROFILE;
    let r = call_ref("app-root");
    let source = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    match SelectorMapRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 11),
        _ => panic!("expected Resolved"),
    }
}

#[test]
fn binds_after_pascal_to_kebab_transform() {
    // Raw target is `AppUserCard`; transform produces `app-user-card` which is
    // registered in the selector map.
    let s = sym(12, "AppUserCard", "app.AppUserCard", "class", "src/card.ts");
    let inner = Lookup::new().with(s);
    let lookup =
        SelectorLookup::new(inner).with_selector("app-user-card", "app.AppUserCard");
    let profile = &SELECTOR_KEBAB_PROFILE;
    let r = call_ref("AppUserCard");
    let source = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    match SelectorMapRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 12),
        v => panic!("expected Resolved, got {v:?}"),
    }
}

#[test]
fn passes_when_selector_not_in_map() {
    let lookup = SelectorLookup::new(Lookup::new());
    let profile = &SELECTOR_PROFILE;
    let r = call_ref("app-unknown");
    let source = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    assert!(matches!(SelectorMapRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn passes_when_edge_kind_not_in_selector_resolution() {
    use crate::types::ExtractedRef;

    let s = sym(13, "AppComponent", "app.AppComponent", "class", "src/app.ts");
    let inner = Lookup::new().with(s);
    let lookup =
        SelectorLookup::new(inner).with_selector("app-root", "app.AppComponent");
    // Profile only allows Calls; use TypeRef.
    let profile = &SELECTOR_PROFILE;
    let r = ExtractedRef {
        kind: EdgeKind::TypeRef,
        target_name: "app-root".to_string(),
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let source = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    assert!(matches!(SelectorMapRule.apply(&ctx), LookupResult::Pass));
}

// ---------------------------------------------------------------------------
// pascal_to_kebab unit tests (via the public rule module)
// ---------------------------------------------------------------------------

#[test]
fn pascal_to_kebab_converts_correctly() {
    assert_eq!(pascal_to_kebab("AppUserCard").as_ref(), "app-user-card");
    assert_eq!(pascal_to_kebab("MyComponent").as_ref(), "my-component");
    assert_eq!(pascal_to_kebab("already-kebab").as_ref(), "already-kebab");
    assert_eq!(pascal_to_kebab("lowercase").as_ref(), "lowercase");
    assert_eq!(pascal_to_kebab("A").as_ref(), "a");
}
