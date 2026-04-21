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

use super::predicates;
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
        if predicates::is_powershell_builtin(&ref_ctx.extracted_ref.target_name) {
            return None;
        }

        engine::resolve_common("powershell", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // PowerShell auto-loads modules on first cmdlet use — a bare
        // `Verb-Noun` call doesn't need an explicit import. Route any
        // unresolved ref matching the cmdlet pattern to the stdlib/gallery
        // ecosystem so the demand loop can surface it.
        if is_cmdlet_name(&ref_ctx.extracted_ref.target_name) {
            return Some("powershell-stdlib".to_string());
        }
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_powershell_builtin)
    }
}

/// PowerShell cmdlet naming convention: `Verb-Noun` with both parts being
/// alphanumeric identifiers starting with an uppercase letter. Anything
/// else (free functions, user-defined cmdlets, dot-sourced scripts) takes
/// the regular resolution path.
fn is_cmdlet_name(name: &str) -> bool {
    let (verb, noun) = match name.split_once('-') {
        Some((v, n)) if !v.is_empty() && !n.is_empty() => (v, n),
        _ => return false,
    };
    let is_ident_part = |s: &str| {
        let mut it = s.chars();
        match it.next() {
            Some(c) if c.is_ascii_uppercase() => {}
            _ => return false,
        }
        it.all(|c| c.is_ascii_alphanumeric())
    };
    is_ident_part(verb) && is_ident_part(noun)
}
