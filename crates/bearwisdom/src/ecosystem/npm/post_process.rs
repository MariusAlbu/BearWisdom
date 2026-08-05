// =============================================================================
// ecosystem/npm/post_process.rs — post-parse fixups for external d.ts files
// =============================================================================

use crate::ecosystem::externals::ts_package_from_virtual_path;

use super::symbol_index::NPM_GLOBALS_MODULE;

#[cfg(test)]
#[path = "post_process_tests.rs"]
mod tests;
use super::ts_scan::{scan_declare_global_blocks, scan_global_script_top_level_decls};
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

    let mut globals = scan_declare_global_blocks(source);
    // A global-script `.d.ts` (no top-level import/export) contributes its
    // top-level `declare const`/`var`/`function`/… as ambient globals too —
    // the `@types/jest` shape (`declare const expect: jest.Expect`) the block /
    // namespace sweep above misses.
    globals.extend(scan_global_script_top_level_decls(source));
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
pub(crate) fn ts_post_process_external(
    pf: &mut crate::types::ParsedFile,
    arena: &crate::type_checker::core::types::TypeArena,
) {
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
        // `declare ` is the necessary precondition for any ambient contribution —
        // `declare global { … }`, `declare namespace`, and the global-script
        // top-level `declare const/var/…` all carry it; a file without it can
        // produce no globals, so the backfill would no-op.
        if source.contains("declare ") {
            backfill_declare_global_symbols(pf, source);
        }
    }
    prefix_ts_external_symbols(pf, &pkg, arena);
    // `extract_component_selectors` (run in the parse pass) keyed each selector on
    // the class's BARE qname, but every symbol was just package-prefixed — so prefix
    // the selector's class qname to match. An external `<nb-card>` must map to
    // `@nebular/theme.NbCardComponent`, the qname its class symbol now carries.
    let prefix = format!("{pkg}.");
    for (_selector, class_qname) in &mut pf.component_selectors {
        if !class_qname.starts_with(&prefix) {
            *class_qname = format!("{prefix}{class_qname}");
        }
    }
}

pub(crate) fn prefix_ts_external_symbols(
    pf: &mut crate::types::ParsedFile,
    package: &str,
    arena: &crate::type_checker::core::types::TypeArena,
) {
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
        // The extractor stamps declared_type / return_type / param_types /
        // generic_params with TypeIds keyed on the symbol's BARE (pre-prefix)
        // qname. After prefixing, those TypeIds reference a name no symbol
        // carries, so Phase A would write a self/return type the chain walker
        // can't resolve and Phase B (which only fills empty slots) couldn't
        // correct it. A Variable's declared type (`declare const api: ApiType`)
        // is requalified with the package prefix instead — the annotation names
        // a type in the package's own surface, and the ref that carried the
        // local name is rewritten to the import's source name by the
        // import-semantics pass, so Phase B has nothing to re-derive it from.
        // Everything else is cleared so Phase B re-derives the
        // correctly-qualified types from refs/signatures.
        sym.declared_type = match sym.declared_type.take() {
            Some(id) if matches!(sym.kind, crate::types::SymbolKind::Variable) => {
                requalify_named_type(arena, id, &prefix)
            }
            _ => None,
        };
        sym.return_type = None;
        sym.param_types.clear();
        sym.generic_params.clear();
    }
    // Alias targets are keyed by the bare type name the extractor recorded
    // (`Result`), but the chain walker looks them up by the receiver's
    // package-qualified head (`@scope/pkg.Result`, derived from a method's
    // qualified return type). Qualify the keys to match. Without this,
    // member lookup THROUGH an external alias (intersection / mapped — e.g.
    // `RenderResult`'s `BoundFunctions` branch) misses, while own-member
    // lookup still works because members key on the already-qualified parent.
    // The names a target REFERS to are prefixed the same way, but only when
    // this file declares them: a mapped type's source (`{ [K in Keys]: V }`)
    // is a sibling type in the package's own surface, while a generic
    // parameter, a literal, or a type imported from another package is not and
    // must stay as written.
    let declared: std::collections::HashSet<String> = pf
        .symbols
        .iter()
        .map(|s| s.qualified_name.clone())
        .collect();
    let qualify = |name: &mut String| {
        if name.is_empty() || name.starts_with(&prefix) || name.starts_with(&globals_prefix) {
            return;
        }
        let candidate = format!("{prefix}{name}");
        if declared.contains(&candidate) {
            *name = candidate;
        }
    };
    for (name, target) in pf.alias_targets.iter_mut() {
        match target {
            crate::types::AliasTarget::Mapped { source, .. }
            | crate::types::AliasTarget::IntersectionMapped { source, .. } => qualify(source),
            _ => {}
        }
        if !name.starts_with(&prefix) && !name.starts_with(&globals_prefix) {
            *name = format!("{prefix}{name}");
        }
    }
}

