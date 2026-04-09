// =============================================================================
// languages/puppet/resolve.rs — Puppet resolution rules
//
// Puppet references:
//
//   include apache::config            → Calls, target_name = "apache::config"
//   require nginx                     → Calls, target_name = "nginx"
//   class { 'myapp::web': }           → TypeRef, target_name = "myapp::web"
//   file { '/etc/app.conf': }         → TypeRef (built-in resource type)
//   Class['apache']                   → TypeRef, target_name = "apache"
//
// Resolution strategy:
//   1. Same-file: classes and defined types in the same manifest.
//   2. Import-based: Puppet autoloads classes from the module path using the
//      `::` namespace separator. `apache::config` maps to
//      `apache/manifests/config.pp`.
//   3. Global name lookup (cross-file).
//   4. Built-in Puppet resource types are external.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct PuppetResolver;

impl LanguageResolver for PuppetResolver {
    fn language_ids(&self) -> &[&str] {
        &["puppet"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // Puppet uses autoloading based on the `::` namespace — no explicit
        // import statements to collect.
        FileContext {
            file_path: file.path.clone(),
            language: "puppet".to_string(),
            imports: Vec::new(),
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

        // Built-in Puppet resource types never live in the project index.
        if builtins::is_puppet_builtin(target) {
            return None;
        }

        engine::resolve_common("puppet", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Combine resource types and built-in functions for the common helper.
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_puppet_builtin)
            .map(|_| "puppet".to_string())
    }
}

