use super::predicates;
use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct ClojureHooks;

impl LanguageEngineHooks for ClojureHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Java interop — method calls start with `.`, constructor calls end with `.`.
        if predicates::is_java_interop(target) {
            return Some("java".to_string());
        }

        // Fully-qualified Java class references.
        if predicates::is_java_class_ref(target) {
            return Some("java".to_string());
        }

        // Bare CamelCase names imported via :import — classify as Java external
        // when any Java package import is present.
        let is_camel = target.starts_with(|c: char| c.is_uppercase())
            && !target.contains('-')
            && !target.contains('/');
        if is_camel {
            let has_java_import = file_ctx.imports.iter().any(|imp| {
                imp.module_path
                    .as_deref()
                    .map(predicates::is_java_class_ref)
                    .unwrap_or(false)
            });
            if has_java_import {
                return Some("java".to_string());
            }
        }

        // Per-:refer named imports — match target against the alias.
        if let Some(import) = file_ctx.imports.iter().find(|i| {
            i.alias.as_deref() == Some(target.as_str()) && i.module_path.is_some()
        }) {
            if let Some(ns) = import.module_path.as_deref() {
                return Some(ns.to_string());
            }
        }

        // Side-effect-only `(:require [ns])` reclassification — only when no
        // internal project symbol exists by this name.
        if target.is_empty() || target.contains('/') || target.starts_with(':') {
            return None;
        }
        let has_internal_match = lookup
            .by_name(target)
            .iter()
            .any(|s| !s.file_path.starts_with("ext:"));
        if has_internal_match {
            return None;
        }
        let wildcard_ns = file_ctx.imports.iter().find(|i| {
            i.is_wildcard
                && i.module_path
                    .as_deref()
                    .map(|p| p.contains('.'))
                    .unwrap_or(false)
        })?;
        let ns = wildcard_ns.module_path.as_deref()?;
        let root = ns.split('.').next().unwrap_or(ns);
        Some(root.to_string())
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        resolve::detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }
}

pub static CLOJURE_HOOKS: ClojureHooks = ClojureHooks;