/// Requalify a declared type whose nominal head carries the bare (pre-prefix)
/// name, minting the package-qualified equivalent: `Class("ApiType")` →
/// `Class("{prefix}ApiType")`, `Apply { ApiType, args }` → the same with a
/// requalified base and the args untouched (bare arg heads resolve by name at
/// walk time). Already-prefixed heads pass through unchanged, keeping the
/// rewrite idempotent. Any other shape (primitive, function, union, …) returns
/// `None` — the caller drops it, matching the pre-prefix clear.
fn requalify_named_type(
    arena: &crate::type_checker::core::types::TypeArena,
    id: crate::type_checker::core::types::TypeId,
    prefix: &str,
) -> Option<crate::type_checker::core::types::TypeId> {
    use crate::type_checker::core::types::Type;
    match arena.get(id) {
        Type::Class(name) => {
            if name.starts_with(prefix) {
                Some(id)
            } else {
                Some(arena.class(&format!("{prefix}{name}")))
            }
        }
        Type::Apply { base, args } => match arena.get(base) {
            Type::Class(name) => {
                if name.starts_with(prefix) {
                    Some(id)
                } else {
                    let new_base = arena.class(&format!("{prefix}{name}"));
                    Some(arena.intern(Type::Apply { base: new_base, args }))
                }
            }
            _ => None,
        },
        // A composite annotation (`const v: A & B`) requalifies arm by arm.
        // Every arm must requalify: a partial rewrite would silently drop the
        // arm that carries the members the walk is looking for.
        Type::Intersection(arms) => requalify_arms(arena, &arms, prefix)
            .map(|arms| arena.intern(Type::Intersection(arms))),
        Type::Union(arms) => {
            requalify_arms(arena, &arms, prefix).map(|arms| arena.intern(Type::Union(arms)))
        }
        // A function-typed value (`declare const make: (opts) => Client<…>`)
        // yields its RETURN when called — the return's head is the name in the
        // package's own surface, so requalify it; a return that can't be
        // (a primitive, a generic param) keeps the annotation as written. The
        // params stay untouched — bare param heads resolve by name at walk
        // time. Never drop the whole capture: the function shape itself is
        // what lets a call root peel through to the return.
        Type::Function { params, return_ } => {
            let new_return = requalify_named_type(arena, return_, prefix).unwrap_or(return_);
            if new_return == return_ {
                Some(id)
            } else {
                Some(arena.intern(Type::Function { params, return_: new_return }))
            }
        }
        _ => None,
    }
}

/// Requalify every arm of a composite type, or `None` if any arm cannot be.
fn requalify_arms(
    arena: &crate::type_checker::core::types::TypeArena,
    arms: &[crate::type_checker::core::types::TypeId],
    prefix: &str,
) -> Option<Vec<crate::type_checker::core::types::TypeId>> {
    let out: Vec<_> = arms
        .iter()
        .filter_map(|&a| requalify_named_type(arena, a, prefix))
        .collect();
    (out.len() == arms.len()).then_some(out)
}
