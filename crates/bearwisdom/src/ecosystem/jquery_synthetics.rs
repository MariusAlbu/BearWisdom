// =============================================================================
// ecosystem/jquery_synthetics.rs — Synthetic chain-type entries for jQuery
//
// `$(sel).addClass(...).on('click', fn)` chains through a JQuery object whose
// methods mostly return JQuery for fluent chaining. Library symbols must not
// live in is_*_builtin predicates (see feedback_no_hardcoded_library_builtins) —
// this file models jQuery's runtime surface as a synthetic NPM package so the
// chain walker can traverse it via field_type / return_type like any other
// typed library.
//
// Scope:
//   jquery.JQuery       — interface with the ~80 chainable methods
//   jquery.$            — function → return_type = jquery.JQuery
//   jquery.jquery       — alias of $, used when `import jquery from 'jquery'`
//
// Globals (for pages that inline-load via <script src="jquery.js"> and have
// no import statement in scope):
//   __npm_globals__.$ / $$ / jQuery — function → return_type = jquery.JQuery
//
// Fires when `node_modules/jquery` is present in any discoverable node_modules
// directory (same probe order as dayjs_synthetics and js_test_chains).
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("jquery-synthetics");
const LEGACY_ECOSYSTEM_TAG: &str = "jquery-synthetics";
const LANGUAGES: &[&str] = &["javascript", "typescript"];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Returns a synthetic ParsedFile for jQuery when jQuery appears to be in
/// use in the project. Probes the npm location (node_modules/jquery) plus
/// classic Rails / PHP / generic asset-pipeline conventions so the
/// synthetic fires for server-rendered projects that include jQuery via
/// `<script src=…>` or `//= require jquery` instead of npm.
pub fn synthetic_jquery_file(project_root: &Path) -> Option<ParsedFile> {
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

    let npm_present = nm_dirs.iter().any(|nm| {
        nm.join("jquery").join("package.json").exists()
            || nm.join("jquery").join("dist").join("jquery.js").exists()
    });
    if npm_present {
        return Some(jquery_synthetic());
    }

    // Rails asset pipeline / generic server-rendered project conventions.
    // Check a handful of directories for any `jquery*.js` file. Directory
    // scans are scoped to shallow asset folders to keep activation cheap.
    const ASSET_DIRS: &[&str] = &[
        "vendor/assets/javascripts",
        "app/assets/javascripts",
        "public/javascripts",
        "public/assets",
        "public/js",
        "assets/js",
        "web/static/vendor",
    ];
    for dir_rel in ASSET_DIRS {
        let dir = project_root.join(dir_rel);
        if !dir.is_dir() { continue; }
        if dir_has_jquery_file(&dir) {
            return Some(jquery_synthetic());
        }
    }

    // Ruby: `gem 'jquery-rails'` in Gemfile → classic Rails jQuery include.
    if let Ok(gemfile) = std::fs::read_to_string(project_root.join("Gemfile")) {
        if gemfile.contains("jquery-rails") || gemfile.contains("'jquery'") {
            return Some(jquery_synthetic());
        }
    }

    None
}

