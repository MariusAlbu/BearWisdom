// =============================================================================
// vbnet/hooks.rs — VB.NET language engine hooks.
//
// VB.NET shares the .NET type system with C#: same namespace + Imports
// pattern, same class/struct/interface taxonomy. Resolution rides on the
// DefaultResolver tower; the file-context build step normalises
// `Imports` refs the extractor emits into ImportEntry rows.
// =============================================================================

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct VbNetHooks;

impl LanguageEngineHooks for VbNetHooks {
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

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static VBNET_HOOKS: VbNetHooks = VbNetHooks;
