// =============================================================================
// ada/resolve.rs — Ada resolution rules
//
// Scope rules for Ada:
//
//   1. Scope chain walk: innermost subprogram/block → package → library.
//   2. Same-file resolution: all declarations in the same compilation unit.
//   3. Import-based resolution:
//        `with Package_Name;` → makes Package_Name visible (dot-qualified)
//        `use Package_Name;`  → brings all exported names into direct scope
//
// The extractor emits EdgeKind::Imports with:
//   target_name = package name (both `with` and `use` clauses)
//   module      = None (Ada imports are always the package name itself)
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Ada language resolver.
pub struct AdaResolver;

impl LanguageResolver for AdaResolver {
    fn language_ids(&self) -> &[&str] {
        &["ada"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // Both `with` and `use` clauses produce Imports edges. The
            // package_renaming_declaration handler in the extractor sets
            // `module` to the renamed-target package (e.g. for
            // `package Trace renames Simple_Logging;` the ref carries
            // target_name="Trace" and module=Some("Simple_Logging"));
            // when present, that's the actual module to look up.
            let module_path = r
                .module
                .clone()
                .unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: true, // Ada `use` makes all names visible
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "ada".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        let target_lower = target.to_lowercase();
        let simple = target.split('.').last().unwrap_or(target);
        let simple_lower = simple.to_lowercase();

        // Ada identifiers are case-insensitive; check same-file with case folding
        // before delegating to the common resolver (which uses exact matching).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == simple_lower
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ada_same_file_ci",
                    resolved_yield_type: None,
                });
            }
        }

        // Use-clause lookup: `use Ada.Text_IO;` (encoded as a wildcard import
        // by `build_file_context`) brings every public symbol of `Ada.Text_IO`
        // into scope. Resolving a bare `Put_Line(...)` call means searching
        // each use'd package's direct members for a case-insensitive name
        // match. The engine's wildcard path uses file_stem matching, which
        // breaks for GNAT's krunched filenames (`a-textio.ads` vs module
        // last-segment `text_io`); the qname-via-members_of path here works
        // independent of the file naming convention.
        if !target.contains('.') {
            for import in &file_ctx.imports {
                if !import.is_wildcard {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                for sym in lookup.members_of(module_path) {
                    if sym.name.to_lowercase() == simple_lower
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "ada_use_clause",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Dotted target with leading segment matching a use'd package: try
        // the full qname, then walk back through dotted segments. Handles
        // `Ada.Text_IO.Put_Line` written explicitly even when `use Ada;` is
        // active.
        if target.contains('.') {
            // Try direct qname lookup case-insensitively by walking
            // members_of for each successively-shorter parent prefix.
            let parts: Vec<&str> = target.split('.').collect();
            for split in (1..parts.len()).rev() {
                let parent = parts[..split].join(".");
                let leaf = parts[split..].join(".");
                let leaf_lower = leaf.to_lowercase();
                for sym in lookup.members_of(&parent) {
                    if sym.qualified_name
                        .rsplit_once('.')
                        .map(|(_, n)| n)
                        .unwrap_or(&sym.qualified_name)
                        .to_lowercase()
                        == leaf_lower
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "ada_qualified_ci",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        let _ = target_lower;
        engine::resolve_common("ada", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Ada standard library imports are classified by their top-level package name.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let root = target.split('.').next().unwrap_or(target);
            if matches!(root, "Ada" | "System" | "Interfaces" | "GNAT" | "Standard") {
                return Some(root.to_string());
            }
            // Non-stdlib imports: fall through to common handler.
        }

        // Bare names are classified by the engine's keywords() set
        // populated from ada/keywords.rs. The Ada.* / System.* / GNAT.*
        // import-classification above handles the namespace cases.
        let _ = (file_ctx, ref_ctx, project_ctx);
        None
    }
}
