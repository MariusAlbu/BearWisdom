// =============================================================================
// engine/import_qualify — bind bare annotation heads to the declaring file's
// imports.
//
// A type annotation is lexically scoped: `getClient(): Client` names whatever
// `Client` the declaring file imports, not a same-named global. Extractors
// intern annotation tokens as written, so the compiled TypeId carries a bare
// head that a global namesake (a TS lib `Client`, a stdlib `Response`) can
// capture during the walk. This pass rewrites those heads to the import's
// indexed qname (`@pkg.Client`) once that declaration is materialized.
//
// Deferred across `ingest` calls: internal files compile before the externals
// their imports name are materialized, so a candidate absent in one batch is
// retried after the next. Only the id-keyed type slots are rewritten — the
// qname-keyed slot is a cross-file first-winner that a foreign file's imports
// must never mutate.
// =============================================================================

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{EdgeKind, ParsedFile};

use super::contract::util::is_type_like_kind;
use super::contract::{Symbol, TypeInfo};

/// One file's deferred requalification work: the import-derived candidate map
/// and the ids of the symbols whose type slots it governs.
pub(crate) struct PendingFile {
    /// The declaring file's path — consulted for local type shadows.
    pub(crate) path: String,
    /// Locally bound name → indexed-qname candidates, best first.
    pub(crate) imports: FxHashMap<String, Vec<String>>,
    /// Ids of this file's symbols (the `type_info_by_id` keys to rewrite).
    pub(crate) symbol_ids: Vec<i64>,
}

/// Build a file's pending entry from its import-describing refs. `None` when
/// the file has no non-relative named imports (nothing to requalify against).
pub(crate) fn collect_pending(
    pf: &ParsedFile,
    symbol_id_map: &crate::indexer::write::SymbolIds,
) -> Option<PendingFile> {
    let mut imports: FxHashMap<String, Vec<String>> = FxHashMap::default();
    for r in &pf.refs {
        if !(r.is_import_binding || r.kind == EdgeKind::Imports) {
            continue;
        }
        let Some(module) = r.module.as_deref() else {
            continue;
        };
        if module.starts_with('.') || r.target_name == "*" {
            continue;
        }
        // A rename import carries the module's ORIGINAL declared name as a
        // single-segment chain; the candidate is built from the original,
        // keyed by the locally bound name.
        let original = r
            .chain
            .as_ref()
            .and_then(|c| match c.segments.as_slice() {
                [seg] if seg.name != r.target_name => Some(seg.name.as_str()),
                _ => None,
            })
            .unwrap_or(&r.target_name);
        let mut cands = vec![format!("{module}.{original}")];
        let root = package_root(module);
        if root != module {
            cands.push(format!("{root}.{original}"));
        }
        imports.entry(r.target_name.clone()).or_insert(cands);
    }
    if imports.is_empty() {
        return None;
    }
    let symbol_ids: Vec<i64> = pf
        .symbols
        .iter()
        .enumerate()
        .filter_map(|(i, s)| symbol_id_map.id_of(&pf.path, i, &s.qualified_name))
        .collect();
    Some(PendingFile { path: pf.path.clone(), imports, symbol_ids })
}

/// Apply every pending entry whose candidates are now materialized, retaining
/// the rest for the next ingest. A locally-declared type of the same name
/// shadows the import permanently (its entry is dropped unresolved).
pub(crate) fn apply_pending(
    pending: &mut Vec<PendingFile>,
    arena: &TypeArena,
    by_qname: &BTreeMap<String, Symbol>,
    by_file: &FxHashMap<String, Vec<Symbol>>,
    type_info_by_id: &mut FxHashMap<i64, TypeInfo>,
) {
    pending.retain_mut(|entry| {
        let mut resolved: FxHashMap<String, String> = FxHashMap::default();
        let file_syms = by_file.get(&entry.path);
        entry.imports.retain(|local, cands| {
            let shadowed = file_syms.is_some_and(|syms| {
                syms.iter().any(|s| s.name == *local && is_type_like_kind(&s.kind))
            });
            if shadowed {
                return false;
            }
            let hit = cands.iter().find(|c| {
                by_qname.get(*c).is_some_and(|s| is_type_like_kind(&s.kind))
            });
            match hit {
                Some(q) => {
                    resolved.insert(local.clone(), q.clone());
                    false
                }
                None => true,
            }
        });
        if !resolved.is_empty() {
            for id in &entry.symbol_ids {
                let Some(ti) = type_info_by_id.get_mut(id) else {
                    continue;
                };
                if let Some(new) = ti.return_type_id.and_then(|t| requalify_type(arena, t, &resolved)) {
                    ti.return_type_id = Some(new);
                }
                if let Some(new) = ti.field_type_id.and_then(|t| requalify_type(arena, t, &resolved)) {
                    ti.field_type_id = Some(new);
                }
            }
        }
        !entry.imports.is_empty()
    });
}

