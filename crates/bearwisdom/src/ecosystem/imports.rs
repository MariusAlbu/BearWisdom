// =============================================================================
// ecosystem/imports.rs — shared import-resolution layer
//
// One layer to canonicalize refs against import statements, called by every
// language extractor whose ecosystem has aliasing semantics (npm covers
// TS/TSX/JS/JSX/MJS/CJS + embedded scripts in Vue/Svelte/Astro/Angular;
// pub covers Dart). The shape of an import statement differs per language
// but the *resolution rules* — namespace prefix unwrap, renamed alias
// substitution, default vs named distinction — are universal within an
// ecosystem.
//
// Without this layer each language extractor reinvents the post-pass and
// loses information differently. With it, every ECMAScript-family file in
// the index has the same canonical ref shape going into demand-seed and
// the resolver.
//
// Contract: `ExtractedRef::target_name` is **the canonical exported name
// in the resolved module** — not the local alias from the importing file.
// `ExtractedRef::module` is **the resolved final module path** (relative
// or bare specifier). `ExtractedRef::namespace_segments` carries the
// intermediate qualifier path from `Foo.A.B` → namespace_segments=["A"],
// target_name="B", module=<Foo's import>.
// =============================================================================

use std::collections::HashMap;

use crate::types::{EdgeKind, ExtractedRef};

/// Per-import metadata. Built by each language's extractor by walking
/// import statements in its own grammar; consumed by the shared
/// resolver below.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// Local name as written in the importing file (`Foo` in
    /// `import * as Foo from 'pkg'`, `bar` in `import { foo as bar }
    /// from 'pkg'`).
    pub local_name: String,
    /// Resolved module specifier as the source code wrote it (`pkg`,
    /// `./foo`, `@scope/pkg/sub`). The resolver downstream may rewrite
    /// relative paths to absolute file paths; here we keep the raw
    /// specifier so the resolver chooses its own canonicalization.
    pub module: String,
    pub kind: ImportKind,
}

/// Discriminates the import shape so the rewrite picks the right
/// substitution semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// `import X from 'pkg'` — `X` is the local alias for the module's
    /// default export. ECMAScript-only; other ecosystems treat default
    /// imports as bare specifiers.
    Default,
    /// `import { X } from 'pkg'` — bare named import; local name == X.
    /// `import { X as Y } from 'pkg'` — renamed; local name = Y,
    /// `exported_name` = X. Resolver substitutes Y refs back to X so
    /// they match the source module's export list.
    Named { exported_name: String },
    /// `import * as Foo from 'pkg'` (ECMAScript) /
    /// `import 'foo.dart' as foo;` (Dart) — `Foo` is a synthetic
    /// namespace; `Foo.X` is qualified access into the module.
    Namespace,
    /// `import 'pkg';` — side-effect only, no local name. Carried in
    /// the map for completeness; never matches a ref.
    SideEffect,
}

/// Apply ecosystem-uniform import semantics to a ref vec, in-place.
///
/// Idempotent — running it twice produces the same result, because
/// after the first run all renamed aliases are substituted, all
/// namespace prefixes are split off, and the only refs left to process
/// are bare identifiers or already-canonicalized refs whose
/// `target_name` doesn't match any import's `local_name`.
///
/// For each ref, the algorithm:
///
/// 1. Looks at the leftmost identifier touched by the ref. For chain
///    refs that's `chain.segments[0].name`; for plain refs that's
///    either `target_name` itself or its first dot-segment.
/// 2. Looks up that name in `imports`.
/// 3. Rewrites the ref according to the import's `kind`:
///    - **Default**: set `module`, leave `target_name` (the default
///      export's "name" on the imported side is conventionally
///      `default`, but most resolvers treat the local name as the
///      symbol — leave that alone and let downstream lookup match by
///      module + local name).
///    - **Named**: set `module`. If the named import was renamed
///      (`exported_name` differs from `local_name`), substitute
///      `target_name` so it matches the source module's export.
///    - **Namespace**: set `module`. Split `target_name` at the first
///      dot — leftmost segment was the alias and is now absorbed; the
///      remainder becomes `namespace_segments` + new `target_name` (the
///      leaf).
///    - **SideEffect**: skipped.
///
/// Refs whose first segment doesn't match any import are left alone —
/// they're either local symbols, ambient globals, or names from
/// somewhere the import system can't see (typeof references to other
/// files via tsconfig, build-system magic, etc.).
pub fn resolve_import_refs(
    refs: &mut Vec<ExtractedRef>,
    imports: &HashMap<String, ImportEntry>,
) {
    if imports.is_empty() {
        return;
    }
    for r in refs.iter_mut() {
        // Skip imports refs themselves — they describe the import, not a
        // use of an imported name.
        if r.kind == EdgeKind::Imports {
            continue;
        }
        // Already canonicalized: respect the prior decision.
        if r.module.is_some() {
            continue;
        }
        rewrite_one(r, imports);
    }
}

