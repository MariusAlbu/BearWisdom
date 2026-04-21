// =============================================================================
// ecosystem/js_test_chains.rs — Synthetic chain-type entries for JS test frameworks
//
// Chai and Vitest fluent assertion APIs expose deeply chained method/property
// calls (`expect(x).to.be.equal(y)`, `vi.spyOn(...).toHaveBeenCalledOnce()`)
// that the chain walker can't follow because the methods' return types are not
// populated in TypeInfo after normal parsing.
//
// Root cause: chai's `@types/chai/index.d.ts` declares everything inside
// `declare global { namespace Chai { ... } }`. Tree-sitter extracts the symbols
// (Chai.Assertion, Chai.LanguageChains, …) but the TypeRef edges for the
// property types within that nested namespace are emitted against simple names
// (`Assertion`, not `chai.Chai.Assertion`), so the TypeInfo builder can't
// correlate them post-prefix. Vitest's MockInstance chain has a similar gap.
//
// Fix: inject one synthetic ParsedFile per active framework. Each file carries
// the minimal set of ExtractedSymbol + ExtractedRef entries the TypeInfo
// builder needs to populate field_type and return_type for the critical chain
// hops. The file paths start with `ext:ts:` so `external_type_qname` can
// resolve short names like "Assertion" to "chai.Assertion".
//
// Called from NpmEcosystem::parse_metadata_only — fires once per indexing run
// when a chai or vitest dep root exists under node_modules.
// =============================================================================

use std::path::Path;

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Returns synthetic ParsedFile entries for every test framework whose
/// `node_modules` directory is present under `project_root`. Returns an empty
/// vec when neither chai nor vitest is installed.
pub fn synthetic_test_chain_files(project_root: &Path) -> Vec<ParsedFile> {
    // Collect candidate node_modules directories using the same probe order
    // as the npm externals locator: BEARWISDOM_TS_NODE_MODULES override first,
    // then project_root/node_modules as fallback.
    let mut nm_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            if seg.is_dir() { nm_dirs.push(seg) }
        }
    }
    if nm_dirs.is_empty() {
        let local = project_root.join("node_modules");
        if local.is_dir() { nm_dirs.push(local) }
    }
    if nm_dirs.is_empty() {
        return Vec::new();
    }

    let chai_present = nm_dirs.iter().any(|nm| {
        nm.join("@types").join("chai").is_dir() || nm.join("chai").join("index.d.ts").exists()
    });
    let vitest_present = nm_dirs.iter().any(|nm| {
        nm.join("vitest").is_dir() || nm.join("@vitest").join("expect").is_dir()
    });

    let mut out = Vec::new();
    if chai_present { out.push(chai_synthetic()); }
    if vitest_present { out.push(vitest_synthetic()); }
    out
}

// ---------------------------------------------------------------------------
// Chai synthetic
// ---------------------------------------------------------------------------

