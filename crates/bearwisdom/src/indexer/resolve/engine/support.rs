// =============================================================================
// indexer/resolve/engine/support — helpers shared by multiple rules
//
// Only code that genuinely repeats across rule files lives here. A helper used
// by a single rule is copied inline into that rule's file instead, so each rule
// reads top-to-bottom without chasing this module. Everything here is a pure
// function of its inputs.
// =============================================================================

use std::borrow::Cow;
use std::collections::BTreeMap;

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{
    FileContext, Symbol, SymbolInfo, SymbolLookup, TypeInfo, RESOLVED_CONFIDENCE,
};
use crate::type_checker::profile::language_profile::{NameNormalization, NormSpec};
use crate::types::EdgeKind;

/// Resolve a module-tagged value `TypeRef` to the type of the value module
/// `module` exports as `key`. A module-tagged value ref is what
/// `typeof import('m')['k']` (or `typeof import('m')`) annotations produce: the
/// ref means "the type of the value exported as `k` from `m`", NOT a type name —
/// so it must never be looked up as a bare type name against the scope.
///
/// The exported value's declaring symbol is reached two ways: a local export
/// rename (`export { local as k }`, captured in `export_alias`) maps `k` to the
/// local declaration's qname; otherwise the direct export `m.k` is the
/// declaration. The declaration's own `field_type` / `field_type_id` is the
/// resolved type (`globalExpect: ExpectStatic` → `ExpectStatic`).
///
/// `typed_qname` is the symbol the ref is being resolved FOR; a declaration that
/// resolves back to it is the self-referential `m.k` case and is declined so the
/// caller falls back rather than typing a value as itself. Returns the resolved
/// `(field_type, field_type_id)`, or `None` when the export or its type can't be
/// resolved.
pub(crate) fn resolve_module_exported_value_type(
    module: &str,
    key: &str,
    typed_qname: &str,
    export_alias: &FxHashMap<String, FxHashMap<String, String>>,
    by_qname: &BTreeMap<String, Symbol>,
    type_info: &FxHashMap<String, TypeInfo>,
) -> Option<(String, Option<crate::type_checker::core::types::TypeId>)> {
    let declaring_qname = export_alias
        .get(module)
        .and_then(|aliases| aliases.get(key))
        .cloned()
        .or_else(|| {
            let direct = format!("{module}.{key}");
            by_qname.contains_key(&direct).then_some(direct)
        })?;
    if declaring_qname == typed_qname {
        return None;
    }
    // A `typeof <value>` whose declared value is a function/method: the value IS
    // that callable, so its type is the function's own qname — calling the typed
    // property then yields the function's return type (the chain walker's
    // callable-named unwrap). A function carries no `field_type` of its own, so
    // the field-type read below would otherwise drop it. A non-callable value's
    // type is its declared field_type.
    if by_qname
        .get(&declaring_qname)
        .is_some_and(|s| matches!(s.kind.as_str(), "function" | "method"))
    {
        return Some((declaring_qname, None));
    }
    let ti = type_info.get(&declaring_qname)?;
    let field_type = ti.field_type.clone()?;
    Some((field_type, ti.field_type_id))
}

/// The workspace package id the file imports `name` from, when the import's
/// module specifier is a bare specifier that resolves to a sibling workspace
/// package. `None` when nothing imports `name`, the binding specifier is
/// relative, or no workspace package declares it.
///
/// Lets a bare reference to a name that exists in several sibling packages bind
/// to the package the use site actually imports it from, instead of a first-wins
/// same-name pick. The caller filters its own candidate set by the returned id.
pub(crate) fn import_scoped_package_id(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    name: &str,
) -> Option<i64> {
    for import in &file_ctx.imports {
        let names_target =
            import.imported_name == name || import.alias.as_deref() == Some(name);
        if !names_target {
            continue;
        }
        let Some(spec) = import.module_path.as_deref() else {
            continue;
        };
        if let Some(pkg) = lookup.workspace_package_id(spec) {
            return Some(pkg);
        }
    }
    None
}

