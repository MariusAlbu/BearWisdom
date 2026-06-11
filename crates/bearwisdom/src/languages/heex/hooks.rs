// HEEx language hooks.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::languages::elixir;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

/// Map a Phoenix template path to its co-located `*View` module file.
///
/// A template under `…/templates/<context>/<name>.html.{heex,eex}` is rendered
/// in the scope of `…/views/<context>_view.ex`. Returns the view-module file
/// path when the template sits beneath a `templates/<context>/` directory.
fn colocated_view_file(template_path: &str) -> Option<String> {
    let norm = template_path.replace('\\', "/");
    let segments: Vec<&str> = norm.split('/').collect();
    // Locate `templates/<context>/<file>`: `templates` followed by a context
    // directory and at least the template file.
    let templates_idx = segments.iter().position(|s| *s == "templates")?;
    let context = segments.get(templates_idx + 1)?;
    // The template file must come after the context directory.
    if templates_idx + 2 >= segments.len() {
        return None;
    }
    let mut view_segments: Vec<String> =
        segments[..templates_idx].iter().map(|s| s.to_string()).collect();
    view_segments.push("views".to_string());
    view_segments.push(format!("{context}_view.ex"));
    Some(view_segments.join("/"))
}

pub struct HeexHooks;

impl LanguageEngineHooks for HeexHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);
            if elixir::predicates::is_external_elixir_module(root) {
                return Some(root.to_string());
            }
        }
        None
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "heex".to_string(),
            imports: Vec::<ImportEntry>::new(),
            file_namespace: None,
        })
    }

    /// Bind a bare template helper call to a same-named function in the
    /// template's co-located Phoenix `*View` module. Scope-directed: the
    /// candidate set is the view module's own symbols, never a whole-program
    /// by-name match. Dotted targets are left to the generic external paths.
    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        if target.contains('.') {
            return None;
        }
        let view_file = colocated_view_file(&file_ctx.file_path)?;
        let func = lookup.in_file(&view_file).iter().find(|s| {
            s.name == *target && matches!(s.kind.as_str(), "method" | "function")
        })?;
        Some(Resolution {
            target_symbol_id: func.id,
            confidence: RESOLVED_CONFIDENCE,
            strategy: "heex_colocated_view_fn",
            resolved_yield_type: None,
            flow_emit: None,
        })
    }
}

pub static HEEX_HOOKS: HeexHooks = HeexHooks;
