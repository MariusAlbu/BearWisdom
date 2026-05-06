// =============================================================================
// perl/resolve.rs — Perl resolution rules
//
// Scope rules for Perl:
//
//   1. Scope chain walk: innermost subroutine/package → outermost.
//   2. Same-file resolution: all subroutines in the file are visible.
//   3. By-name lookup: for used modules, symbols may be defined elsewhere.
//
// Perl import model:
//   `use Module;`       → target_name = "Module"
//   `use Module qw(…);` → target_name = "Module"
//   `require Module;`   → target_name = "Module"
//
// The extractor emits EdgeKind::Imports with target_name = the module name.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Perl language resolver.
pub struct PerlResolver;

impl LanguageResolver for PerlResolver {
    fn language_ids(&self) -> &[&str] {
        &["perl"]
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "perl".to_string(),
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

        // Bare-name walker lookup. perl_stdlib walks <perl_root>/lib/<ver>/
        // for core modules (Carp, Data::Dumper, File::Path, IO::File, ...).
        // Interpreter built-ins (print, chomp, map, ...) are handled by the
        // engine's primitive set populated from `keywords()` — they
        // classify as `"primitive"` namespace via classify_external_name.
        if !target.contains("::") {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "perl_synthetic_global",
                    resolved_yield_type: None,
                });
            }
        }

        engine::resolve_common("perl", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // perl_stdlib walker emits real symbols; interpreter built-ins
        // are handled by the engine's keywords() primitive set. Names
        // that exhaust resolve() stay unresolved rather than being
        // blanket-classified.
        None
    }
}