/// Build a synthetic ParsedFile that gives the chain walker everything it needs
/// to follow `expect(x).to.be.equal(y)` and variants.
///
/// Symbol layout (all qualified under the `chai.` package prefix):
///
///   chai.Assertion           — interface (enables external_type_qname lookup)
///   chai.Assertion.to        — property → field_type = "chai.Assertion"
///   chai.Assertion.be        — property → field_type = "chai.Assertion"
///   chai.Assertion.been      — property → field_type = "chai.Assertion"
///   chai.Assertion.is        — property → field_type = "chai.Assertion"
///   chai.Assertion.that      — property → field_type = "chai.Assertion"
///   chai.Assertion.which     — property → field_type = "chai.Assertion"
///   chai.Assertion.and       — property → field_type = "chai.Assertion"
///   chai.Assertion.has       — property → field_type = "chai.Assertion"
///   chai.Assertion.have      — property → field_type = "chai.Assertion"
///   chai.Assertion.with      — property → field_type = "chai.Assertion"
///   chai.Assertion.at        — property → field_type = "chai.Assertion"
///   chai.Assertion.of        — property → field_type = "chai.Assertion"
///   chai.Assertion.same      — property → field_type = "chai.Assertion"
///   chai.Assertion.but       — property → field_type = "chai.Assertion"
///   chai.Assertion.does      — property → field_type = "chai.Assertion"
///   chai.Assertion.not       — property → field_type = "chai.Assertion"
///   chai.Assertion.deep      — property → field_type = "chai.Assertion"
///   chai.Assertion.nested    — property → field_type = "chai.Assertion"
///   chai.Assertion.own       — property → field_type = "chai.Assertion"
///   chai.Assertion.ordered   — property → field_type = "chai.Assertion"
///   chai.Assertion.any       — property → field_type = "chai.Assertion"
///   chai.Assertion.all       — property → field_type = "chai.Assertion"
///   chai.Assertion.ok        — property → field_type = "chai.Assertion"
///   chai.Assertion.true      — property → field_type = "chai.Assertion"
///   chai.Assertion.false     — property → field_type = "chai.Assertion"
///   chai.Assertion.null      — property → field_type = "chai.Assertion"
///   chai.Assertion.undefined — property → field_type = "chai.Assertion"
///   chai.Assertion.exist     — property → field_type = "chai.Assertion"
///   chai.Assertion.empty     — property → field_type = "chai.Assertion"
///   chai.Assertion.NaN       — property → field_type = "chai.Assertion"
///   chai.Assertion.finite    — property → field_type = "chai.Assertion"
///   chai.Assertion.a         — property → field_type = "chai.Assertion"
///   chai.Assertion.an        — property → field_type = "chai.Assertion"
///   chai.Assertion.equal     — method   → return_type = "chai.Assertion"
///   chai.Assertion.equals    — method   → return_type = "chai.Assertion"
///   chai.Assertion.eq        — method   → return_type = "chai.Assertion"
///   chai.Assertion.eql       — method   → return_type = "chai.Assertion"
///   chai.Assertion.eqls      — method   → return_type = "chai.Assertion"
///   chai.Assertion.include   — method   → return_type = "chai.Assertion"
///   chai.Assertion.includes  — method   → return_type = "chai.Assertion"
///   chai.Assertion.contain   — method   → return_type = "chai.Assertion"
///   chai.Assertion.contains  — method   → return_type = "chai.Assertion"
///   chai.Assertion.throw     — method   → return_type = "chai.Assertion"
///   chai.Assertion.throws    — method   → return_type = "chai.Assertion"
///   chai.Assertion.above     — method   → return_type = "chai.Assertion"
///   chai.Assertion.below     — method   → return_type = "chai.Assertion"
///   chai.Assertion.least     — method   → return_type = "chai.Assertion"
///   chai.Assertion.most      — method   → return_type = "chai.Assertion"
///   chai.Assertion.within    — method   → return_type = "chai.Assertion"
///   chai.Assertion.property  — method   → return_type = "chai.Assertion"
///   chai.Assertion.keys      — method   → return_type = "chai.Assertion"
///   chai.Assertion.members   — method   → return_type = "chai.Assertion"
///   chai.Assertion.satisfy   — method   → return_type = "chai.Assertion"
///   chai.Assertion.match     — method   → return_type = "chai.Assertion"
///   chai.Assertion.matches   — method   → return_type = "chai.Assertion"
///   chai.Assertion.length    — method   → return_type = "chai.Assertion"
///   chai.Assertion.lengthOf  — method   → return_type = "chai.Assertion"
///   chai.Assertion.string    — method   → return_type = "chai.Assertion"
///   chai.Assertion.itself    — property → field_type = "chai.Assertion"
///
///   chai.ExpectStatic        — interface
///   chai.expect              — function → return_type = "chai.Assertion"
fn chai_synthetic() -> ParsedFile {
    const PKG: &str = "chai";
    const ASSERTION: &str = "chai.Assertion";
    const PATH: &str = "ext:ts:chai/__bw_synthetic__.d.ts";

    // Chaining properties: `to`, `be`, `been`, … all return Assertion.
    const CHAIN_PROPS: &[&str] = &[
        "to", "be", "been", "is", "that", "which", "and", "has", "have",
        "with", "at", "of", "same", "but", "does", "not", "deep", "nested",
        "own", "ordered", "any", "all", "ok", "itself", "a", "an",
        "true", "false", "null", "undefined", "exist", "empty", "NaN", "finite",
        "extensible", "sealed", "frozen",
    ];

    // Terminal/chaining methods: all return Assertion so callers can keep
    // chaining if they want (chai's fluent API is uniform here).
    const CHAIN_METHODS: &[&str] = &[
        "equal", "equals", "eq", "eql", "eqls",
        "include", "includes", "contain", "contains",
        "throw", "throws", "Throw",
        "above", "gt", "greaterThan",
        "below", "lt", "lessThan",
        "least", "gte", "greaterThanOrEqual",
        "most", "lte", "lessThanOrEqual",
        "within",
        "instanceof", "instanceOf",
        "property", "ownProperty", "haveOwnProperty",
        "ownPropertyDescriptor", "haveOwnPropertyDescriptor",
        "keys", "key",
        "members", "oneOf",
        "satisfy", "satisfies",
        "match", "matches",
        "string",
        "length", "lengthOf",
        "closeTo", "approximately",
        "respondTo", "respondsTo",
        "increase", "increases",
        "decrease", "decreases",
        "change", "changes",
        "called", "calledWith", "calledOnce", "calledTwice",
        "calledBefore", "calledAfter",
    ];

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Index 0: chai.Assertion (interface — needed so external_type_qname("Assertion") works)
    symbols.push(sym(ASSERTION, "Assertion", SymbolKind::Interface, PKG, None));

    // Index 1: chai.ExpectStatic
    symbols.push(sym(
        &format!("{PKG}.ExpectStatic"),
        "ExpectStatic",
        SymbolKind::Interface,
        PKG,
        None,
    ));

    // Index 2: chai.expect (function returning Assertion)
    let expect_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{PKG}.expect"),
        "expect",
        SymbolKind::Function,
        PKG,
        None,
        &format!("expect(val: any): {ASSERTION}"),
    ));
    refs.push(type_ref(expect_idx, ASSERTION));

    // Chain properties — each is a Property with a TypeRef to chai.Assertion.
    for &prop in CHAIN_PROPS {
        let idx = symbols.len();
        symbols.push(sym(
            &format!("{ASSERTION}.{prop}"),
            prop,
            SymbolKind::Property,
            ASSERTION,
            Some(0),
        ));
        refs.push(type_ref(idx, ASSERTION));
    }

    // Chain methods — each is a Method with a TypeRef to chai.Assertion.
    for &method in CHAIN_METHODS {
        let idx = symbols.len();
        symbols.push(sym_with_sig(
            &format!("{ASSERTION}.{method}"),
            method,
            SymbolKind::Method,
            ASSERTION,
            Some(0),
            &format!("{method}(...): {ASSERTION}"),
        ));
        refs.push(type_ref(idx, ASSERTION));
    }

    make_parsed_file(PATH, symbols, refs)
}