/// `true` when `kind` names a type a `this`/`self` keyword or an inherited
/// member can attach to — a class-like declaration, not a namespace, function,
/// or value.
pub(crate) fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "trait"
            | "object"
            | "record"
            | "protocol"
            | "actor"
            | "mixin"
            | "annotation"
    )
}

/// `true` when `qualified_name` reads as `module_path` (slash / colon /
/// dot-separated) prefix followed by `.` and one or more segments.
pub(crate) fn qname_under_module(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    qualified_name.starts_with(&needle) || qualified_name == dotted
}

/// Stricter form of `qname_under_module`: candidate must sit DIRECTLY under the
/// module — exactly one segment deeper. `Assertions.assertTrue` matches
/// `Assertions`; `Assertions.Nested.foo` does not.
pub(crate) fn qname_directly_under(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    let Some(rest) = qualified_name.strip_prefix(needle.as_str()) else {
        return false;
    };
    !rest.contains('.')
}

/// Minimum score margin the top candidate must beat the runner-up by for
/// `pick_ranked_candidate` to commit; a smaller margin is too ambiguous to guess.
pub(crate) const RANK_MARGIN: i32 = 100;

/// Score a same-name candidate for import-scoped selection — higher is better.
/// Reads only the candidate row and the use site's file context (its package,
/// imports, path): same workspace package +1000, an import naming the candidate's
/// package +500 or its namespace +300, ambient +200, shared path prefix
/// +10/segment, an external-depth penalty, and a visibility hint.
pub(crate) fn score_candidate(
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    sym: &Symbol,
) -> i32 {
    let mut s: i32 = 0;
    if let (Some(caller_pkg), Some(sym_pkg)) = (file_package_id, sym.package_id) {
        if caller_pkg == sym_pkg {
            s += 1000;
        }
    }
    for import in &file_ctx.imports {
        let Some(mod_path) = import.module_path.as_deref() else {
            continue;
        };
        if let Some(wp_id) = lookup.workspace_package_id(mod_path) {
            if Some(wp_id) == sym.package_id {
                s += 500;
            }
        }
        if qname_under_module(&sym.qualified_name, mod_path) {
            s += 300;
        }
    }
    if lookup.is_ambient_path(&sym.file_path) {
        s += 200;
    }
    s += path_proximity_score(&file_ctx.file_path, &sym.file_path);
    if lookup.is_external_file(&sym.file_path) {
        let depth = sym.file_path.matches('/').count() as i32;
        s -= depth.min(20);
    }
    match sym.visibility.as_deref() {
        Some("public") => s += 50,
        Some("private") => s -= 200,
        _ => {}
    }
    s
}

/// The shared import-scoped candidate chokepoint. Returns the top scorer when it
/// beats the runner-up by `RANK_MARGIN`, the sole candidate when there is one, or
/// `None` when the field is empty or too ambiguous to commit (the caller keeps
/// its own fallback). Deterministic: equal scores break by ascending id, so the
/// pick never depends on candidate insertion order.
pub(crate) fn pick_ranked_candidate<'a>(
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    candidates: &[&'a Symbol],
) -> Option<&'a Symbol> {
    match candidates.len() {
        0 => return None,
        1 => return Some(candidates[0]),
        _ => {}
    }
    let mut scored: Vec<(i32, &'a Symbol)> = candidates
        .iter()
        .map(|sym| (score_candidate(file_ctx, file_package_id, lookup, sym), *sym))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    let (top_score, top) = scored[0];
    let (runner_score, _) = scored[1];
    if top_score - runner_score < RANK_MARGIN {
        return None;
    }
    Some(top)
}

/// Shared-directory-prefix score between two file paths: 10 × the number of
/// shared leading directory segments. Separators normalise to `/`; the filename
/// is dropped before comparing.
fn path_proximity_score(caller_path: &str, candidate_path: &str) -> i32 {
    let caller_norm = caller_path.replace('\\', "/");
    let candidate_norm = candidate_path.replace('\\', "/");
    let caller_dir = caller_norm.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let candidate_dir = candidate_norm.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let caller_segs: Vec<&str> = caller_dir.split('/').filter(|s| !s.is_empty()).collect();
    let candidate_segs: Vec<&str> =
        candidate_dir.split('/').filter(|s| !s.is_empty()).collect();
    caller_segs
        .iter()
        .zip(candidate_segs.iter())
        .take_while(|(a, b)| a == b)
        .count() as i32
        * 10
}

