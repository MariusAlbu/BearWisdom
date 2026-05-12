// =============================================================================
// fortran/resolve.rs — Fortran resolution rules
//
// Scope rules for Fortran:
//
//   1. Scope chain walk: innermost subroutine/function → module → program.
//   2. Same-file resolution: all top-level procedures visible within the file.
//   3. Import-based resolution:
//        `use module_name`                     → wildcard import (all public symbols)
//        `use module_name, only: sym1, sym2`   → named import for each symbol
//        `use module_name, only: local => src` → rename: local_name resolves to src
//
// The extractor emits EdgeKind::Imports refs in three shapes:
//
//   1. Module-level wildcard:
//        target_name = module_name, module = None, namespace_segments = []
//
//   2. Named only-symbol:
//        target_name = symbol_name, module = Some(module_name), namespace_segments = []
//
//   3. Rename (local_name => source_name):
//        target_name = local_name, module = Some(source_name),
//        namespace_segments = [module_name]
//
// `build_file_context` translates shape (3) into an ImportEntry with
//   imported_name = source_name (what to look up in the index)
//   alias         = local_name  (what call-site refs use)
//   module_path   = module_name
// so `resolve_common`'s alias-aware lookup can find it.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Fortran language resolver.
pub struct FortranResolver;

impl LanguageResolver for FortranResolver {
    fn language_ids(&self) -> &[&str] {
        &["fortran"]
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

            let is_rename = !r.namespace_segments.is_empty();

            if is_rename {
                // Shape (3): rename `local_name => source_name`.
                // namespace_segments[0] = module_name, module = source_name,
                // target_name = local_name.
                let module_path = r.namespace_segments.first().cloned();
                let source_name = r.module.clone().unwrap_or_default();
                let local_name = r.target_name.clone();
                if !source_name.is_empty() {
                    imports.push(ImportEntry {
                        // imported_name is the actual symbol name in the module.
                        imported_name: source_name,
                        module_path,
                        // alias is the local call-site name.
                        alias: Some(local_name),
                        is_wildcard: false,
                    });
                }
            } else if r.module.is_some() {
                // Shape (2): named only-symbol.
                // target_name = symbol, module = module_name.
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: r.module.clone(),
                    alias: None,
                    is_wildcard: false,
                });
            } else {
                // Shape (1): module-level wildcard.
                // target_name = module_name.
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(r.target_name.clone()),
                    alias: None,
                    is_wildcard: true,
                });
            }
        }

        FileContext {
            file_path: file.path.clone(),
            language: "fortran".to_string(),
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
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        let target = &ref_ctx.extracted_ref.target_name;
        let target_lower = target.to_lowercase();

        // Fortran is case-insensitive: check same-file with lowercased comparison
        // before delegating to resolve_common (which is case-sensitive).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == target_lower
                && predicates::kind_compatible(ref_ctx.extracted_ref.kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "fortran_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Derived-type method call: `module` holds the declared type name
        // (as extracted by the local type map or left as the variable name).
        // Probe members_of(type_name) for a bound procedure with this name.
        // When found, the backing subroutine is the same-named symbol anywhere
        // in the index (Fortran bound procedures delegate to top-level subs).
        if let Some(type_name) = &ref_ctx.extracted_ref.module {
            let type_lower = type_name.to_lowercase();
            // Try both case variants — Fortran is case-insensitive but symbol
            // storage preserves source case from the extractor.
            for tname in [type_name.as_str(), type_lower.as_str()] {
                for member in lookup.members_of(tname) {
                    if member.name.to_lowercase() == target_lower {
                        // Bound procedure declared. Resolve to the backing
                        // subroutine by name (confidence 0.9) or fall back
                        // to the member symbol itself (confidence 0.85).
                        for sym in lookup.by_name(target) {
                            if sym.name.to_lowercase() == target_lower
                                && predicates::kind_compatible(
                                    ref_ctx.extracted_ref.kind,
                                    &sym.kind,
                                )
                            {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 0.9,
                                    strategy: "fortran_type_member",
                                    resolved_yield_type: None,
                                    flow_emit: None,
                                });
                            }
                        }
                        return Some(Resolution {
                            target_symbol_id: member.id,
                            confidence: 0.85,
                            strategy: "fortran_type_member_direct",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        engine::resolve_common("fortran", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Fortran intrinsics + type specifiers + control-flow keywords
        // are classified by the engine's keywords() set (case-sensitive
        // match — KEYWORDS holds the lowercase forms; refs are normalised
        // upstream).
        None
    }
}