// ---------------------------------------------------------------------------
// Vitest synthetic
// ---------------------------------------------------------------------------

/// Build a synthetic ParsedFile for Vitest's `vi.spyOn(...)` and Mock chains.
///
/// Symbol layout:
///
///   vitest.Vi                          — interface
///   vitest.Vi.spyOn                    — method → return_type = "vitest.MockInstance"
///   vitest.Vi.fn                       — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance                — interface
///   vitest.MockInstance.toHaveBeenCalled         — method → return_type = void (no TypeRef)
///   vitest.MockInstance.toHaveBeenCalledOnce     — method → return_type = void
///   vitest.MockInstance.toHaveBeenCalledTimes    — method → return_type = void
///   vitest.MockInstance.toHaveBeenCalledWith     — method → return_type = void
///   vitest.MockInstance.toHaveBeenLastCalledWith — method → return_type = void
///   vitest.MockInstance.toHaveBeenNthCalledWith  — method → return_type = void
///   vitest.MockInstance.toHaveReturned           — method → return_type = void
///   vitest.MockInstance.toHaveReturnedTimes      — method → return_type = void
///   vitest.MockInstance.toHaveReturnedWith       — method → return_type = void
///   vitest.MockInstance.toHaveLastReturnedWith   — method → return_type = void
///   vitest.MockInstance.toHaveNthReturnedWith    — method → return_type = void
///   vitest.MockInstance.mockReturnValue          — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockReturnValueOnce      — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockResolvedValue        — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockRejectedValue        — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockImplementation       — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockImplementationOnce   — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockReset                — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockRestore              — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mockClear                — method → return_type = "vitest.MockInstance"
///   vitest.MockInstance.mock                     — property → field_type = "vitest.MockContext"
///   vitest.MockContext                           — interface (mock.calls etc.)
///
/// Also covers the Jest-compat aliases vitest re-exports:
///   vitest.MockInstance aliases: mockReturnThis, getMockImplementation, etc.
fn vitest_synthetic() -> ParsedFile {
    const PKG: &str = "vitest";
    const VI: &str = "vitest.Vi";
    const MOCK: &str = "vitest.MockInstance";
    const MOCK_CTX: &str = "vitest.MockContext";
    const PATH: &str = "ext:ts:vitest/__bw_synthetic__.d.ts";

    // Vitest mock methods that return MockInstance (for chaining mock setups).
    const MOCK_CHAIN_METHODS: &[&str] = &[
        "mockReturnValue",
        "mockReturnValueOnce",
        "mockReturnThis",
        "mockResolvedValue",
        "mockResolvedValueOnce",
        "mockRejectedValue",
        "mockRejectedValueOnce",
        "mockImplementation",
        "mockImplementationOnce",
        "mockReset",
        "mockRestore",
        "mockClear",
        "getMockName",
        "mockName",
        "withImplementation",
    ];

    // Mock assertion methods (return void — no TypeRef needed, but we still
    // emit the symbol so the final segment of a chain resolves).
    const MOCK_ASSERT_METHODS: &[&str] = &[
        "toHaveBeenCalled",
        "toHaveBeenCalledOnce",
        "toHaveBeenCalledTimes",
        "toHaveBeenCalledWith",
        "toHaveBeenLastCalledWith",
        "toHaveBeenNthCalledWith",
        "toHaveReturned",
        "toHaveReturnedTimes",
        "toHaveReturnedWith",
        "toHaveLastReturnedWith",
        "toHaveNthReturnedWith",
    ];

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // vitest.Vi (interface)
    let vi_idx = symbols.len();
    symbols.push(sym(VI, "Vi", SymbolKind::Interface, PKG, None));

    // vi.spyOn → MockInstance
    let spy_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{VI}.spyOn"),
        "spyOn",
        SymbolKind::Method,
        VI,
        Some(vi_idx),
        &format!("spyOn(...): {MOCK}"),
    ));
    refs.push(type_ref(spy_idx, MOCK));

    // vi.fn → MockInstance
    let fn_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{VI}.fn"),
        "fn",
        SymbolKind::Method,
        VI,
        Some(vi_idx),
        &format!("fn(...): {MOCK}"),
    ));
    refs.push(type_ref(fn_idx, MOCK));

    // vi.mocked → MockInstance
    let mocked_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{VI}.mocked"),
        "mocked",
        SymbolKind::Method,
        VI,
        Some(vi_idx),
        &format!("mocked(...): {MOCK}"),
    ));
    refs.push(type_ref(mocked_idx, MOCK));

    // vitest.MockInstance (interface)
    let mock_idx = symbols.len();
    symbols.push(sym(MOCK, "MockInstance", SymbolKind::Interface, PKG, None));

    // Mock chaining methods → MockInstance
    for &method in MOCK_CHAIN_METHODS {
        let idx = symbols.len();
        symbols.push(sym_with_sig(
            &format!("{MOCK}.{method}"),
            method,
            SymbolKind::Method,
            MOCK,
            Some(mock_idx),
            &format!("{method}(...): {MOCK}"),
        ));
        refs.push(type_ref(idx, MOCK));
    }

    // Mock assertion methods (void return — emit symbol only, no TypeRef).
    for &method in MOCK_ASSERT_METHODS {
        symbols.push(sym_with_sig(
            &format!("{MOCK}.{method}"),
            method,
            SymbolKind::Method,
            MOCK,
            Some(mock_idx),
            &format!("{method}(...): void"),
        ));
    }

    // mock property → MockContext (for `.mock.calls` etc.)
    let mock_prop_idx = symbols.len();
    symbols.push(sym(
        &format!("{MOCK}.mock"),
        "mock",
        SymbolKind::Property,
        MOCK,
        Some(mock_idx),
    ));
    refs.push(type_ref(mock_prop_idx, MOCK_CTX));

    // vitest.MockContext (interface for calls/results/instances arrays)
    symbols.push(sym(MOCK_CTX, "MockContext", SymbolKind::Interface, PKG, None));

    // Top-level `vi` variable → type Vi (enables `vi.spyOn(...)` chain root)
    let vi_var_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{PKG}.vi"),
        "vi",
        SymbolKind::Variable,
        PKG,
        None,
        "vi: Vi",
    ));
    refs.push(type_ref(vi_var_idx, VI));

    make_parsed_file(PATH, symbols, refs)
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