/// The full directory portion of a file path (everything before the final
/// segment). Path separators are normalized to `/`. Returns `None` for a bare
/// filename. For `schema/users/model.prisma` returns `Some("schema/users")`.
pub(crate) fn parent_dir(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    normalized.rsplit_once('/').map(|(dir, _)| dir.to_string())
}

/// The file's basename-stem equals the module (case-insensitive on both
/// inputs). A basename with no extension matches whole. Does NOT consider
/// directory segments.
pub(crate) fn basename_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    if module_lower.is_empty() {
        return false;
    }
    let normalized = file_path_lower.replace('\\', "/");
    let Some(basename) = normalized.rsplit('/').next() else {
        return false;
    };
    match basename.rsplit_once('.') {
        Some((stem, _ext)) => stem == module_lower,
        None => basename == module_lower,
    }
}

/// File path's basename stem or any path segment matches the module
/// (case-insensitive on both inputs). External `ext:<lang>:<pkg>` paths match on
/// the trailing colon-delimited component.
pub(crate) fn path_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    if basename_stem_matches(file_path_lower, module_lower) {
        return true;
    }
    let normalized = file_path_lower.replace('\\', "/");
    normalized.split('/').any(|seg| {
        seg == module_lower
            || seg
                .split(':')
                .next_back()
                .map_or(false, |tail| tail == module_lower)
    })
}

/// Trim a source-file extension off a module/path string for stem comparison.
pub(crate) fn trim_source_extension(path: &str) -> &str {
    path.trim_end_matches(".svelte")
        .trim_end_matches(".vue")
        .trim_end_matches(".tsx")
        .trim_end_matches(".jsx")
        .trim_end_matches(".mts")
        .trim_end_matches(".cts")
        .trim_end_matches(".ts")
        .trim_end_matches(".js")
        .trim_end_matches(".cs")
        .trim_end_matches(".cljc")
        .trim_end_matches(".cljs")
        .trim_end_matches(".clj")
        .trim_end_matches(".astro")
        .trim_end_matches(".mdx")
}

/// Strip a leading `{kw}.` from `target` when `kw` is one of `self_keywords`
/// (Python `self.method` → `method`). Only the first matching keyword strips,
/// and only when followed by `.`. An empty `self_keywords` slice returns
/// `target` unchanged.
pub(crate) fn strip_self_keyword<'t>(target: &'t str, self_keywords: &[&str]) -> &'t str {
    for kw in self_keywords {
        if let Some(rest) = target.strip_prefix(kw) {
            if let Some(after) = rest.strip_prefix('.') {
                return after;
            }
        }
    }
    target
}