/// Rewrite every bare nominal head in `ty` that `resolve` maps, recursing
/// through applications, wrappers, composites, and function signatures.
/// `None` when nothing changed (the caller keeps the original id).
fn requalify_type(
    arena: &TypeArena,
    ty: TypeId,
    resolve: &FxHashMap<String, String>,
) -> Option<TypeId> {
    match arena.get(ty) {
        Type::Class(name) => {
            if name.contains('.') || name.contains("::") {
                return None;
            }
            resolve.get(&name).map(|q| arena.class(q))
        }
        // A bound nominal is not a bare name; requalification never touches it.
        Type::Decl { .. } => None,
        Type::Apply { base, args } => {
            let new_base = requalify_type(arena, base, resolve);
            let new_args: Vec<Option<TypeId>> =
                args.iter().map(|&a| requalify_type(arena, a, resolve)).collect();
            if new_base.is_none() && new_args.iter().all(Option::is_none) {
                return None;
            }
            let args = args
                .iter()
                .zip(&new_args)
                .map(|(&old, new)| new.unwrap_or(old))
                .collect();
            Some(arena.intern(Type::Apply { base: new_base.unwrap_or(base), args }))
        }
        Type::Optional(inner) => {
            requalify_type(arena, inner, resolve).map(|i| arena.intern(Type::Optional(i)))
        }
        Type::AsyncWrapper(inner) => {
            requalify_type(arena, inner, resolve).map(|i| arena.intern(Type::AsyncWrapper(i)))
        }
        Type::Iterator(inner) => {
            requalify_type(arena, inner, resolve).map(|i| arena.intern(Type::Iterator(i)))
        }
        Type::Constructor(inner) => {
            requalify_type(arena, inner, resolve).map(|i| arena.intern(Type::Constructor(i)))
        }
        Type::Union(arms) => {
            requalify_arms(arena, &arms, resolve).map(|arms| arena.intern(Type::Union(arms)))
        }
        Type::Intersection(arms) => {
            requalify_arms(arena, &arms, resolve)
                .map(|arms| arena.intern(Type::Intersection(arms)))
        }
        Type::Tuple(items) => {
            requalify_arms(arena, &items, resolve).map(|items| arena.intern(Type::Tuple(items)))
        }
        Type::Function { params, return_ } => {
            let new_params: Vec<Option<TypeId>> =
                params.iter().map(|&p| requalify_type(arena, p, resolve)).collect();
            let new_return = requalify_type(arena, return_, resolve);
            if new_return.is_none() && new_params.iter().all(Option::is_none) {
                return None;
            }
            let params = params
                .iter()
                .zip(&new_params)
                .map(|(&old, new)| new.unwrap_or(old))
                .collect();
            Some(arena.intern(Type::Function {
                params,
                return_: new_return.unwrap_or(return_),
            }))
        }
        Type::Primitive(_) | Type::Generic { .. } | Type::Literal(_) | Type::Unknown => None,
    }
}

/// Rewrite a composite's arms independently; `None` when every arm is
/// unchanged.
fn requalify_arms(
    arena: &TypeArena,
    arms: &[TypeId],
    resolve: &FxHashMap<String, String>,
) -> Option<Vec<TypeId>> {
    let rewritten: Vec<Option<TypeId>> =
        arms.iter().map(|&a| requalify_type(arena, a, resolve)).collect();
    if rewritten.iter().all(Option::is_none) {
        return None;
    }
    Some(
        arms.iter()
            .zip(&rewritten)
            .map(|(&old, new)| new.unwrap_or(old))
            .collect(),
    )
}

/// The package root of a module specifier: the first path segment, or the
/// first two for a scoped npm package (`@scope/pkg/sub` → `@scope/pkg`).
fn package_root(module: &str) -> &str {
    if let Some(rest) = module.strip_prefix('@') {
        match rest.match_indices('/').nth(1) {
            Some((i, _)) => &module[..i + 1],
            None => module,
        }
    } else {
        module.split('/').next().unwrap_or(module)
    }
}

#[cfg(test)]
#[path = "import_qualify_tests.rs"]
mod tests;
