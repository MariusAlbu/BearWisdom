use super::*;
use crate::indexer::resolve::engine::contract::{FileContext, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{accept_any, ref_ctx, source_symbol, Lookup};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::ExtractedRef;

/// A lookup whose include closure places exactly one spelling.
struct Includes {
    inner: Lookup,
    placed: &'static str,
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for Includes {}

impl crate::indexer::resolve::engine::contract::IncludeLookup for Includes {
    fn include_spec_resolves(&self, source_file: &str, spec: &str) -> bool {
        source_file == "lib/url.c" && spec == self.placed
    }
}

impl SymbolLookup for Includes {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        self.inner.by_name(name)
    }
    fn by_qualified_name(&self, name: &str) -> Option<&Symbol> {
        self.inner.by_qualified_name(name)
    }
    fn members_of(&self, name: &str) -> SymbolSet<'_> {
        self.inner.members_of(name)
    }
    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        self.inner.types_by_name(name)
    }
    fn in_namespace(&self, name: &str) -> Vec<&Symbol> {
        self.inner.in_namespace(name)
    }
    fn has_in_namespace(&self, name: &str) -> bool {
        self.inner.has_in_namespace(name)
    }
    fn in_file(&self, file: &str) -> SymbolSet<'_> {
        self.inner.in_file(file)
    }
    fn field_type_name(&self, name: &str) -> Option<&str> {
        self.inner.field_type_name(name)
    }
    fn return_type_name(&self, name: &str) -> Option<&str> {
        self.inner.return_type_name(name)
    }
    fn generic_params(&self, name: &str) -> Option<Vec<String>> {
        self.inner.generic_params(name)
    }
    fn reexports_from(&self, file: &str) -> &[(String, String)] {
        self.inner.reexports_from(file)
    }
    fn is_external_name(&self, name: &str, language: &str) -> bool {
        self.inner.is_external_name(name, language)
    }
}

fn include(spec: &str, is_include: bool) -> ExtractedRef {
    ExtractedRef {
        is_include,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: spec.rsplit('/').next().unwrap().to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: Some(spec.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn apply(reference: &ExtractedRef, placed: &'static str) -> LookupResult {
    let lookup = Includes {
        inner: Lookup::new(),
        placed,
    };
    let source = source_symbol("main");
    let fc = FileContext {
        file_path: "lib/url.c".to_string(),
        language: "c".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let rc = ref_ctx(reference, &source, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    IncludeFileRule.apply(&ctx)
}

#[test]
fn placed_include_is_drained_at_the_file_level() {
    assert!(matches!(
        apply(&include("curl/curl.h", true), "curl/curl.h"),
        LookupResult::Drained
    ));
}

#[test]
fn unplaced_include_stays_on_the_ladder() {
    assert!(matches!(
        apply(&include("stdio.h", true), "curl/curl.h"),
        LookupResult::Pass
    ));
}

#[test]
fn ordinary_imports_are_not_includes() {
    assert!(matches!(
        apply(&include("curl/curl.h", false), "curl/curl.h"),
        LookupResult::Pass
    ));
}
