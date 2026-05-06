// =============================================================================
// ecosystem/hexo_runtime.rs — Hexo helper discovery (real on-disk)
//
// Hexo is a static-site generator that injects template helpers
// (`partial`, `is_home`, `is_post`, `url_for`, `clean_url`, `paginator`,
// ...) into EJS / JS render contexts. These names are not declared in
// the user's templates — they're registered at runtime by the Hexo
// engine (`hexo/dist/plugins/helper/index.js`) and by the project's
// own theme scripts (`hexo.extend.helper.register('NAME', fn)`).
//
// Without a discovery path, every theme template that calls
// `is_home()` / `url_for(...)` / `clean_url(...)` lands in
// unresolved_refs. The hexo-reference theme corpus shows ~290
// unresolved calls of this exact shape.
//
// **Discovery strategy** (real on-disk only, no vendored data):
//   1. Activation: project has `_config.yml` whose head mentions Hexo,
//      OR has a `node_modules/hexo` directory.
//   2. Hexo core helpers — scan
//      `node_modules/hexo/{dist,lib}/plugins/helper/index.js` for
//      `helper.register('NAME', ...)` literal-string registrations.
//   3. Theme + project scripts — walk `themes/*/scripts/**/*.js` and
//      `scripts/**/*.js` for
//      `hexo.extend.helper.register('NAME', ...)` registrations.
//   4. Emit a single synthetic ParsedFile (`ext:hexo-runtime:helpers.js`)
//      containing one Function symbol per discovered helper name. The
//      EJS / JS resolvers' by-name fallback then matches calls against
//      these symbols.
//
// When neither Hexo install nor theme scripts are reachable, the
// ecosystem returns no roots — refs to helpers stay unresolved (the
// honest signal).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("hexo-runtime");
const ECOSYSTEM_TAG: &str = "hexo-runtime";
const LANGUAGES: &[&str] = &["ejs", "javascript"];

pub struct HexoRuntimeEcosystem;

impl Ecosystem for HexoRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // Hexo is a project dep, not an EJS substrate. Gate on the project's
        // package.json declaring `hexo` in dependencies or devDependencies.
        // EJS-only projects (Express templates, Adonis views, ...) skip this.
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/package.json",
                field_path: "dependencies",
                value: "hexo",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/package.json",
                field_path: "devDependencies",
                value: "hexo",
            },
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_hexo_root(ctx.project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesise_hexo_helpers(&dep.root))
    }
}

