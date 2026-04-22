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
// .NET interop — local-var type binding (Parts 1/2/3):
//
//   When PowerShell scripts create .NET objects and then call methods or
//   access properties on them, the extractor emits member-access refs like:
//     $border.Style   → target_name="Style",  module=Some("border"),  kind=TypeRef
//     $border.Add_MouseLeftButtonUp(…) → target_name="Add_MouseLeftButtonUp",
//                                         module=Some("border"), kind=Calls
//
//   Without type information for `$border`, the resolver has no way to match
//   these refs against the .NET framework index — they all land as
//   unresolved_refs.
//
//   Fix: the extractor now emits one sentinel Imports ref per .NET binding:
//     kind=Imports, target_name="dotnet-stdlib", module=Some(var_name)
//
//   In `build_file_context` we collect these sentinels and encode each binding
//   as an ImportEntry { imported_name: var_name, module_path: "dotnet-stdlib" }.
//
//   In `infer_external_namespace` we detect any unresolved ref whose `module`
//   field matches a .NET-bound variable name and route it to the "dotnet-stdlib"
//   ecosystem, turning an unresolved_ref into an external_ref.
//
//   In `resolve` we skip the normal index lookup for such refs — the .NET
//   framework members won't be in the project index.
//
// PowerShell import model:
//   `Import-Module ModuleName`       → target_name = "ModuleName"
//   `using module ./MyModule.psm1`   → target_name = "./MyModule.psm1"
//   `using namespace System.Text`    → target_name = "System.Text"
//
// The extractor emits EdgeKind::Imports with target_name = the module or
// namespace identifier.
// =============================================================================

use super::extract::DOTNET_BINDING_SENTINEL;
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

            // Part 1: collect .NET local-variable type bindings emitted by the
            // extractor as sentinel Imports refs (target_name == "dotnet-stdlib").
            // Encode each as an ImportEntry so `is_dotnet_bound_var` and
            // `infer_external_namespace` can look them up cheaply.
            if r.target_name == DOTNET_BINDING_SENTINEL {
                if let Some(var_name) = &r.module {
                    imports.push(ImportEntry {
                        imported_name: var_name.clone(),
                        module_path: Some(DOTNET_BINDING_SENTINEL.to_string()),
                        alias: None,
                        is_wildcard: false,
                    });
                }
                continue; // don't also emit as a regular import
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

        // Part 2: if the ref's qualifier variable is .NET-bound, skip the
        // project-index lookup entirely — the member won't be there.
        // This prevents a false match against any same-named project symbol
        // and lets the ref fall through to `infer_external_namespace` cleanly.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_dotnet_bound_var(module, file_ctx) {
                return None;
            }
        }

        engine::resolve_common("powershell", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Part 3: member-access refs on .NET-bound variables → dotnet-stdlib.
        // Covers both property reads (TypeRef) and method calls (Calls):
        //   $border.Style          → module="border", kind=TypeRef
        //   $border.Add_Click(…)   → module="border", kind=Calls
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_dotnet_bound_var(module, file_ctx) {
                return Some(DOTNET_BINDING_SENTINEL.to_string());
            }
        }

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether `var_name` (stripped of `$`) is bound to a .NET type in the
/// current file's import entries (placed there by `build_file_context`).
fn is_dotnet_bound_var(var_name: &str, file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        imp.module_path.as_deref() == Some(DOTNET_BINDING_SENTINEL)
            && imp.imported_name == var_name
    })
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
