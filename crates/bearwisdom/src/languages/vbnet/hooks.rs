// =============================================================================
// vbnet/hooks.rs — VB.NET language engine hooks.
//
// VB.NET shares the .NET type system with C#: same namespace + Imports
// pattern, same class/struct/interface taxonomy. Resolution rides on the
// DefaultResolver tower; the file-context build step normalises
// `Imports` refs the extractor emits into ImportEntry rows.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct VbNetHooks;

// .NET namespace external classifier. NuGet manifest deps drive it when a
// ProjectContext is present; otherwise only the two always-present .NET SDK
// prefixes (System / Microsoft) are recognised. These are namespace facts,
// not an API-name table.
fn is_external_dotnet_namespace(project_ctx: Option<&ProjectContext>, ns: &str) -> bool {
    match project_ctx {
        Some(ctx) => crate::languages::csharp::hooks::is_manifest_external_namespace(ctx, ns),
        None => ns.starts_with("System") || ns.starts_with("Microsoft"),
    }
}

impl LanguageEngineHooks for VbNetHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // `Imports System.X` / `Imports <NuGetPkg>` — the import target IS the
        // namespace to classify.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            if is_external_dotnet_namespace(project_ctx, target) {
                return Some(target.clone());
            }
            return None;
        }

        // Bare type/call refs: classify external when any wildcard Imports
        // namespace is a .NET external. The bare name (`Button`) lives under an
        // external wildcard (`System.Windows.Controls`), so the ref is external
        // even when no hydrated symbol keyed under the FQN. Pick the longest
        // matching external namespace.
        let mut best: Option<&str> = None;
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }
            if is_external_dotnet_namespace(project_ctx, ns)
                && (best.is_none() || ns.len() > best.unwrap().len())
            {
                best = Some(ns);
            }
        }
        best.map(|s| s.to_string())
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        let mut file_namespace: Option<String> = None;
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let name = r.target_name.clone();
            let module = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: name,
                module_path: module,
                alias: None,
                is_wildcard: true,
            });
        }
        // File-level namespace, when the extractor placed a namespace_block at
        // the top: take the first qname-prefix that ends with a dot.
        for sym in &file.symbols {
            if sym.kind == crate::types::SymbolKind::Namespace {
                file_namespace = Some(sym.qualified_name.clone());
                break;
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "vbnet".to_string(),
            imports,
            file_namespace,
        })
    }
}

pub static VBNET_HOOKS: VbNetHooks = VbNetHooks;