impl ExternalSourceLocator for HexoRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_hexo_root(project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> { Vec::new() }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<ParsedFile>> {
        let roots = discover_hexo_root(project_root);
        let root = roots.first()?;
        Some(synthesise_hexo_helpers(&root.root))
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<HexoRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(HexoRuntimeEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_hexo_root(project_root: &Path) -> Vec<ExternalDepRoot> {
    if !looks_like_hexo_project(project_root) {
        return Vec::new();
    }
    vec![ExternalDepRoot {
        module_path: "hexo".to_string(),
        version: String::from("local"),
        root: project_root.to_path_buf(),
        ecosystem: ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

/// A project is a Hexo blog when either:
///   * `node_modules/hexo` is installed (most reliable signal), OR
///   * `_config.yml` exists at root with a Hexo-flavoured header.
pub(crate) fn looks_like_hexo_project(root: &Path) -> bool {
    if root.join("node_modules").join("hexo").is_dir() {
        return true;
    }
    let config = root.join("_config.yml");
    if let Ok(text) = std::fs::read_to_string(&config) {
        let head: String = text.lines().take(20).collect::<Vec<_>>().join("\n");
        let lower = head.to_ascii_lowercase();
        if lower.contains("hexo") {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Synthesis
// ---------------------------------------------------------------------------

/// Emit one synthetic `ParsedFile` whose symbols are every Hexo helper
/// name reachable from `project_root`:
///   * core helpers from `node_modules/hexo/{dist,lib}/plugins/helper/
///     index.js` (literal `helper.register('NAME', ...)`)
///   * theme + project helpers from `themes/*/scripts/**/*.js` and
///     `scripts/**/*.js` (literal `hexo.extend.helper.register('NAME',
///     ...)`)
///
/// Names are dedup'd; the symbol's `qualified_name` carries the
/// origin (`hexo.core` vs `hexo.theme`) so downstream tools can
/// distinguish.
pub(crate) fn synthesise_hexo_helpers(project_root: &Path) -> Vec<ParsedFile> {
    let mut core_names: Vec<String> = Vec::new();
    let mut theme_names: Vec<String> = Vec::new();

    // --- Hexo core ---------------------------------------------------------
    for rel in &[
        "node_modules/hexo/dist/plugins/helper/index.js",
        "node_modules/hexo/lib/plugins/helper/index.js",
    ] {
        let p = project_root.join(rel);
        if let Ok(text) = std::fs::read_to_string(&p) {
            extract_helper_names(&text, "helper.register", &mut core_names);
        }
    }

    // --- Theme + project scripts ------------------------------------------
    let mut script_dirs: Vec<PathBuf> = Vec::new();
    let themes = project_root.join("themes");
    if themes.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&themes) {
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    let scripts = path.join("scripts");
                    if scripts.is_dir() {
                        script_dirs.push(scripts);
                    }
                }
            }
        }
    }
    let root_scripts = project_root.join("scripts");
    if root_scripts.is_dir() {
        script_dirs.push(root_scripts);
    }
    for dir in &script_dirs {
        scan_scripts_dir(dir, &mut theme_names, 0);
    }

    if core_names.is_empty() && theme_names.is_empty() {
        return Vec::new();
    }

    // Emit each helper under the `__npm_globals__.<name>` qualified name —
    // the same namespace Handlebars/Ember register-helper synthetics use.
    // This makes the existing TypeScript resolver's `ts_npm_globals`
    // fallback (which the JS resolver delegates to, including for embedded
    // JS in .ejs files) match these symbols without any per-language hook.
    // Originating scope (`hexo.core` vs `hexo.theme`) is preserved on
    // `scope_path` for downstream tools that want to disambiguate.
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let globals = crate::ecosystem::npm::NPM_GLOBALS_MODULE;
    for (name, scope) in core_names.iter().map(|n| (n, "hexo.core"))
        .chain(theme_names.iter().map(|n| (n, "hexo.theme")))
    {
        if !emitted.insert(name.clone()) {
            continue;
        }
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: format!("{globals}.{name}"),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("Hexo helper from {scope}")),
            doc_comment: None,
            scope_path: Some(scope.to_string()),
            parent_index: None,
        });
    }

    if symbols.is_empty() {
        return Vec::new();
    }
    tracing::info!(
        "hexo-runtime: extracted {} core + {} theme helper(s) from {}",
        core_names.len(),
        theme_names.len(),
        project_root.display()
    );

    let n = symbols.len();
    vec![ParsedFile {
        path: "ext:hexo-runtime:helpers.js".to_string(),
        language: "javascript".to_string(),
        content_hash: format!("hexo-runtime-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }]
}

fn scan_scripts_dir(dir: &Path, out: &mut Vec<String>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || name == "node_modules" {
                    continue;
                }
            }
            scan_scripts_dir(&path, out, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("js") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                extract_helper_names(&text, "hexo.extend.helper.register", out);
                // Some themes alias `hexo.extend.helper` to a local var;
                // also catch the bare `helper.register` form when the
                // file imports / unpacks it that way.
                extract_helper_names(&text, "helper.register", out);
            }
        }
    }
}

/// Scan `text` for `<marker>('NAME', ...)` or `<marker>("NAME", ...)`
/// patterns and append each unique NAME to `out`. The marker is matched
/// at an identifier boundary — `xhelper.register` won't false-trigger.
pub(crate) fn extract_helper_names(text: &str, marker: &str, out: &mut Vec<String>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(rel) = text[i..].find(marker) else { break };
        let start = i + rel;
        // Identifier-boundary check on the byte before the match.
        if start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                i = start + marker.len();
                continue;
            }
        }
        let after_marker = start + marker.len();
        // Expect `(` (allowing whitespace).
        let mut j = after_marker;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            i = after_marker;
            continue;
        }
        j += 1;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
            j += 1;
        }
        if j >= bytes.len() {
            i = after_marker;
            continue;
        }
        let quote = bytes[j];
        if quote != b'\'' && quote != b'"' && quote != b'`' {
            i = after_marker;
            continue;
        }
        let name_start = j + 1;
        let mut k = name_start;
        while k < bytes.len() {
            if bytes[k] == b'\\' && k + 1 < bytes.len() {
                k += 2;
                continue;
            }
            if bytes[k] == quote {
                break;
            }
            k += 1;
        }
        if k >= bytes.len() {
            i = after_marker;
            continue;
        }
        let raw = &text[name_start..k];
        let name = raw.trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
        i = k + 1;
    }
}

#[cfg(test)]
#[path = "hexo_runtime_tests.rs"]
mod tests;
