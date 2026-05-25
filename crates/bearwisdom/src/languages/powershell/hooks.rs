// PowerShell language hooks. Absorbed from the deleted `powershell/resolve.rs`.

use super::extract::{is_dotnet_type_name, DOTNET_BINDING_SENTINEL};
use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct PowerShellHooks;

fn looks_like_external_executable(name: &str) -> bool {
    if name.is_empty() || name.contains('-') {
        return false;
    }
    if let Some(stem) = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".cmd"))
        .or_else(|| name.strip_suffix(".bat"))
        .or_else(|| name.strip_suffix(".com"))
        .or_else(|| name.strip_suffix(".msi"))
    {
        return is_bare_executable_stem(stem);
    }
    if name.contains('.') {
        return false;
    }
    is_bare_executable_stem(name)
}

fn is_bare_executable_stem(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_dotnet_bound_var(var_name: &str, file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        imp.module_path.as_deref() == Some(DOTNET_BINDING_SENTINEL)
            && imp.imported_name.eq_ignore_ascii_case(var_name)
    })
}

fn is_cmdlet_name(name: &str) -> bool {
    let (verb, noun) = match name.split_once('-') {
        Some((v, n)) if !v.is_empty() && !n.is_empty() => (v, n),
        _ => return false,
    };
    let is_ident_part = |s: &str| {
        let mut it = s.chars();
        match it.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => return false,
        }
        it.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    is_ident_part(verb) && is_ident_part(noun)
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    _project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    if let Some(module) = &ref_ctx.extracted_ref.module {
        if is_dotnet_bound_var(module, file_ctx) {
            return Some(DOTNET_BINDING_SENTINEL.to_string());
        }
        if is_dotnet_type_name(module) {
            return Some(DOTNET_BINDING_SENTINEL.to_string());
        }
    }
    let target = &ref_ctx.extracted_ref.target_name;
    if is_cmdlet_name(target) {
        return Some("powershell-stdlib".to_string());
    }
    if is_dotnet_type_name(target) {
        return Some(DOTNET_BINDING_SENTINEL.to_string());
    }
    if let Some((module_part, leaf)) = target.split_once('\\') {
        if !module_part.is_empty() && is_cmdlet_name(leaf) {
            return Some(module_part.to_string());
        }
    }
    if ref_ctx.extracted_ref.kind == EdgeKind::Calls
        && looks_like_external_executable(&ref_ctx.extracted_ref.target_name)
    {
        return Some("cli".to_string());
    }
    None
}

impl LanguageEngineHooks for PowerShellHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            if r.target_name == DOTNET_BINDING_SENTINEL {
                if let Some(var_name) = &r.module {
                    imports.push(ImportEntry {
                        imported_name: var_name.clone(),
                        module_path: Some(DOTNET_BINDING_SENTINEL.to_string()),
                        alias: None,
                        is_wildcard: false,
                    });
                }
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "powershell".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_dotnet_bound_var(module, file_ctx) || is_dotnet_type_name(module) {
                return None;
            }
        }
        if let Some(res) = (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all() {
            return Some(res);
        }
        let target = &ref_ctx.extracted_ref.target_name;
        None
    }
}

pub static POWERSHELL_HOOKS: PowerShellHooks = PowerShellHooks;