/// Scan a directory (non-recursive, cheap) for any file whose name starts
/// with `jquery` and ends with `.js` — covers `jquery.js`, `jquery.min.js`,
/// `jquery-3.6.0.min.js`, etc.
fn dir_has_jquery_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false; };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("jquery") && lower.ends_with(".js") {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// jQuery synthetic
// ---------------------------------------------------------------------------

fn jquery_synthetic() -> ParsedFile {
    const PKG: &str = "jquery";
    const JQ: &str = "jquery.JQuery";
    const PATH: &str = "ext:ts:jquery/__bw_synthetic__.d.ts";
    const GLOBALS: &str = "__npm_globals__";

    // Fluent methods — all return JQuery. The set covers the common DOM-
    // manipulation, traversal, event, and effects surface. A handful of
    // methods (`.is()`, `.hasClass()`, `.length`) actually return boolean /
    // number at runtime, but modelling them as JQuery-returning is a harmless
    // over-approximation for chain continuation: they almost never appear in
    // the middle of a chain, and when they do the resolver just overshoots.
    const CHAIN_METHODS: &[&str] = &[
        // class / attribute manipulation
        "addClass", "removeClass", "toggleClass", "hasClass",
        "css", "html", "text", "val", "attr", "prop", "removeAttr", "removeProp", "data", "removeData",
        // visibility / effects
        "hide", "show", "toggle",
        "fadeIn", "fadeOut", "fadeToggle", "fadeTo",
        "slideDown", "slideUp", "slideToggle",
        "animate", "stop", "delay", "queue", "dequeue", "clearQueue", "finish",
        // events
        "on", "off", "one", "trigger", "triggerHandler", "bind", "unbind",
        "hover", "ready", "click", "dblclick", "change", "submit", "focus", "blur",
        "focusin", "focusout", "keydown", "keyup", "keypress",
        "mousedown", "mouseup", "mousemove", "mouseenter", "mouseleave", "mouseover", "mouseout",
        "scroll", "resize", "select", "load", "unload",
        // DOM insertion / removal
        "appendTo", "prependTo", "insertAfter", "insertBefore",
        "after", "before", "prepend", "append",
        "detach", "remove", "empty",
        "replaceWith", "replaceAll",
        "wrap", "unwrap", "wrapAll", "wrapInner",
        // traversal
        "parent", "parents", "parentsUntil", "offsetParent",
        "children", "siblings", "closest",
        "next", "nextAll", "nextUntil",
        "prev", "prevAll", "prevUntil",
        "find", "filter", "not", "is", "has",
        "eq", "first", "last", "slice", "index",
        "add", "addBack", "end", "contents", "pushStack",
        // iteration
        "each", "map",
        // conversion / measurement
        "toArray",
        "width", "height", "innerWidth", "innerHeight", "outerWidth", "outerHeight",
        "offset", "position",
        "scrollTop", "scrollLeft",
        // form serialization
        "serialize", "serializeArray",
        // promise-ish
        "promise", "done", "fail", "then", "always",
        // internals most user code never touches but still chain
        "get",
    ];

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // jquery.JQuery interface — the chain receiver.
    let jq_idx = symbols.len();
    symbols.push(sym(JQ, "JQuery", SymbolKind::Interface, PKG, None));

    // jquery.$ — the primary selector/factory function.
    let dollar_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{PKG}.$"),
        "$",
        SymbolKind::Function,
        PKG,
        None,
        &format!("$(selector: any, context?: any): {JQ}"),
    ));
    refs.push(type_ref(dollar_idx, JQ));

    // jquery.jquery — default-export alias (`import jquery from 'jquery'`).
    let jquery_idx = symbols.len();
    symbols.push(sym_with_sig(
        &format!("{PKG}.jquery"),
        "jquery",
        SymbolKind::Function,
        PKG,
        None,
        &format!("jquery(selector: any, context?: any): {JQ}"),
    ));
    refs.push(type_ref(jquery_idx, JQ));

    // Chainable methods on JQuery.
    for &method in CHAIN_METHODS {
        let idx = symbols.len();
        symbols.push(sym_with_sig(
            &format!("{JQ}.{method}"),
            method,
            SymbolKind::Method,
            JQ,
            Some(jq_idx),
            &format!("{method}(...): {JQ}"),
        ));
        refs.push(type_ref(idx, JQ));
    }

    // __npm_globals__ entries — for HTML/ERB/EEX templates that inline-load
    // jQuery via <script src="jquery.js"> and never import the package. The
    // call-root helper probes these as the last fallback for bare callees.
    // `$$` commonly aliases the same selector in Prototype/cash-dom-adjacent
    // setups; we point it at jquery.$ so code that treats them interchangeably
    // walks the JQuery chain without special-casing.
    for alias in ["$", "$$", "jQuery"] {
        let g_idx = symbols.len();
        symbols.push(sym_with_sig(
            &format!("{GLOBALS}.{alias}"),
            alias,
            SymbolKind::Function,
            GLOBALS,
            None,
            &format!("{alias}(selector: any, context?: any): {JQ}"),
        ));
        refs.push(type_ref(g_idx, JQ));
    }

    make_parsed_file(PATH, symbols, refs)
}

// ---------------------------------------------------------------------------
// Builder helpers (duplicated from dayjs_synthetics for locality)
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
// Ecosystem impl — activates on JS/TS language presence so the synthetic
// fires for Rails / PHP / asset-pipeline projects without a node_modules
// tree as well as npm-based projects. NpmEcosystem used to emit jQuery
// synthetics via parse_metadata_only; that path has been removed — this
// is now the single source of truth.
// ---------------------------------------------------------------------------

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "jquery-synthetics".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:jquery-synthetics"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct JquerySynthEcosystem;

