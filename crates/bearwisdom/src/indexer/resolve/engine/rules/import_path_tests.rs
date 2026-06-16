use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::indexer::resolve::engine::contract::{FileContext, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{accept_any, ref_ctx, source_symbol};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    CandidateDirs, DEFAULT_PROFILE, ImportResolution, LanguageProfile, StemMatch,
};
use crate::types::{EdgeKind, ExtractedRef};

// ---------------------------------------------------------------------------
// Minimal SymbolLookup that serves `in_file` queries.
// ---------------------------------------------------------------------------

struct FileLookup {
    by_file: HashMap<String, Vec<Symbol>>,
}

impl FileLookup {
    fn new() -> Self {
        Self {
            by_file: HashMap::new(),
        }
    }

    fn with_file_sym(
        mut self,
        file_path: &str,
        id: i64,
        name: &'static str,
        kind: &'static str,
    ) -> Self {
        self.by_file
            .entry(file_path.to_string())
            .or_default()
            .push(Symbol {
                id,
                name: name.to_string(),
                qualified_name: name.to_string(),
                kind: kind.to_string(),
                visibility: None,
                file_path: Arc::from(file_path),
                scope_path: None,
                package_id: None,
                signature: None,
            });
        self
    }
}

impl SymbolLookup for FileLookup {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::empty()
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file<'a>(&'a self, path: &str) -> SymbolSet<'a> {
        match self.by_file.get(path) {
            Some(v) => SymbolSet::Borrowed(v.as_slice()),
            None => SymbolSet::empty(),
        }
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
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &[]
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// A minimal ImportResolution spec for handlebars-style partial includes.
// ---------------------------------------------------------------------------

const HBS_IR: ImportResolution = ImportResolution {
    extensions: &["hbs"],
    candidate_dirs: CandidateDirs::SelfDir,
    index_files: &[],
    underscore_variant: true,
    kebab_variant: false,
    decline_leading_slash: false,
    stem_match: StemMatch::StemExact,
    bind_kind: "template",
    strategy_tag: "default_import_path",
};

static HBS_PROFILE: LanguageProfile = LanguageProfile {
    import_resolution: Some(HBS_IR),
    ..DEFAULT_PROFILE
};

fn imports_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn resolve(lookup: &FileLookup, source_file: &str, target: &str) -> Option<i64> {
    let r = imports_ref(target);
    let s = source_symbol("template");
    let fc = FileContext {
        file_path: source_file.to_string(),
        language: "handlebars".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &HBS_PROFILE,
    };
    match ImportPathRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_sibling_template_by_stem() {
    // `src/views/page.hbs` includes `header` — resolved to `src/views/header.hbs`.
    let lookup = FileLookup::new()
        .with_file_sym("src/views/header.hbs", 3, "header", "template");
    assert_eq!(resolve(&lookup, "src/views/page.hbs", "header"), Some(3));
}

#[test]
fn declines_non_imports_edge_kind() {
    // The rule must pass on any non-Imports edge even when a profile is set.
    let lookup = FileLookup::new()
        .with_file_sym("src/views/header.hbs", 3, "header", "template");
    let r = crate::indexer::resolve::engine::testkit::call_ref("header");
    let s = source_symbol("template");
    let fc = FileContext {
        file_path: "src/views/page.hbs".to_string(),
        language: "handlebars".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &HBS_PROFILE,
    };
    assert!(matches!(ImportPathRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn declines_when_no_import_resolution_configured() {
    // DEFAULT_PROFILE has `import_resolution: None` — the rule is inert.
    let lookup = FileLookup::new()
        .with_file_sym("src/views/header.hbs", 3, "header", "template");
    let r = imports_ref("header");
    let s = source_symbol("template");
    let fc = FileContext {
        file_path: "src/views/page.hbs".to_string(),
        language: "handlebars".to_string(),
        imports: Vec::new(),
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
    assert!(matches!(ImportPathRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn binds_underscore_variant() {
    // With `underscore_variant: true`, `header` also tries `_header.hbs`.
    // StemExact checks `sym.name == file_stem`; for `_header.hbs` the stem is
    // `_header`, so the symbol must be named `_header` to match.
    let lookup = FileLookup::new()
        .with_file_sym("src/views/_header.hbs", 7, "_header", "template");
    assert_eq!(resolve(&lookup, "src/views/page.hbs", "header"), Some(7));
}