fn rewrite_one(r: &mut ExtractedRef, imports: &HashMap<String, ImportEntry>) {
    // Three distinct ref shapes go through here:
    //
    //   (a) Multi-segment chain refs (`obj.method()` with chain.len ≥ 2)
    //       → safe to set module from chain[0], rewrite chain[0] for
    //         renamed imports.
    //
    //   (b) Qualified target_name (`Foo.Bar` with no chain)
    //       → split on `.`, recover namespace_segments + target_name +
    //         module. This is the case that used to silently lose info
    //         (`Oazapfts.RequestOpts` was just an unresolved string).
    //
    //   (c) Bare target_name (no chain, no dot)
    //       → leave alone UNLESS this is a renamed named import where
    //         the local alias is being used directly (`import { foo as
    //         bar } from 'pkg'; bar()` emits target=`bar` which the
    //         source module doesn't export by that name). The resolver's
    //         scope-walk handles non-renamed bare refs better than
    //         module-attributed lookup — `in_module_from` is name-only
    //         and can't traverse class inheritance, while scope-walk
    //         hands the chain walker generic-aware resolution.
    //
    // The shape determines what we do; each branch handles its case
    // explicitly. Imports refs and already-canonicalized refs were
    // skipped at the top-level loop.
    let (head, tail): (String, Option<String>) = if let Some(chain) = &r.chain {
        if chain.segments.len() < 2 {
            // Chain of length 1 = bare identifier. Treat the same as
            // no-chain bare below: act broadly only for Calls (JSX
            // component invocations and direct calls to imports);
            // bare TypeRefs go through the renamed-only path so the
            // resolver's scope walk + chain walker keep their class-
            // hierarchy traversal capability.
            handle_bare(r, imports);
            return;
        }
        let Some(first) = chain.segments.first() else {
            return;
        };
        (first.name.clone(), None)
    } else {
        match r.target_name.split_once('.') {
            Some((h, t)) if !t.is_empty() => (h.to_string(), Some(t.to_string())),
            _ => {
                handle_bare(r, imports);
                return;
            }
        }
    };

    let Some(entry) = imports.get(&head) else {
        return;
    };
    if entry.kind == ImportKind::SideEffect {
        return;
    }

    match &entry.kind {
        ImportKind::Namespace => {
            // `Foo.X.Y` → namespace_segments=["X"], target="Y", module=Foo's import.
            // Chain shapes (`Foo.method()`) keep target_name as the leaf
            // method (already what the chain walker expects); we only set
            // module so the demand-seed/resolver can route.
            r.module = Some(entry.module.clone());
            if r.chain.is_none() {
                if let Some(tail) = tail {
                    let mut parts: Vec<String> = tail.split('.').map(String::from).collect();
                    if let Some(leaf) = parts.pop() {
                        r.namespace_segments = parts;
                        r.target_name = leaf;
                    }
                }
                // No tail → ref was just `Foo` itself (rare for a
                // namespace import; usually means `typeof Foo` or
                // similar). Leave target_name alone.
            }
        }
        ImportKind::Named { exported_name } => {
            r.module = Some(entry.module.clone());
            // For chain-rooted refs where the local was renamed
            // (`import { foo as bar }; bar.method()`), the chain walker
            // resolves chain[0] against the file's symbol table. The
            // imported local `bar` gets a binding row, but it points at
            // the export named `foo` in the source module — so the
            // chain walker needs `chain[0].name == "foo"` to follow the
            // edge cleanly. Rewrite in place when the rename is
            // non-trivial.
            if exported_name != &head {
                if let Some(chain) = r.chain.as_mut() {
                    if let Some(first) = chain.segments.first_mut() {
                        first.name = exported_name.clone();
                    }
                }
            }
            // Substitute renamed local back to the exported name when
            // the ref names the local directly (no chain).
            if r.chain.is_none() && r.target_name == head && exported_name != &head {
                r.target_name = exported_name.clone();
            } else if r.chain.is_none() {
                if let Some(tail) = tail {
                    // `bar.X` where `bar` is renamed — treat like namespace.
                    let mut parts: Vec<String> = tail.split('.').map(String::from).collect();
                    if let Some(leaf) = parts.pop() {
                        r.namespace_segments = parts;
                        r.target_name = leaf;
                    }
                }
            }
        }
        ImportKind::Default => {
            // Default imports get module attribution; the resolver
            // treats the local name as the imported value's name. If
            // the user qualifies the default import (`Foo.member`),
            // that's a member access — leave target/segments to the
            // chain walker / namespace_segments split below.
            r.module = Some(entry.module.clone());
            if r.chain.is_none() {
                if let Some(tail) = tail {
                    let mut parts: Vec<String> = tail.split('.').map(String::from).collect();
                    if let Some(leaf) = parts.pop() {
                        r.namespace_segments = parts;
                        r.target_name = leaf;
                    }
                }
            }
        }
        ImportKind::SideEffect => {}
    }
}

