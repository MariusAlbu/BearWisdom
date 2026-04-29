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
// .NET interop — local-var type binding (Pass 1 + Pass 2):
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
//   Pass 2 adds three more sentinel kinds (same wire format, different source):
//
//   Part 1 — hashtable-indexer registry:
//     `$sync["Key"].Dispatcher.Invoke(…)`
//     extractor detects `$sync[` on any line → sentinel module=Some("sync")
//     extractor's invokation_module / extract_member_access walks through
//     element_access nodes to find the root variable → ref.module = "sync"
//     is_dotnet_bound_var("sync", …) → true → dotnet-stdlib.
//
//   Part 2 — pipeline variable `$_`:
//     `ForEach-Object { $_.Visibility = … }`
//     extractor detects `$_.` on any line → sentinel module=Some("_")
//     `$_` is a plain variable so ref.module = "_" via existing path.
//     is_dotnet_bound_var("_", …) → true → dotnet-stdlib.
//
//   Part 3 — cmdlet-result chain:
//     `(Get-Date).ToString("…")`
//     extractor detects `(Get-Date).` → sentinel module=Some("__cmdlet_get_date")
//     invokation_module recognizes parenthesized_expression → command →
//     returns "__cmdlet_get_date" as module.
//     is_dotnet_bound_var("__cmdlet_get_date", …) → true → dotnet-stdlib.
//
// PowerShell import model:
//   `Import-Module ModuleName`       → target_name = "ModuleName"
//   `using module ./MyModule.psm1`   → target_name = "./MyModule.psm1"
//   `using namespace System.Text`    → target_name = "System.Text"
//
// The extractor emits EdgeKind::Imports with target_name = the module or
// namespace identifier.
// =============================================================================

use super::extract::{is_dotnet_type_name, DOTNET_BINDING_SENTINEL};
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
        //
        // Also skip when the module is itself a .NET type name (static members
        // like `[Windows.Visibility]::Visible` → module="Windows.Visibility").
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_dotnet_bound_var(module, file_ctx) || is_dotnet_type_name(module) {
                return None;
            }
        }

        if let Some(res) = engine::resolve_common(
            "powershell", file_ctx, ref_ctx, lookup, predicates::kind_compatible,
        ) {
            return Some(res);
        }

        // PowerShell bare-name fallback. Counterpart to the SCSS / Bash /
        // Python / Java `<lang>_bare_name` resolver steps. PowerShell
        // scripts call functions globally (no per-file namespace once
        // a script is dot-sourced) and reference module-private cmdlets
        // by bare name. The module / scope-chain / same-file machinery
        // can't bind these. Index-wide name lookup gated by `.ps1` /
        // `.psm1` / `.psd1` file extension.
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_ps = path.ends_with(".ps1")
                    || path.ends_with(".psm1")
                    || path.ends_with(".psd1");
                if !is_ps {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "powershell_bare_name",
                    resolved_yield_type: None,
                });
            }
        }

        None
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
        //
        // Also covers static member access on .NET type literals:
        //   [Windows.Visibility]::Visible  → module="Windows.Visibility", kind=TypeRef
        //   [System.IO.File]::Exists(…)    → module="System.IO.File", kind=Calls
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_dotnet_bound_var(module, file_ctx) {
                return Some(DOTNET_BINDING_SENTINEL.to_string());
            }
            // Static member on a bare .NET type literal (no sentinel needed —
            // the type name itself is the signal).
            if is_dotnet_type_name(module) {
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

        // External executables invoked as PowerShell commands — `git`,
        // `dotnet`, `npm`, `curl`, `fsh`, etc. These are not PowerShell
        // symbols at all; they're processes resolved via `$env:PATH` at
        // runtime. Classify them under the `cli` namespace so they leave
        // unresolved_refs while the graph still records the invocation.
        if ref_ctx.extracted_ref.kind == EdgeKind::Calls
            && looks_like_external_executable(&ref_ctx.extracted_ref.target_name)
        {
            return Some("cli".to_string());
        }

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_powershell_builtin)
    }
}

/// A command name looks like an external executable when it:
///   * is a short identifier with no `-` (cmdlets use Verb-Noun),
///   * contains no `.` (dotted paths are method / property access), and
///   * contains no `$` / whitespace / special shell metacharacters.
///
/// PowerShell users also commonly invoke well-known CLI tools (`git`,
/// `docker`, `kubectl`, `az`) this way; they're impossible to resolve
/// locally but shouldn't pollute `unresolved_refs`.
fn looks_like_external_executable(name: &str) -> bool {
    if name.is_empty() || name.contains('-') || name.contains('.') {
        return false;
    }
    // Must start with an alphabetic char (exclude operators, digits, etc.).
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    // Remaining chars: alphanumeric or underscore (valid identifier).
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether `var_name` (stripped of `$`) is bound to a .NET type in the
/// current file's import entries (placed there by `build_file_context`).
/// Matches case-insensitively — PowerShell variable references are case-
/// insensitive, so `$Tweaks` and `$tweaks` alias the same binding.
fn is_dotnet_bound_var(var_name: &str, file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        imp.module_path.as_deref() == Some(DOTNET_BINDING_SENTINEL)
            && imp.imported_name.eq_ignore_ascii_case(var_name)
    })
}

/// PowerShell cmdlet naming convention: `Verb-Noun` with both parts being
/// alphanumeric identifiers. PowerShell is case-insensitive so `New-object`
/// is the same cmdlet as `New-Object` — both match here. Anything else
/// (free functions, user-defined cmdlets, dot-sourced scripts) takes the
/// regular resolution path.
fn is_cmdlet_name(name: &str) -> bool {
    let (verb, noun) = match name.split_once('-') {
        Some((v, n)) if !v.is_empty() && !n.is_empty() => (v, n),
        _ => return false,
    };
    let is_ident_part = |s: &str| {
        let mut it = s.chars();
        match it.next() {
            Some(c) if c.is_ascii_alphabetic() => {}
            _ => return false,
        }
        it.all(|c| c.is_ascii_alphanumeric())
    };
    is_ident_part(verb) && is_ident_part(noun)
}
