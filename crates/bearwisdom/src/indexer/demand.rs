// =============================================================================
// indexer/demand.rs — R6 demand-driven external extraction
//
// Before parsing externals, we collect the set of names each external module
// is actually referenced by from project code. External extractors use this
// set to skip declarations the project doesn't touch — a 1.8MB `lib.dom.d.ts`
// that the project only uses 20 types from gets parsed as ~20 declarations
// instead of tens of thousands.
//
// Two sources of demand:
//   * **Direct imports** — an `ExtractedRef` whose `module` field is a bare
//     specifier adds its `target_name` to that module's demand set. Covers
//     `import { useState } from 'react'` → demand["react"] += {"useState"}.
//   * **Chain leaf names** — calls/accesses on imported objects contribute
//     their final segment name to the package attributed to the chain root.
//     Covers `axios.get(...)` → demand["axios"] += {"get"}. The root segment
//     must match an imported alias so we can attribute.
//
// Closure expansion (not yet implemented — planned follow-up):
//   * After the first external-extraction pass, walk kept external symbols'
//     `TypeRef` refs. Each target name becomes a new demand item for the
//     same module. Re-parse externals. Repeat until fixed point or 2-hop
//     cap. Catches types referenced from imported types' signatures.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ParsedFile};
use std::collections::{HashMap, HashSet};

/// Per-module demand set: module_path → names the project references.
///
/// "module_path" matches the specifier in `ExtractedRef::module` — bare TS
/// package names like `"react"` or `"@tanstack/react-query"`, Python dotted
/// imports like `"django.contrib.auth.models"`, etc. The same key is used
/// by external extractors when looking up whether to keep a declaration.
///
/// Special key [`GLOBAL_KEY`]: names referenced by project code WITHOUT an
/// import statement — DOM types in TS (`Document`, `fetch`), Node globals
/// (`process`, `Buffer`), Python builtins referenced through bare names.
/// External extractors pulling in ambient-global declaration files
/// (`lib.dom.d.ts`, `@types/node`) use this bucket as their demand set.
#[derive(Debug, Default, Clone)]
pub struct DemandSet {
    per_module: HashMap<String, HashSet<String>>,
}

/// Special module key for ambient-global demand. See struct docs on
/// [`DemandSet`].
pub const GLOBAL_KEY: &str = "__globals__";

impl DemandSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a single demand item.
    pub fn add(&mut self, module: &str, name: &str) {
        if name.is_empty() {
            return;
        }
        self.per_module
            .entry(module.to_string())
            .or_default()
            .insert(name.to_string());
    }

    /// Look up demand for a module. `None` means no demand tracked — the
    /// external extractor should take the permissive path (keep everything).
    pub fn for_module(&self, module: &str) -> Option<&HashSet<String>> {
        self.per_module.get(module)
    }

    /// Total (module, name) tuples tracked.
    pub fn total_items(&self) -> usize {
        self.per_module.values().map(|s| s.len()).sum()
    }

    /// Number of distinct modules with at least one demand.
    pub fn module_count(&self) -> usize {
        self.per_module.len()
    }

    pub fn is_empty(&self) -> bool {
        self.per_module.is_empty()
    }

    /// Build a DemandSet by scanning every project ParsedFile's refs.
    ///
    /// Scans each ref twice:
    ///   1. If it has a bare-module import, add `target_name` to that module's demand.
    ///   2. If it has a chain whose root segment matches an import alias in
    ///      this file, add the ref's leaf `target_name` to the import's module.
    ///
    /// Project files only — don't feed externals through here (their refs
    /// should not drive the same pass; handled by closure expansion later).
    pub fn from_parsed_files(parsed: &[ParsedFile]) -> Self {
        let mut set = Self::new();

        for pf in parsed {
            if pf.path.starts_with("ext:") {
                continue;
            }
            // Build a per-file alias → module map from this file's import refs
            // so chain roots like `useState(...)` can be attributed back to
            // the package they came from.
            let alias_to_module = build_alias_to_module(&pf.refs);

            for r in &pf.refs {
                add_from_ref(&mut set, r, &alias_to_module);
            }
        }

        set
    }

    /// Accessor for the ambient-global bucket (see [`GLOBAL_KEY`]).
    pub fn globals(&self) -> Option<&HashSet<String>> {
        self.for_module(GLOBAL_KEY)
    }
}

/// Build `local_alias → module_path` from a file's imports. TS/JS
/// extractor emits one `EdgeKind::TypeRef` ref per imported binding whose
/// `module` field is the source module.
fn build_alias_to_module(refs: &[ExtractedRef]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for r in refs {
        let Some(module) = r.module.as_deref() else { continue };
        if !is_bare_specifier(module) {
            continue;
        }
        // Only TypeRef (import bindings) and Imports contribute aliases.
        // Calls/instantiates refs carry a module field too after the
        // post-pass annotator; those are already demand items themselves.
        if matches!(r.kind, EdgeKind::TypeRef | EdgeKind::Imports) {
            out.insert(r.target_name.clone(), module.to_string());
        }
    }
    out
}

