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
//   5. Forge module references: first `::` segment is a known forge module →
//      classified as external without index lookup.
// =============================================================================

use super::predicates;
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
        if predicates::is_puppet_builtin(target) {
            return None;
        }

        // Forge module references are external — skip index lookup.
        if let Some(prefix) = target.split("::").next() {
            if predicates::is_forge_module(prefix) {
                return None;
            }
        }

        // For qualified names with `::`, the extractor stores the full name as
        // `target_name` (e.g. "profile::base"). `resolve_common` step 5 handles
        // `target.contains("::")` via `by_qualified_name`. We additionally try
        // matching just the last segment in the same file (for locally-defined
        // classes whose qualified_name was recorded without the module prefix).
        if target.contains("::") {
            let last_segment = target.split("::").last().unwrap_or(target.as_str());
            for sym in lookup.in_file(&file_ctx.file_path) {
                if (sym.name == *target || sym.name == last_segment || sym.qualified_name == *target)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "puppet_qualified_same_file",
                        resolved_yield_type: None,
                    });
                }
            }

            // Cross-file: try exact qualified name, then just last segment
            // (handles classes declared without module prefix in their own file).
            if let Some(sym) = lookup.by_qualified_name(target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "puppet_qualified_global",
                        resolved_yield_type: None,
                    });
                }
            }
            for sym in lookup.by_name(last_segment) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "puppet_unqualified_fallback",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        engine::resolve_common("puppet", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Forge module: first `::` segment is a known forge module prefix.
        if let Some(prefix) = target.split("::").next() {
            if predicates::is_forge_module(prefix) {
                return Some(format!("puppet_forge::{prefix}"));
            }
        }

        // Built-in resource types and functions.
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_puppet_builtin)
            .map(|_| "puppet".to_string())
    }
}

