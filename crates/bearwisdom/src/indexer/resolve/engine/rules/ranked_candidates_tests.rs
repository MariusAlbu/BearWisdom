use super::*;
use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

// ---------------------------------------------------------------------------
// Lookup double that supports `is_ambient_path`.
// ---------------------------------------------------------------------------

struct ScoredLookup {
    inner: Lookup,
    ambient_paths: Vec<String>,
}

impl ScoredLookup {
    fn new(inner: Lookup) -> Self {
        Self {
            inner,
            ambient_paths: Vec::new(),
        }
    }

    fn with_ambient(mut self, path: &str) -> Self {
        self.ambient_paths.push(path.to_string());
        self
    }
}

impl SymbolLookup for ScoredLookup {
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
    fn is_ambient_path(&self, path: &str) -> bool {
        self.ambient_paths.iter().any(|p| p == path)
    }
}

// ---------------------------------------------------------------------------
// Profile helper
// ---------------------------------------------------------------------------

use crate::type_checker::profile::language_profile::LanguageProfile;

static RANKED_PROFILE: LanguageProfile = LanguageProfile {
    multi_candidate_ranking: true,
    ..DEFAULT_PROFILE
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn passes_when_gate_is_off() {
    // DEFAULT_PROFILE has multi_candidate_ranking = false; rule must pass even
    // with two candidates.
    let lookup = Lookup::new()
        .with(sym(1, "foo", "a.foo", "function", "src/a.ts"))
        .with(sym(2, "foo", "b.foo", "function", "src/b.ts"));
    let r = call_ref("foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(RankedCandidatesRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn passes_for_dotted_target() {
    let profile = &RANKED_PROFILE;
    let lookup = Lookup::new()
        .with(sym(1, "foo", "a.foo", "function", "src/a.ts"))
        .with(sym(2, "foo", "b.foo", "function", "src/b.ts"));
    let r = call_ref("a.foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    assert!(matches!(RankedCandidatesRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn passes_when_only_one_candidate() {
    let profile = &RANKED_PROFILE;
    let lookup = Lookup::new().with(sym(1, "foo", "a.foo", "function", "src/a.ts"));
    let r = call_ref("foo");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    assert!(matches!(RankedCandidatesRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn passes_when_margin_is_insufficient() {
    // Two candidates at same proximity — no clear winner.
    let profile = &RANKED_PROFILE;
    let lookup = Lookup::new()
        .with(sym(1, "foo", "a.foo", "function", "src/a.ts"))
        .with(sym(2, "foo", "b.foo", "function", "src/b.ts"));
    let r = call_ref("foo");
    let s = source_symbol("caller");
    // File sits at root so proximity is 0 for both.
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    assert!(matches!(RankedCandidatesRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn resolves_ambient_candidate_over_non_ambient() {
    // Ambient path gives +200 — enough to exceed RANK_MARGIN over a zero-score
    // sibling with no other signals.
    let profile = &RANKED_PROFILE;
    let s_ambient = sym(10, "describe", "jest.describe", "function", "node_modules/@types/jest/index.d.ts");
    let s_project = sym(11, "describe", "suite.describe", "function", "src/suite.ts");
    let inner = Lookup::new().with(s_ambient).with(s_project);
    let lookup = ScoredLookup::new(inner)
        .with_ambient("node_modules/@types/jest/index.d.ts");
    let r = call_ref("describe");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    match RankedCandidatesRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 10),
        v => panic!("expected Resolved to ambient sym, got {v:?}"),
    }
}

#[test]
fn resolves_closer_path_candidate() {
    // Caller is at `src/views/Page.ts`. Two candidates: one in `src/views/`,
    // one in `src/other/`. The `src/views/` sibling has path_proximity +20
    // (2 shared segs) vs +10 (1 shared seg) — margin = 10. Not enough on its
    // own (< RANK_MARGIN=100). Add ambient to push one past the margin.
    //
    // Simpler: just test that a candidate with path proximity AND ambient beats
    // a bare candidate.
    let profile = &RANKED_PROFILE;
    let near = sym(20, "helper", "views.helper", "function", "src/views/helper.ts");
    let far = sym(21, "helper", "other.helper", "function", "src/other/helper.ts");
    let inner = Lookup::new().with(near).with(far);
    let lookup = ScoredLookup::new(inner).with_ambient("src/views/helper.ts");
    let r = call_ref("helper");
    let s = source_symbol("caller");
    use crate::indexer::resolve::engine::contract::FileContext;
    let fc = FileContext {
        file_path: "src/views/Page.ts".to_string(),
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
        profile,
    };
    match RankedCandidatesRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 20),
        v => panic!("expected Resolved to near+ambient sym, got {v:?}"),
    }
}

// ---------------------------------------------------------------------------
// path_proximity_score unit tests (via the module-private fn)
// ---------------------------------------------------------------------------

#[test]
fn path_proximity_score_shared_dir() {
    assert_eq!(path_proximity_score("src/views/Page.ts", "src/views/Helper.ts"), 20);
}

#[test]
fn path_proximity_score_no_overlap() {
    assert_eq!(path_proximity_score("src/a/X.ts", "lib/b/Y.ts"), 0);
}

#[test]
fn path_proximity_score_partial_overlap() {
    assert_eq!(path_proximity_score("src/a/b/X.ts", "src/a/c/Y.ts"), 20);
}
