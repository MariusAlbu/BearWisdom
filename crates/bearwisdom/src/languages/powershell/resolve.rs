// =============================================================================
// powershell/resolve.rs — PowerShell resolution rules
//
// Scope rules for PowerShell:
//
//   1. Scope chain walk: innermost function/class → outermost.
//   2. Same-file resolution: all top-level functions and classes are visible
//      within the file.
//   3. By-name lookup: for imported modules, symbols may be defined elsewhere.
//
// PowerShell import model:
//   `Import-Module ModuleName`       → target_name = "ModuleName"
//   `using module ./MyModule.psm1`   → target_name = "./MyModule.psm1"
//   `using namespace System.Text`    → target_name = "System.Text"
//
// The extractor emits EdgeKind::Imports with target_name = the module or
// namespace identifier.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// PowerShell language resolver.
pub struct PowerShellResolver;

impl LanguageResolver for PowerShellResolver {
    fn language_ids(&self) -> &[&str] {
        &["powershell"]
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
            language: "powershell".to_string(),
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

        // PowerShell built-in cmdlets are never in the index.
        if builtins::is_powershell_builtin(&ref_ctx.extracted_ref.target_name) {
            return None;
        }

        engine::resolve_common("powershell", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_powershell_builtin)
    }
}