/// Normalize a name for the bare-name binding comparison. Applied identically to
/// a candidate's name and the ref's target before they are compared.
///
/// `NameNormalization::None` is the identity transform and borrows unchanged. A
/// `Spec` whose deltas are all off is also identity and borrows. Otherwise the
/// transform runs in a fixed order: strip a wrapping sigil pair, strip the first
/// matching leading prefix, remove the configured characters anywhere, then fold
/// ASCII case.
pub(crate) fn normalize_name(norm: NameNormalization, s: &str) -> Cow<'_, str> {
    let spec = match norm {
        NameNormalization::None => return Cow::Borrowed(s),
        NameNormalization::Spec(spec) => spec,
    };
    if is_identity_spec(&spec) {
        return Cow::Borrowed(s);
    }

    let mut cur = s;

    // 1. Sigil wrapper: when the name both starts with `prefix` and ends with
    //    `suffix`, drop both. The first matching pair wins.
    for (prefix, suffix) in spec.strip_sigils {
        if let Some(inner) = cur.strip_prefix(*prefix) {
            if let Some(inner) = inner.strip_suffix(*suffix) {
                cur = inner;
                break;
            }
        }
    }

    // 2. Leading prefix: drop the first declared prefix that matches. A prefix
    //    runs before the case-fold step, so when the spec folds case the prefix
    //    match folds too; a case-sensitive spec keeps the exact byte match.
    for prefix in spec.strip_prefixes {
        let matched_len = if spec.case_insensitive {
            cur.get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| prefix.len())
        } else {
            cur.starts_with(*prefix).then_some(prefix.len())
        };
        if let Some(len) = matched_len {
            cur = &cur[len..];
            break;
        }
    }

    // 3 & 4. Remove the configured characters anywhere and fold case. Both need
    //        an owned buffer; build it once.
    let needs_char_strip = !spec.strip_chars.is_empty();
    if !needs_char_strip && !spec.case_insensitive {
        return Cow::Borrowed(cur);
    }
    let mut out = String::with_capacity(cur.len());
    for ch in cur.chars() {
        if needs_char_strip && spec.strip_chars.contains(&ch) {
            continue;
        }
        if spec.case_insensitive {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

/// A `NormSpec` whose every field is the default (no sigils, no prefixes, no
/// chars, case-sensitive) is the identity transform — `normalize_name` borrows
/// rather than allocating for it.
pub(crate) fn is_identity_spec(spec: &NormSpec) -> bool {
    !spec.case_insensitive
        && spec.strip_chars.is_empty()
        && spec.strip_prefixes.is_empty()
        && spec.strip_sigils.is_empty()
}

/// Walk re-export chains from `module_path` to the module that defines
/// `target_name`, returning the declaring symbol when found. A `module_path` is
/// a resolved file path (or a workspace-package entry whose `reexports_from`
/// fallback resolves it). Both `export { X } from '...'` (named) and `export *
/// from '...'` (wildcard) hops are followed; the wildcard sweep runs after the
/// named entries so a more-specific named re-export wins. `kind_compatible`
/// gates a candidate by the ref's edge kind. Bounded at `MAX_DEPTH` hops.
pub(crate) fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
) -> Option<SymbolInfo> {
    const MAX_DEPTH: u32 = 5;
    if depth >= MAX_DEPTH {
        return None;
    }

    let reexports = lookup.reexports_from(module_path);
    if reexports.is_empty() {
        return None;
    }

    let mut wildcard_sources: Vec<&str> = Vec::new();

    for (exported_name, source_module) in reexports {
        // A bare source module is followable when it resolves to a file OR names
        // a sibling workspace package whose barrel can be recovered. An
        // unresolvable bare source (a true external) is skipped.
        if !is_relative_specifier(source_module)
            && lookup.resolve_module_from(module_path, source_module).is_none()
            && workspace_pkg_barrels(lookup, source_module).is_empty()
        {
            continue;
        }

        if exported_name == "*" {
            wildcard_sources.push(source_module.as_str());
            continue;
        }

        if exported_name != target_name {
            continue;
        }

        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                return Some(reexport_resolution(sym.id, "reexport_chain"));
            }
        }
        if is_relative_specifier(source_module) {
            if let Some(res) = resolve_relative_reexport(
                lookup,
                module_path,
                source_module,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_chain",
            ) {
                return Some(res);
            }
        } else if let Some(res) = resolve_reexport_by_matching_file(
            lookup,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            "reexport_chain",
        ) {
            return Some(res);
        }

        if let Some(res) = follow_reexport_source(
            module_path,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth,
        ) {
            return Some(res);
        }
    }

    for source_module in wildcard_sources {
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                return Some(reexport_resolution(sym.id, "reexport_star"));
            }
        }
        if is_relative_specifier(source_module) {
            if let Some(res) = resolve_relative_reexport(
                lookup,
                module_path,
                source_module,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_star",
            ) {
                return Some(res);
            }
        } else if let Some(res) = resolve_reexport_by_matching_file(
            lookup,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            "reexport_star",
        ) {
            return Some(res);
        }

        if let Some(res) = follow_reexport_source(
            module_path,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth,
        ) {
            return Some(res);
        }
    }

    None
}