fn sym(
    qualified_name: &str,
    name: &str,
    kind: SymbolKind,
    scope: &str,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: Some(scope.to_string()),
        parent_index,
    }
}

fn sym_with_sig(
    qualified_name: &str,
    name: &str,
    kind: SymbolKind,
    scope: &str,
    parent_index: Option<usize>,
    signature: &str,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(signature.to_string()),
        doc_comment: None,
        scope_path: Some(scope.to_string()),
        parent_index,
    }
}

fn type_ref(source_symbol_index: usize, target_name: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: target_name.to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        module: None,
        chain: None,
        byte_offset: 0,
    }
}

fn make_parsed_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    let n_syms = symbols.len();
    let n_refs = refs.len();
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: format!("synthetic-{path}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n_syms],
        ref_origin_languages: vec![None; n_refs],
        symbol_from_snippet: vec![false; n_syms],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn chai_synthetic_has_assertion_interface() {
        let pf = chai_synthetic();
        let assertion = pf.symbols.iter().find(|s| s.qualified_name == "chai.Assertion");
        assert!(assertion.is_some(), "chai.Assertion interface must be present");
        assert_eq!(assertion.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn chai_synthetic_chain_props_have_type_refs() {
        let pf = chai_synthetic();
        // Every chain property must have exactly one TypeRef pointing to chai.Assertion.
        for prop in &["to", "be", "been", "is", "that", "which", "and", "has", "have",
                      "not", "deep", "ok"] {
            let qname = format!("chai.Assertion.{prop}");
            let sym_idx = pf.symbols.iter().position(|s| s.qualified_name == qname)
                .unwrap_or_else(|| panic!("missing chain property {qname}"));
            assert_eq!(
                pf.symbols[sym_idx].kind,
                SymbolKind::Property,
                "{qname} must be a Property"
            );
            let has_ref = pf.refs.iter().any(|r| {
                r.source_symbol_index == sym_idx
                    && r.kind == EdgeKind::TypeRef
                    && r.target_name == "chai.Assertion"
            });
            assert!(has_ref, "{qname} must have a TypeRef to chai.Assertion");
        }
    }

    #[test]
    fn chai_synthetic_methods_have_type_refs() {
        let pf = chai_synthetic();
        for method in &["equal", "equals", "eq", "eql", "include", "throw", "match"] {
            let qname = format!("chai.Assertion.{method}");
            let sym_idx = pf.symbols.iter().position(|s| s.qualified_name == qname)
                .unwrap_or_else(|| panic!("missing chain method {qname}"));
            assert_eq!(
                pf.symbols[sym_idx].kind,
                SymbolKind::Method,
                "{qname} must be a Method"
            );
            let has_ref = pf.refs.iter().any(|r| {
                r.source_symbol_index == sym_idx
                    && r.kind == EdgeKind::TypeRef
                    && r.target_name == "chai.Assertion"
            });
            assert!(has_ref, "{qname} must have a TypeRef to chai.Assertion");
        }
    }

    #[test]
    fn chai_expect_returns_assertion() {
        let pf = chai_synthetic();
        let expect_sym = pf.symbols.iter().position(|s| s.qualified_name == "chai.expect")
            .expect("chai.expect must be present");
        assert_eq!(pf.symbols[expect_sym].kind, SymbolKind::Function);
        let has_ref = pf.refs.iter().any(|r| {
            r.source_symbol_index == expect_sym
                && r.kind == EdgeKind::TypeRef
                && r.target_name == "chai.Assertion"
        });
        assert!(has_ref, "chai.expect must have a TypeRef to chai.Assertion");
    }

    #[test]
    fn vitest_synthetic_has_vi_interface() {
        let pf = vitest_synthetic();
        let vi = pf.symbols.iter().find(|s| s.qualified_name == "vitest.Vi");
        assert!(vi.is_some(), "vitest.Vi interface must be present");
        assert_eq!(vi.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn vitest_spyon_returns_mock_instance() {
        let pf = vitest_synthetic();
        let spy_idx = pf.symbols.iter().position(|s| s.qualified_name == "vitest.Vi.spyOn")
            .expect("vitest.Vi.spyOn must be present");
        let has_ref = pf.refs.iter().any(|r| {
            r.source_symbol_index == spy_idx
                && r.kind == EdgeKind::TypeRef
                && r.target_name == "vitest.MockInstance"
        });
        assert!(has_ref, "vitest.Vi.spyOn must return vitest.MockInstance");
    }

    #[test]
    fn vitest_mock_chain_methods_present() {
        let pf = vitest_synthetic();
        for method in &[
            "toHaveBeenCalled",
            "toHaveBeenCalledOnce",
            "toHaveBeenCalledWith",
            "mockReturnValue",
            "mockImplementation",
            "mockReset",
        ] {
            let qname = format!("vitest.MockInstance.{method}");
            assert!(
                pf.symbols.iter().any(|s| s.qualified_name == qname),
                "missing {qname}"
            );
        }
    }

    #[test]
    fn parallel_vecs_are_consistent() {
        for pf in [chai_synthetic(), vitest_synthetic()] {
            assert_eq!(
                pf.symbols.len(),
                pf.symbol_origin_languages.len(),
                "symbol_origin_languages must match symbol count"
            );
            assert_eq!(
                pf.refs.len(),
                pf.ref_origin_languages.len(),
                "ref_origin_languages must match ref count"
            );
            assert_eq!(
                pf.symbols.len(),
                pf.symbol_from_snippet.len(),
                "symbol_from_snippet must match symbol count"
            );
        }
    }
}
