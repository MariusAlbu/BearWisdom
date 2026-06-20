// =============================================================================
// ecosystem/npm/post_process.rs — post-parse fixups for external d.ts files
// =============================================================================

use crate::ecosystem::externals::ts_package_from_virtual_path;

use super::symbol_index::NPM_GLOBALS_MODULE;
use super::ts_scan::scan_declare_global_blocks;
use super::{normalize_virtual_rel, LEGACY_ECOSYSTEM_TAG};

// ---------------------------------------------------------------------------
// Post-process: prefix declaration-file symbols with their package name
// ---------------------------------------------------------------------------

/// Prefix every symbol's `qualified_name` (and `scope_path`) in a parsed
/// TypeScript external file with the owning package name.
///
/// TypeScript declaration files don't carry a package-level scope, so the
/// extractor yields bare qualified names like `Button`. Rewrite them to
/// `fake-ui.Button` so the TS resolver's `{import_module}.{target}` lookup
/// matches. Idempotent: already-prefixed names are left alone.
/// Re-scan `source` for `declare global { ... }` blocks and inject a
/// `SymbolKind::Variable` entry per declared name that the TypeScript
/// extractor missed. The extractor's ambient-block descent is incomplete
/// (only a subset of const/let/function/class decls inside declare global
/// surface as top-level symbols), which starves the heuristic resolver's
/// declare-global priority of the files it relies on. Re-scanning at
/// post-process time is the narrowest correctness fix.
pub(crate) fn backfill_declare_global_symbols(pf: &mut crate::types::ParsedFile, source: &str) {
    use crate::types::{ExtractedSymbol, SymbolKind};

    let globals = scan_declare_global_blocks(source);
    if globals.is_empty() {
        return;
    }
    let existing: std::collections::HashSet<String> =
        pf.symbols.iter().map(|s| s.name.clone()).collect();
    let existing_qnames: std::collections::HashSet<String> = pf
        .symbols
        .iter()
        .map(|s| s.qualified_name.clone())
        .collect();
    for name in globals {
        // Dotted names (`Express.Multer.File`) are namespace paths whose
        // inner symbols the TS extractor already lifts as proper
        // class/interface/namespace symbols at the right qname. Emitting a
        // synthetic Variable here would only duplicate them under a name
        // the heuristic resolver's qname-derived index still wouldn't key
        // on (it uses the qname's last segment, not `sym.name`). Restrict
        // the backfill to flat top-level decls (`expect`, `describe`,
        // `Buffer`, `process`) where the extractor's ambient-block descent
        // is the unreliable bit.
        if name.contains('.') {
            continue;
        }
        // Primary entry — the symbol the package owns. After
        // prefix_ts_external_symbols runs, this becomes
        // `<package>.<name>` so package-qualified lookups
        // (`@angular/localize.$localize`) match.
        if !existing.contains(&name) {
            pf.symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name.clone(),
                kind: SymbolKind::Variable,
                visibility: None,
                start_line: 0,
                end_line: 0,
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
            });
        }
        // Shadow entry under the synthetic globals namespace so the
        // resolver's bare-name fallback (`ts_npm_globals` strategy in
        // languages/typescript/resolve.rs) can find globals like
        // `$localize`, `describe`, `it`, `cy`, `expect` that the source
        // references without an explicit `import`. The shadow is a
        // separate symbol so the package-prefix pass below leaves its
        // qname intact (it short-circuits when the qname already starts
        // with the package prefix; we add a special-case for the globals
        // namespace right next to that check).
        let shadow_qname = format!("{NPM_GLOBALS_MODULE}.{name}");
        if !existing_qnames.contains(&shadow_qname) {
            pf.symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: shadow_qname,
                kind: SymbolKind::Variable,
                visibility: None,
                start_line: 0,
                end_line: 0,
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
            });
        }
    }
}

/// Post-process a TS external file pulled through the demand-driven path.
/// Mirrors what the eager-walk locator's `post_process_parsed` does: scan
/// for `declare global` / `declare namespace` blocks and inject any names
/// the extractor missed, then prefix every symbol's qname with the owning
/// package. Both demand-driven entry points (`stage_link::seed_demand_*`
/// and `expand::expand_*`) must call this so the symbol table is shaped
/// the same regardless of which pass pulled the file in.
pub(crate) fn ts_post_process_external(pf: &mut crate::types::ParsedFile) {
    let Some(pkg) = ts_package_from_virtual_path(&pf.path).map(str::to_string) else {
        return;
    };
    // TS core lib (lib.dom.d.ts, lib.es*.d.ts, …) declares runtime globals
    // — `HTMLElement`, `Document`, `Promise`, etc. — at ambient scope.
    // Prefixing them under a synthetic package would mangle their qnames
    // away from the bare names the chain walker queries, so we skip the
    // post-processing entirely for files served from the synthetic
    // `__ts_lib__` module. Backfill is also a no-op for these — they
    // don't carry `declare global { … }` blocks.
    if pkg == crate::ecosystem::ts_lib_dom::TS_LIB_SYNTHETIC_MODULE {
        return;
    }
    let source_snapshot = pf.content.clone();
    if let Some(source) = source_snapshot.as_deref() {
        if source.contains("declare global") || source.contains("declare namespace") {
            backfill_declare_global_symbols(pf, source);
        }
    }
    prefix_ts_external_symbols(pf, &pkg);
}

pub(crate) fn prefix_ts_external_symbols(pf: &mut crate::types::ParsedFile, package: &str) {
    if package.is_empty() {
        return;
    }
    let prefix = format!("{package}.");
    let globals_prefix = format!("{NPM_GLOBALS_MODULE}.");
    for sym in &mut pf.symbols {
        // Shadow symbols pushed by `backfill_declare_global_symbols` carry
        // the synthetic globals qname so the resolver's bare-name fallback
        // can find them. Don't tack the package prefix in front — that
        // would mangle the namespace key the resolver looks up.
        if sym.qualified_name.starts_with(&globals_prefix) {
            continue;
        }
        if !sym.qualified_name.starts_with(&prefix) {
            sym.qualified_name = format!("{prefix}{}", sym.qualified_name);
        }
        sym.scope_path = match sym.scope_path.take() {
            Some(sp) if !sp.starts_with(&prefix) => Some(format!("{prefix}{sp}")),
            Some(sp) => Some(sp),
            None => Some(package.to_string()),
        };
    }
    // Alias targets are keyed by the bare type name the extractor recorded
    // (`Result`), but the chain walker looks them up by the receiver's
    // package-qualified head (`@scope/pkg.Result`, derived from a method's
    // qualified return type). Qualify the keys to match. Without this,
    // member lookup THROUGH an external alias (intersection / mapped — e.g.
    // `RenderResult`'s `BoundFunctions` branch) misses, while own-member
    // lookup still works because members key on the already-qualified parent.
    for (name, _) in pf.alias_targets.iter_mut() {
        if !name.starts_with(&prefix) && !name.starts_with(&globals_prefix) {
            *name = format!("{prefix}{name}");
        }
    }
}