/// Recurse `follow_reexports` into a re-export's source module. A relative or
/// resolvable source recurses on its resolved file path; a bare workspace-package
/// source recurses on each of the package's re-exporting `index` barrels (the
/// bare specifier has no `resolve_module_from` mapping, so the barrel is
/// recovered from the package's own symbol set).
#[allow(clippy::too_many_arguments)]
fn follow_reexport_source(
    module_path: &str,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
) -> Option<SymbolInfo> {
    if let Some(next) = lookup.resolve_module_from(module_path, source_module) {
        let next = next.to_string();
        return follow_reexports(
            &next,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
        );
    }
    if !is_relative_specifier(source_module) {
        for barrel in workspace_pkg_barrels(lookup, source_module) {
            if let Some(res) = follow_reexports(
                &barrel,
                target_name,
                edge_kind,
                kind_compatible,
                lookup,
                depth + 1,
            ) {
                return Some(res);
            }
        }
        return None;
    }
    // A relative source with no `resolve_module_from` mapping: recurse on each
    // candidate file the joined-and-normalized path could name, so a multi-hop
    // relative re-export chain (`./a` re-exports from `./b`) still threads.
    for next in relative_reexport_candidates(lookup, module_path, source_module) {
        if let Some(res) = follow_reexports(
            &next,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
        ) {
            return Some(res);
        }
    }
    None
}

/// Resolve a relative re-export source to its declaring symbol by file path. The
/// source is joined against the barrel file's directory and normalized; a
/// `by_name(target_name)` candidate whose file path matches that base (with a
/// source extension or an `/index` entry appended) is the declaration. Used when
/// the per-source module-to-file map carries no mapping for the relative
/// specifier, so `in_module_from` returned nothing.
#[allow(clippy::too_many_arguments)]
fn resolve_relative_reexport(
    lookup: &dyn SymbolLookup,
    barrel_path: &str,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    strategy: &'static str,
) -> Option<SymbolInfo> {
    let base = relative_base(barrel_path, source_module)?;
    for sym in lookup.by_name(target_name) {
        if sym.name != target_name || !kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        if relative_file_matches_base(&sym.file_path, &base) {
            return Some(reexport_resolution(sym.id, strategy));
        }
    }
    None
}

/// The candidate file paths a relative re-export source could name — the joined,
/// normalized base with each source extension and `/index` entry appended,
/// filtered to those actually indexed (i.e. that have non-empty
/// `reexports_from`, the only files `follow_reexports` can recurse into). Empty
/// when the base can't be formed.
pub(crate) fn relative_reexport_candidates(
    lookup: &dyn SymbolLookup,
    barrel_path: &str,
    source_module: &str,
) -> Vec<String> {
    let Some(base) = relative_base(barrel_path, source_module) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for cand in extension_candidates(&base) {
        if !lookup.reexports_from(&cand).is_empty() {
            out.push(cand);
        }
    }
    out
}

/// Join a relative `source_module` against `barrel_path`'s directory and collapse
/// `.`/`..` segments. Returns the extension-less base (`packages/q/src/index` +
/// `./queryClient` → `packages/q/src/queryClient`). `None` when `barrel_path` has
/// no directory portion.
fn relative_base(barrel_path: &str, source_module: &str) -> Option<String> {
    let normalized = barrel_path.replace('\\', "/");
    let dir = normalized.rsplit_once('/').map(|(d, _)| d)?;
    let joined = format!("{dir}/{}", source_module.replace('\\', "/"));
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Some(out.join("/"))
}

/// The source-file path candidates for an extension-less `base`: the base with
/// each source extension appended, plus the `base/index.<ext>` directory entry.
fn extension_candidates(base: &str) -> Vec<String> {
    const EXTS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "mts", "cts", "cjs", "svelte", "astro", "vue",
    ];
    let mut out: Vec<String> = Vec::with_capacity(EXTS.len() * 2);
    for ext in EXTS {
        out.push(format!("{base}.{ext}"));
    }
    for ext in EXTS {
        out.push(format!("{base}/index.{ext}"));
    }
    out
}

