// =============================================================================
// ecosystem/dayjs_synthetics.rs — Synthetic chain-type entries for dayjs
//
// dayjs() returns a Dayjs object whose methods return the same type for
// fluent chaining: `dayjs().tz('UTC').format('YYYY-MM-DD')`.
//
// Root cause: the tree-sitter extractor sees `dayjs` as a plain identifier
// at the chain root; `resolve_call_root_type` probes `{module}.dayjs` for a
// return_type to seed the chain walk. Without the synthetic, `dayjs.dayjs`
// has no return_type recorded, so the chain walker bails immediately and the
// intermediate calls are unresolved.
//
// Fix: inject a synthetic ParsedFile that declares:
//   dayjs.dayjs  — function → return_type = "dayjs.Dayjs"
//   dayjs.Dayjs  — interface with every fluent method returning "dayjs.Dayjs"
//
// Called from NpmEcosystem::parse_metadata_only when dayjs is present.
// =============================================================================

use std::path::Path;

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Returns a synthetic ParsedFile for dayjs when the package is present
/// under any discoverable node_modules directory. Returns None otherwise.
pub fn synthetic_dayjs_file(project_root: &Path) -> Option<ParsedFile> {
    let mut nm_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue; }
            if seg.is_dir() { nm_dirs.push(seg); }
        }
    }
    if nm_dirs.is_empty() {
        let local = project_root.join("node_modules");
        if local.is_dir() { nm_dirs.push(local); }
    }

    let dayjs_present = nm_dirs.iter().any(|nm| {
        nm.join("dayjs").join("package.json").exists()
            || nm.join("dayjs").join("index.d.ts").exists()
    });

    if dayjs_present { Some(dayjs_synthetic()) } else { None }
}

// ---------------------------------------------------------------------------
// Dayjs synthetic
// ---------------------------------------------------------------------------

/// Build a synthetic ParsedFile for dayjs fluent chains.
///
/// Symbol layout:
///
///   dayjs.Dayjs       — interface (enables external_type_qname lookup)
///   dayjs.dayjs       — function → return_type = "dayjs.Dayjs"
///
///   Fluent methods (all return dayjs.Dayjs):
///   dayjs.Dayjs.format, add, subtract, tz, utc, local,
///   startOf, endOf, diff, isValid, valueOf, toDate, toISOString,
///   toString, clone, year, month, day, date, hour, minute, second,
///   millisecond, week, weekday, quarter, dayOfYear, unix,
///   isBefore, isSame, isAfter, isSameOrBefore, isSameOrAfter,
///   isBetween, locale, fromNow, from, toNow, to, calendar,
///   humanize, set, get, daysInMonth, toArray, toObject
fn dayjs_synthetic() -> ParsedFile {
    const PKG: &str = "dayjs";
    const DAYJS_TYPE: &str = "dayjs.Dayjs";
    const PATH: &str = "ext:ts:dayjs/__bw_synthetic__.d.ts";

    const CHAIN_METHODS: &[&str] = &[
        "format",
        "add",
        "subtract",
        "tz",
        "utc",
        "local",
        "startOf",
        "endOf",
        "diff",
        "isValid",
        "valueOf",
        "toDate",
        "toISOString",
        "toString",
        "clone",
        "year",
        "month",
        "day",
        "date",
        "hour",
        "minute",
        "second",
        "millisecond",
        "week",
        "weekday",
        "quarter",
        "dayOfYear",
        "unix",
        "isBefore",
        "isSame",
        "isAfter",
        "isSameOrBefore",
        "isSameOrAfter",
        "isBetween",
        "locale",
        "fromNow",
        "from",
        "toNow",
        "to",
        "calendar",
        "humanize",
        "set",
        "get",
        "daysInMonth",
        "toArray",
        "toObject",
    ];

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // dayjs.Dayjs interface — needed so external_type_qname("Dayjs") works.
    let dayjs_type_idx = symbols.len();
    symbols.push(sym(DAYJS_TYPE, "Dayjs", SymbolKind::Interface, PKG, None));

    // dayjs.dayjs function → return_type = dayjs.Dayjs.
    // This is the call-root: `dayjs()` resolves to Dayjs.
    let dayjs_fn_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{PKG}.{PKG}"),
        PKG,
        SymbolKind::Function,
        PKG,
        None,
        &format!("dayjs(...): {DAYJS_TYPE}"),
    ));
    refs.push(type_ref(dayjs_fn_idx, DAYJS_TYPE));

    // Fluent chain methods — each returns Dayjs so chaining continues.
    for &method in CHAIN_METHODS {
        let idx = symbols.len();
        symbols.push(sym_with_sig(
            &format!("{DAYJS_TYPE}.{method}"),
            method,
            SymbolKind::Method,
            DAYJS_TYPE,
            Some(dayjs_type_idx),
            &format!("{method}(...): {DAYJS_TYPE}"),
        ));
        refs.push(type_ref(idx, DAYJS_TYPE));
    }

    make_parsed_file(PATH, symbols, refs)
}

// ---------------------------------------------------------------------------
// Builder helpers (duplicated from js_test_chains for locality)
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
    fn dayjs_synthetic_has_dayjs_interface() {
        let pf = dayjs_synthetic();
        let iface = pf.symbols.iter().find(|s| s.qualified_name == "dayjs.Dayjs");
        assert!(iface.is_some(), "dayjs.Dayjs interface must be present");
        assert_eq!(iface.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn dayjs_fn_returns_dayjs_type() {
        let pf = dayjs_synthetic();
        let fn_idx = pf
            .symbols
            .iter()
            .position(|s| s.qualified_name == "dayjs.dayjs")
            .expect("dayjs.dayjs must be present");
        assert_eq!(pf.symbols[fn_idx].kind, SymbolKind::Function);
        let has_ref = pf.refs.iter().any(|r| {
            r.source_symbol_index == fn_idx
                && r.kind == EdgeKind::TypeRef
                && r.target_name == "dayjs.Dayjs"
        });
        assert!(has_ref, "dayjs.dayjs must have a TypeRef to dayjs.Dayjs");
    }

    #[test]
    fn dayjs_chain_methods_have_type_refs() {
        let pf = dayjs_synthetic();
        for method in &["format", "add", "subtract", "tz", "utc", "startOf", "endOf"] {
            let qname = format!("dayjs.Dayjs.{method}");
            let sym_idx = pf
                .symbols
                .iter()
                .position(|s| s.qualified_name == qname)
                .unwrap_or_else(|| panic!("missing chain method {qname}"));
            assert_eq!(
                pf.symbols[sym_idx].kind,
                SymbolKind::Method,
                "{qname} must be a Method"
            );
            let has_ref = pf.refs.iter().any(|r| {
                r.source_symbol_index == sym_idx
                    && r.kind == EdgeKind::TypeRef
                    && r.target_name == "dayjs.Dayjs"
            });
            assert!(has_ref, "{qname} must have a TypeRef to dayjs.Dayjs");
        }
    }

    #[test]
    fn parallel_vecs_are_consistent() {
        let pf = dayjs_synthetic();
        assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
        assert_eq!(pf.refs.len(), pf.ref_origin_languages.len());
        assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
    }
}
