// =============================================================================
// languages/heex/resolve.rs — HEEx component-call resolution
//
// HEEx templates call Elixir function components via `<.name attr={expr}>`.
// The host extractor emits each `<.name>` as an `EdgeKind::Calls` ref with
// the bare function name as target. Those functions live in the host module's
// imported scope — typically Phoenix.Component or the project's own
// CoreComponents module — and are indexed as ext: or internal symbols.
//
// Resolution model:
//   1. Bare-name lookup against ext: symbols — catches Phoenix built-ins
//      (Phoenix.Component.form, PhoenixHTMLHelpers.Form.label, etc.) that
//      the externals walker indexed from the project's `deps/` tree.
//   2. Bare-name lookup against internal symbols — catches project-defined
//      components (def my_component(assigns)) in the same codebase.
//   3. infer_external_namespace — if the target name dot-contains a module
//      root that is a known Elixir external (Phoenix, Ecto, …) or a listed
//      Mix dep, classify as external rather than unresolved.
//
// No `<.name>` ref carries an import context (HEEx files have no `import`
// directives of their own), so steps 1–2 are the only applicable lookup
// paths. The Elixir predicates module provides the external-module classifier
// reused here.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::languages::elixir;
use crate::types::{EdgeKind, ParsedFile};

pub struct HeexResolver;

impl LanguageResolver for HeexResolver {
    fn language_ids(&self) -> &[&str] {
        &["heex"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // HEEx files carry no import/alias directives of their own; all
        // function components are resolved by name against the symbol index.
        FileContext {
            file_path: file.path.clone(),
            language: "heex".to_string(),
            imports: Vec::<ImportEntry>::new(),
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind != EdgeKind::Calls {
            return None;
        }

        // Only handle bare function-component names — dotted targets
        // (Module.function) go through the heuristic.
        if target.contains('.') {
            return None;
        }

        // Step 1: external symbol lookup. Function components imported via
        // `use Phoenix.Component` or `import SomeModule` are indexed under
        // ext: paths. Prefer those over internal matches to avoid false edges
        // to unrelated same-named internal symbols.
        for sym in lookup.by_name(target) {
            if !sym.file_path.starts_with("ext:") {
                continue;
            }
            if !elixir::predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "heex_ext_component",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }

        // Step 2: internal symbol lookup — project-defined function components.
        for sym in lookup.by_name(target) {
            if sym.file_path.starts_with("ext:") {
                continue;
            }
            if !elixir::predicates::kind_compatible(edge_kind, &sym.kind) {
                continue;
            }
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.80,
                strategy: "heex_internal_component",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Module-qualified component refs like `<MyApp.Components.button>` —
        // delegate to the Elixir external-module classifier.
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);
            if elixir::predicates::is_external_elixir_module(root) {
                return Some(root.to_string());
            }
        }

        None
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