impl Ecosystem for JquerySynthEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("javascript"),
            EcosystemActivation::LanguagePresent("typescript"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        if synthetic_jquery_file(ctx.project_root).is_some() {
            vec![synthetic_dep_root()]
        } else {
            Vec::new()
        }
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![jquery_synthetic()])
    }
}

impl ExternalSourceLocator for JquerySynthEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        if synthetic_jquery_file(project_root).is_some() {
            vec![synthetic_dep_root()]
        } else {
            Vec::new()
        }
    }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<ParsedFile>> {
        synthetic_jquery_file(project_root).map(|pf| vec![pf])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jquery_synthetic_has_jquery_interface() {
        let pf = jquery_synthetic();
        let iface = pf.symbols.iter().find(|s| s.qualified_name == "jquery.JQuery");
        assert!(iface.is_some(), "jquery.JQuery interface must be present");
        assert_eq!(iface.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn dollar_fn_returns_jquery_type() {
        let pf = jquery_synthetic();
        let fn_idx = pf
            .symbols
            .iter()
            .position(|s| s.qualified_name == "jquery.$")
            .expect("jquery.$ must be present");
        assert_eq!(pf.symbols[fn_idx].kind, SymbolKind::Function);
        let has_ref = pf.refs.iter().any(|r| {
            r.source_symbol_index == fn_idx
                && r.kind == EdgeKind::TypeRef
                && r.target_name == "jquery.JQuery"
        });
        assert!(has_ref, "jquery.$ must have a TypeRef to jquery.JQuery");
    }

    #[test]
    fn chain_methods_return_jquery() {
        let pf = jquery_synthetic();
        for method in &["addClass", "on", "trigger", "css", "html", "animate", "find"] {
            let qname = format!("jquery.JQuery.{method}");
            let idx = pf
                .symbols
                .iter()
                .position(|s| s.qualified_name == qname)
                .unwrap_or_else(|| panic!("missing chain method {qname}"));
            assert_eq!(pf.symbols[idx].kind, SymbolKind::Method);
            let has_ref = pf.refs.iter().any(|r| {
                r.source_symbol_index == idx
                    && r.kind == EdgeKind::TypeRef
                    && r.target_name == "jquery.JQuery"
            });
            assert!(has_ref, "{qname} must TypeRef jquery.JQuery");
        }
    }

    #[test]
    fn npm_globals_alias_dollar_to_jquery() {
        let pf = jquery_synthetic();
        for alias in ["$", "$$", "jQuery"] {
            let qname = format!("__npm_globals__.{alias}");
            let idx = pf
                .symbols
                .iter()
                .position(|s| s.qualified_name == qname)
                .unwrap_or_else(|| panic!("missing global {qname}"));
            assert_eq!(
                pf.symbols[idx].kind,
                SymbolKind::Function,
                "{qname} must be Function so return_type_name fires"
            );
            let has_ref = pf.refs.iter().any(|r| {
                r.source_symbol_index == idx
                    && r.kind == EdgeKind::TypeRef
                    && r.target_name == "jquery.JQuery"
            });
            assert!(has_ref, "{qname} must TypeRef jquery.JQuery");
        }
    }

    #[test]
    fn parallel_vecs_are_consistent() {
        let pf = jquery_synthetic();
        assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
        assert_eq!(pf.refs.len(), pf.ref_origin_languages.len());
        assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
    }

    #[test]
    fn activation_fires_for_rails_asset_pipeline() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let assets = root.join("app").join("assets").join("javascripts");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("jquery.min.js"), "// jquery").unwrap();
        assert!(
            synthetic_jquery_file(root).is_some(),
            "Rails app/assets/javascripts/jquery*.js must activate jquery synthetic"
        );
    }

    #[test]
    fn activation_fires_for_vendor_assets_jquery() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let vendor = root.join("vendor").join("assets").join("javascripts");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join("jquery-3.6.0.js"), "// jquery").unwrap();
        assert!(synthetic_jquery_file(root).is_some());
    }

    #[test]
    fn activation_fires_for_gemfile_jquery_rails() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("Gemfile"), "gem 'jquery-rails'\n").unwrap();
        assert!(synthetic_jquery_file(root).is_some());
    }

    #[test]
    fn activation_does_not_fire_for_empty_project() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            synthetic_jquery_file(tmp.path()).is_none(),
            "empty project must not trigger jquery synthetic"
        );
    }
}
