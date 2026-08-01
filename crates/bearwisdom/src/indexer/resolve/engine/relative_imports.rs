// =============================================================================
// engine/relative_imports — following a declaration file's own relative imports
//
// An external `.d.ts` can name a supertype declared in a SIBLING file of the
// same package, reached by a relative specifier. The module-location index keys
// on package specifiers, so it cannot place an intra-package relative path —
// but the on-disk path resolves directly. This module turns those specifiers
// into files, scoped to the imports whose bound name feeds a supertype clause.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::types::EdgeKind;

/// Follow a materialized external file's RELATIVE imports that bring in a type
/// the file `extends`/`implements`, resolving the specifier against the importing
/// file's own directory and pulling the target into the closure.
///
/// A package's member-declaring interface often lives in a sibling module its
/// export map never named — reached only through a shell file's `import { S }
/// from './sub'`, where `interface I extends S`. The `(module, name)` location
/// index keys on package specifiers, so it cannot place an intra-package relative
/// path — but the on-disk path resolves directly.
///
/// Scoped to imports whose bound name feeds a supertype clause: those carry the
/// members a receiver's supertype climb needs. A value-only relative import is
/// NOT followed — chasing every relative specifier drags a package's whole
/// sibling `.d.ts` tree into the closure (a qname-collision and slowdown source).
pub(crate) fn collect_relative_supertype_imports(
    importer: &Path,
    refs: &[crate::types::ExtractedRef],
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let Some(dir) = importer.parent() else {
        return;
    };
    // The supertypes this file's declarations extend/implement, by head name.
    let inherited: HashSet<&str> = refs
        .iter()
        .filter(|r| matches!(r.kind, EdgeKind::Inherits | EdgeKind::Implements))
        .map(|r| supertype_head(&r.target_name))
        .collect();
    if inherited.is_empty() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(importer) else {
        return;
    };
    for (spec, names) in relative_named_imports(&content) {
        if !names.iter().any(|n| inherited.contains(n.as_str())) {
            continue;
        }
        if let Some(file) = resolve_relative_ts_module(dir, &spec) {
            if seen.insert(file.clone()) {
                out.push(file);
            }
        }
    }
}

/// The head of a supertype reference: `Base` for `Base<X, Y>` — the generic
/// args don't name the symbol.
pub(crate) fn supertype_head(target: &str) -> &str {
    target.split('<').next().unwrap_or(target).trim()
}

/// Parse `content`'s `import { … } from '<relative-spec>'` statements into
/// `(spec, imported-names)` pairs, keeping only relative specifiers. Each name in
/// a `{ … }` group is reduced to the local binding: a `type ` modifier and an
/// `as <alias>` rename are stripped. Default and namespace imports carry no
/// brace group and are skipped — a supertype is referenced by a named binding.
pub(crate) fn relative_named_imports(content: &str) -> Vec<(String, Vec<String>)> {
    use crate::ecosystem::npm::extract_quoted_after;
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if !(t.starts_with("import ") || t.starts_with("export ") || t.starts_with("import\t")) {
            continue;
        }
        let Some(spec) = extract_quoted_after(t, " from ") else {
            continue;
        };
        if !spec.starts_with('.') {
            continue;
        }
        let Some(open) = t.find('{') else { continue };
        let Some(close) = t[open..].find('}') else { continue };
        let names: Vec<String> = t[open + 1..open + close]
            .split(',')
            .filter_map(|part| {
                let p = part.trim().strip_prefix("type ").unwrap_or(part.trim()).trim();
                // The LOCAL binding (after `as`) is what an `extends` clause names.
                let local = p.rsplit(" as ").next().unwrap_or(p).trim();
                (!local.is_empty()).then(|| local.to_string())
            })
            .collect();
        if !names.is_empty() {
            out.push((spec.to_string(), names));
        }
    }
    out
}

/// Resolve a relative TS/JS module specifier against `dir`, trying the
/// declaration-first extension order a `.d.ts`-shipping package uses. A spec may
/// carry an ESM `.js`/`.mjs` extension that actually names a `.d.ts` sibling, so
/// the bare stem is probed first; a directory spec resolves to its `index`.
pub(crate) fn resolve_relative_ts_module(dir: &Path, spec: &str) -> Option<PathBuf> {
    const EXTS: &[&str] = &[".d.ts", ".ts", ".tsx", ".d.mts", ".mts", ".d.cts"];
    // Strip a trailing ESM extension so `./sub.js` probes `./sub.d.ts`.
    let stem = spec
        .strip_suffix(".js")
        .or_else(|| spec.strip_suffix(".mjs"))
        .or_else(|| spec.strip_suffix(".cjs"))
        .unwrap_or(spec);
    // Drop the leading `./` so the joined path stays `dir/sub`, not `dir/./sub`
    // (which would leak `/./` into the virtual path).
    let stem = stem.strip_prefix("./").unwrap_or(stem);
    let base = dir.join(stem);
    for ext in EXTS {
        let cand = append_ext(&base, ext);
        if cand.is_file() {
            return Some(cand);
        }
    }
    // The spec already named a concrete file (`./types.d.ts`).
    let direct = dir.join(spec);
    if direct.is_file() {
        return Some(direct);
    }
    // Directory index module.
    for ext in EXTS {
        let cand = base.join(format!("index{ext}"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// `path` with `ext` (a leading-dot extension) appended to its final component —
/// `dir/sub` + `.d.ts` → `dir/sub.d.ts`. Unlike `Path::with_extension`, this
/// never replaces an existing dotted suffix in the stem.
fn append_ext(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(ext);
    PathBuf::from(s)
}

#[cfg(test)]
#[path = "relative_imports_tests.rs"]
mod tests;