/// `true` when `file_path` is one of the source-extension / `/index` forms of
/// `base`. Both inputs are forward-slash normalized; the file matches when it
/// equals a candidate or ends with `/<candidate>` (a suffix match, so a
/// project-relative base resolves against a deeper indexed path).
fn relative_file_matches_base(file_path: &str, base: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    extension_candidates(base)
        .iter()
        .any(|cand| normalized == *cand || normalized.ends_with(&format!("/{cand}")))
}

/// The re-exporting `index` barrels of the workspace package a bare `specifier`
/// names — the files in that package whose basename stem is `index` and whose
/// `reexports_from` is non-empty. Empty when `specifier` is not a workspace
/// package or the package has no re-exporting barrel.
pub(crate) fn workspace_pkg_barrels(lookup: &dyn SymbolLookup, specifier: &str) -> Vec<String> {
    let Some(pkg_id) = lookup.workspace_package_id(specifier) else {
        return Vec::new();
    };
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    let mut barrels: Vec<String> = Vec::new();
    for sym in lookup.symbols_in_package(pkg_id) {
        let path = sym.file_path.as_ref();
        if !seen.insert(path.to_string()) {
            continue;
        }
        if !path_basename_stem_is_index(path) {
            continue;
        }
        if lookup.reexports_from(path).is_empty() {
            continue;
        }
        barrels.push(path.to_string());
    }
    barrels
}

/// `true` when the file's basename stem is `index` — the conventional package
/// barrel (`.../src/index.ts`, `index.js`, `index.tsx`, …).
fn path_basename_stem_is_index(file_path: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    let Some(basename) = normalized.rsplit('/').next() else {
        return false;
    };
    basename.split('.').next().unwrap_or(basename) == "index"
}

/// A resolved-via-re-export `SymbolInfo` tagged with the hop strategy.
fn reexport_resolution(id: i64, strategy: &'static str) -> SymbolInfo {
    SymbolInfo {
        target_symbol_id: id,
        confidence: RESOLVED_CONFIDENCE,
        strategy,
        resolved_yield_type: None,
        flow_emit: None,
    }
}

/// Match a re-export's bare source module to its declaring symbol by file path
/// when the source module isn't a resolvable file. Accepts only when exactly one
/// by-name candidate's file path matches the module — an ambiguous match is no
/// match. Handles Nim-style `std/`/`pkg/` prefixes and `.nim` extensions.
fn resolve_reexport_by_matching_file(
    lookup: &dyn SymbolLookup,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    strategy: &'static str,
) -> Option<SymbolInfo> {
    let mut matches = lookup.by_name(target_name).into_iter().filter(|sym| {
        sym.name == target_name
            && kind_compatible(edge_kind, &sym.kind)
            && reexport_file_path_matches_module(&sym.file_path, source_module)
    });
    let first = matches.next()?;
    let first_file = first.file_path.as_ref();
    if matches.any(|sym| sym.file_path.as_ref() != first_file) {
        return None;
    }
    Some(reexport_resolution(first.id, strategy))
}

/// File-path matcher for re-export module resolution. Handles Nim-style
/// `std/`/`pkg/` prefixes and `.nim` extension matching.
fn reexport_file_path_matches_module(file_path: &str, source_module: &str) -> bool {
    let trimmed = source_module.trim_matches('"').trim_matches('\'').trim();
    let stripped = trimmed
        .strip_prefix("std/")
        .or_else(|| trimmed.strip_prefix("pkg/"))
        .unwrap_or(trimmed)
        .replace('\\', "/");
    let module = stripped.trim_matches('/');
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let candidates = if module.ends_with(".nim") {
        vec![module.to_string()]
    } else {
        vec![format!("{module}.nim"), format!("{module}/mod.nim")]
    };
    candidates
        .iter()
        .any(|candidate| normalized == *candidate || normalized.ends_with(&format!("/{candidate}")))
}

/// A specifier is relative — and therefore project-internal — when it starts
/// with `.`, `/`, or is a Windows drive path.
pub(crate) fn is_relative_specifier(s: &str) -> bool {
    s.starts_with('.') || s.starts_with('/') || (s.len() >= 2 && s.as_bytes()[1] == b':')
}

#[cfg(test)]
#[path = "support_tests.rs"]
mod tests;