fn add_from_ref(
    set: &mut DemandSet,
    r: &ExtractedRef,
    alias_to_module: &HashMap<String, String>,
) {
    // Case 1: ref has an explicit module and the module is bare — add
    // target_name to that module's demand.
    if let Some(module) = r.module.as_deref() {
        if is_bare_specifier(module) {
            set.add(module, &r.target_name);
            return;
        }
        // Non-bare module (relative import) → project-local; nothing to
        // demand from externals.
        return;
    }

    // Case 2: chain-carrying ref. Route by whether the root is an imported
    // alias (→ that module) or unbound (→ globals bucket — `window.foo`,
    // `document.getElementById(...)`, ambient-global chain roots).
    if let Some(chain) = &r.chain {
        if let Some(root) = chain.segments.first() {
            if let Some(module) = alias_to_module.get(&root.name) {
                set.add(module, &r.target_name);
                for seg in chain.segments.iter().skip(1) {
                    set.add(module, &seg.name);
                }
            } else {
                // Unbound chain root → globals. Record both the root
                // (`document`) and every segment along the chain so
                // declarations like `interface Document { getElementById }`
                // get pulled in.
                set.add(GLOBAL_KEY, &root.name);
                for seg in chain.segments.iter().skip(1) {
                    set.add(GLOBAL_KEY, &seg.name);
                }
            }
        }
        return;
    }

    // Case 3: bare identifier ref with no chain and no module. Examples:
    // `HTMLElement` used as a type annotation, `fetch(...)` as a top-level
    // call, `Buffer.from(...)` where the chain builder didn't attach a
    // chain. Attribute to globals unless the identifier matches an
    // import alias (in which case it's internally referenced from its
    // package).
    if !alias_to_module.contains_key(&r.target_name) {
        set.add(GLOBAL_KEY, &r.target_name);
    }
}

/// A module specifier points to an external package rather than project-local
/// code. Bare-specifier rule: no leading `./`, `../`, `/`, `@/`, `~/`, no
/// drive-letter prefix, and either a scoped package (`@scope/pkg`) or a
/// single-segment name.
pub(crate) fn is_bare_specifier(m: &str) -> bool {
    if m.is_empty() {
        return false;
    }
    if m.starts_with('.') || m.starts_with('/') || m.starts_with("@/") || m.starts_with("~/") {
        return false;
    }
    let bytes = m.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return false;
    }
    // A scoped npm package (`@scope/pkg`) is bare. Other slash-containing
    // specifiers are tsconfig aliases / project-local.
    if m.starts_with('@') {
        return true;
    }
    !m.contains('/')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChainSegment, EdgeKind, ExtractedRef, FlowMeta, MemberChain, ParsedFile, SegmentKind,
    };

    fn mk_ref(target: &str, kind: EdgeKind, module: Option<&str>) -> ExtractedRef {
        ExtractedRef {
            source_symbol_index: 0,
            target_name: target.to_string(),
            kind,
            line: 0,
            module: module.map(|s| s.to_string()),
            namespace_segments: Vec::new(),
            chain: None,
            byte_offset: 0,
        }
    }

    fn mk_ref_with_chain(target: &str, root: &str) -> ExtractedRef {
        ExtractedRef {
            source_symbol_index: 0,
            target_name: target.to_string(),
            kind: EdgeKind::Calls,
            line: 0,
            module: None,
            chain: Some(MemberChain {
                segments: vec![
                    ChainSegment {
                        name: root.to_string(),
                        node_kind: "identifier".into(),
                        kind: SegmentKind::Identifier,
                        declared_type: None,
                        type_args: Vec::new(),
                        optional_chaining: false,
                    },
                    ChainSegment {
                        name: target.to_string(),
                        node_kind: "property_identifier".into(),
                        kind: SegmentKind::Property,
                        declared_type: None,
                        type_args: Vec::new(),
                        optional_chaining: false,
                    },
                ],
            }),
            namespace_segments: Vec::new(),
            byte_offset: 0,
        }
    }

    fn pf(path: &str, refs: Vec<ExtractedRef>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "typescript".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            symbols: Vec::new(),
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: Vec::new(),
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: None,
            has_errors: false,
            flow: FlowMeta::default(),
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
        }
    }

    #[test]
    fn bare_specifier_classification() {
        assert!(is_bare_specifier("react"));
        assert!(is_bare_specifier("@tanstack/react-query"));
        assert!(!is_bare_specifier("./foo"));
        assert!(!is_bare_specifier("../bar/baz"));
        assert!(!is_bare_specifier("/abs/path"));
        assert!(!is_bare_specifier("@/local/alias"));
        assert!(!is_bare_specifier("react-dom/client")); // deep import is a subpath
        assert!(!is_bare_specifier(""));
    }

    #[test]
    fn direct_import_adds_to_demand() {
        let file = pf(
            "app.ts",
            vec![mk_ref("useState", EdgeKind::TypeRef, Some("react"))],
        );
        let d = DemandSet::from_parsed_files(&[file]);
        assert!(d.for_module("react").unwrap().contains("useState"));
    }

    #[test]
    fn relative_import_ignored() {
        let file = pf(
            "app.ts",
            vec![mk_ref("foo", EdgeKind::TypeRef, Some("./util"))],
        );
        let d = DemandSet::from_parsed_files(&[file]);
        assert!(d.for_module("./util").is_none());
    }

    #[test]
    fn chain_on_imported_alias_adds_leaf_to_module() {
        let file = pf(
            "app.ts",
            vec![
                // `import axios from 'axios'`
                mk_ref("axios", EdgeKind::TypeRef, Some("axios")),
                // `axios.get(...)`
                mk_ref_with_chain("get", "axios"),
            ],
        );
        let d = DemandSet::from_parsed_files(&[file]);
        let axios = d.for_module("axios").unwrap();
        assert!(axios.contains("axios"));
        assert!(axios.contains("get"));
    }

    #[test]
    fn external_parsed_files_are_skipped() {
        let file = pf(
            "ext:ts:react/index.d.ts",
            vec![mk_ref("internal", EdgeKind::TypeRef, Some("react-dom"))],
        );
        let d = DemandSet::from_parsed_files(&[file]);
        assert_eq!(d.total_items(), 0);
    }
}
