// =============================================================================
// clojure/resolve.rs — Clojure resolution rules
//
// Scope rules for Clojure:
//
//   1. Scope chain walk: innermost let/letfn → defn → ns.
//   2. Same-file resolution: all top-level vars/defs in the namespace are visible.
//   3. Import-based resolution:
//        `(ns my.ns (:require [lib :as l]))` → aliased require
//        `(require '[lib :as l])`            → aliased require
//        `(use 'lib)`                        → wildcard use
//        `(import '(java.util Date))`        → Java class import
//
// Clojure import model:
//   target_name = the local alias or namespace name
//   module      = the canonical namespace when an alias is present
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Clojure language resolver.
pub struct ClojureResolver;

impl LanguageResolver for ClojureResolver {
    fn language_ids(&self) -> &[&str] {
        &["clojure"]
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
            // target_name is the local alias or the full namespace.
            // module is the canonical namespace when an alias is present.
            let ns = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != ns {
                Some(r.target_name.clone())
            } else {
                None
            };

            let is_wildcard = alias.is_none();
            imports.push(ImportEntry {
                imported_name: ns.to_string(),
                module_path: Some(ns.to_string()),
                alias,
                is_wildcard,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "clojure".to_string(),
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
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // clojure.core and special forms are not in the project index.
        if predicates::is_clojure_builtin(target) {
            return None;
        }

        engine::resolve_common("clojure", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Java interop — method calls start with `.` (e.g. `.getBytes`, `.close`)
        // and constructor calls end with `.` (e.g. `File.`, `ArrayList.`).
        // Neither can resolve to a project symbol, so classify them immediately
        // as Java interop externals rather than leaving them unresolved.
        if predicates::is_java_interop(target) {
            return Some("java".to_string());
        }

        // Fully-qualified Java class references (contain a `.` that isn't a
        // Clojure namespace separator we already know about, e.g.
        // `java.io.ByteArrayOutputStream.` or `java.lang.Thread`).
        if predicates::is_java_class_ref(target) {
            return Some("java".to_string());
        }

        // Bare Java class names imported via `:import` (e.g. `File`, `InputStream`,
        // `Server`). The extractor emits the Java package (`java.io`) as a wildcard
        // import, but `infer_external_common` won't match unqualified class names to
        // those wildcard imports because the Clojure manifest guard blocks it.
        //
        // Heuristic: if the target is CamelCase (starts with an uppercase letter,
        // no `-` or `/`) and the file has at least one Java package import, classify
        // it as a Java external. This covers `File`, `InputStream`, `Server`, etc.
        // without touching truly Clojure-style names like `ClojurePlugin` (which are
        // indexed as local project symbols and resolve normally).
        let is_camel = target.starts_with(|c: char| c.is_uppercase())
            && !target.contains('-')
            && !target.contains('/');
        if is_camel {
            let has_java_import = file_ctx.imports.iter().any(|imp| {
                imp.module_path.as_deref().map(predicates::is_java_class_ref).unwrap_or(false)
            });
            if has_java_import {
                return Some("java".to_string());
            }
        }

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_clojure_builtin)
    }
}