/// Handle the bare-target case (no chain or chain.len==1, no dot).
///
/// Behavior splits by ref kind:
///
///   - **Calls** (JSX components, direct invocations of imports): act
///     broadly. Set module from the import + apply renamed-alias
///     rewrite. The downstream resolver's `in_module_from` lookup
///     finds these reliably and we can't rely on scope walk for them
///     in TSX files (JSX call expressions have weaker scope context).
///
///   - **TypeRefs** (bare class/interface/type-alias names): only
///     act for renamed named imports. Setting module on a non-renamed
///     bare TypeRef short-circuits the resolver's scope walk, which
///     would normally find the symbol via the import binding AND
///     hand the chain walker the class for inheritance traversal.
///     `in_module_from` is name-only and stops at the class — no
///     `Kysely → QueryCreator → selectFrom` walk.
///
///   - **Other kinds** (Inherits, Implements, Instantiates, etc.):
///     same conservative path as TypeRef — let the scope walker
///     handle them.
fn handle_bare(r: &mut ExtractedRef, imports: &HashMap<String, ImportEntry>) {
    let local = r
        .chain
        .as_ref()
        .and_then(|c| c.segments.first())
        .map(|s| s.name.as_str())
        .unwrap_or(r.target_name.as_str())
        .to_string();
    let Some(entry) = imports.get(&local) else { return };
    if entry.kind == ImportKind::SideEffect {
        return;
    }

    // For renamed named imports, always rewrite — the local alias
    // never matches a real export.
    let renamed_to: Option<&str> = match &entry.kind {
        ImportKind::Named { exported_name } if exported_name != &local => {
            Some(exported_name.as_str())
        }
        _ => None,
    };

    // Calls — act broadly. JSX components and direct call refs
    // benefit from module attribution.
    let is_call = r.kind == EdgeKind::Calls;

    if renamed_to.is_none() && !is_call {
        return;
    }

    if let Some(new_name) = renamed_to {
        if let Some(chain) = r.chain.as_mut() {
            if let Some(first) = chain.segments.first_mut() {
                first.name = new_name.to_string();
            }
        } else {
            r.target_name = new_name.to_string();
        }
    }
    r.module = Some(entry.module.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ExtractedRef;

    fn typeref(target: &str) -> ExtractedRef {
        ExtractedRef {
            source_symbol_index: 0,
            target_name: target.into(),
            kind: EdgeKind::TypeRef,
            line: 0,
            module: None,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            chain: None,
            byte_offset: 0,
        }
    }

    fn ns(local: &str, module: &str) -> ImportEntry {
        ImportEntry {
            local_name: local.into(),
            module: module.into(),
            kind: ImportKind::Namespace,
        }
    }

    fn named(local: &str, exported: &str, module: &str) -> ImportEntry {
        ImportEntry {
            local_name: local.into(),
            module: module.into(),
            kind: ImportKind::Named { exported_name: exported.into() },
        }
    }

    #[test]
    fn namespace_qualified_typeref_splits_prefix() {
        let mut refs = vec![typeref("Oazapfts.RequestOpts")];
        let imports: HashMap<_, _> = [(
            "Oazapfts".to_string(),
            ns("Oazapfts", "@oazapfts/runtime"),
        )]
        .into_iter()
        .collect();
        resolve_import_refs(&mut refs, &imports);
        assert_eq!(refs[0].target_name, "RequestOpts");
        assert_eq!(refs[0].module.as_deref(), Some("@oazapfts/runtime"));
        assert!(refs[0].namespace_segments.is_empty());
    }

    #[test]
    fn three_segment_namespace_carries_intermediate() {
        let mut refs = vec![typeref("Express.Multer.File")];
        let imports: HashMap<_, _> = [(
            "Express".to_string(),
            ns("Express", "express"),
        )]
        .into_iter()
        .collect();
        resolve_import_refs(&mut refs, &imports);
        assert_eq!(refs[0].target_name, "File");
        assert_eq!(refs[0].namespace_segments, vec!["Multer".to_string()]);
        assert_eq!(refs[0].module.as_deref(), Some("express"));
    }

    #[test]
    fn renamed_named_import_substitutes_target() {
        // import { foo as bar } from 'pkg'; ... bar() — ref carries
        // target_name="bar", needs to become "foo".
        let mut refs = vec![ExtractedRef {
            source_symbol_index: 0,
            target_name: "bar".into(),
            kind: EdgeKind::Calls,
            line: 0,
            module: None,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            chain: None,
            byte_offset: 0,
        }];
        let imports: HashMap<_, _> =
            [("bar".to_string(), named("bar", "foo", "pkg"))].into_iter().collect();
        resolve_import_refs(&mut refs, &imports);
        assert_eq!(refs[0].target_name, "foo");
        assert_eq!(refs[0].module.as_deref(), Some("pkg"));
    }

    #[test]
    fn unmapped_target_left_alone() {
        let mut refs = vec![typeref("LocalThing")];
        let imports: HashMap<_, _> = [(
            "Other".to_string(),
            ns("Other", "other-pkg"),
        )]
        .into_iter()
        .collect();
        resolve_import_refs(&mut refs, &imports);
        assert_eq!(refs[0].target_name, "LocalThing");
        assert!(refs[0].module.is_none());
    }

    #[test]
    fn already_canonicalized_skipped() {
        let mut refs = vec![ExtractedRef {
            source_symbol_index: 0,
            target_name: "X".into(),
            kind: EdgeKind::TypeRef,
            line: 0,
            module: Some("preset".into()),
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            chain: None,
            byte_offset: 0,
        }];
        let imports: HashMap<_, _> = [(
            "X".to_string(),
            named("X", "Y", "different"),
        )]
        .into_iter()
        .collect();
        resolve_import_refs(&mut refs, &imports);
        assert_eq!(refs[0].target_name, "X");
        assert_eq!(refs[0].module.as_deref(), Some("preset"));
    }

    #[test]
    fn idempotent_double_apply() {
        let mut refs = vec![typeref("Foo.Bar")];
        let imports: HashMap<_, _> =
            [("Foo".to_string(), ns("Foo", "pkg"))].into_iter().collect();
        resolve_import_refs(&mut refs, &imports);
        let after_first = refs.clone();
        resolve_import_refs(&mut refs, &imports);
        assert_eq!(refs[0].target_name, after_first[0].target_name);
        assert_eq!(refs[0].module, after_first[0].module);
        assert_eq!(refs[0].namespace_segments, after_first[0].namespace_segments);
    }
}
