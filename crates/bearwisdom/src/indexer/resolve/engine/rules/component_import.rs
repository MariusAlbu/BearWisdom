// =============================================================================
// engine/rules/component_import — JSX/Vue/Svelte component tag resolution
//
// A `Calls` ref whose target starts with an ASCII-uppercase letter is a JSX or
// template component tag. Walks the file's imports looking for one whose
// `imported_name` or `alias` matches the tag head, then binds a kind-compatible
// symbol whose file path matches the import's module path.
//
// When no explicit name match succeeds, falls back to
// `resolve_component_import_module_symbols`: if the import's module path resolves
// to exactly one component-kind symbol, that symbol wins unambiguously.
// Path aliases are tried last — when the module specifier rewrites to a
// concrete path, symbols in that rewritten file are scanned.
//
// Ungated: runs for every language that emits component-tag Calls refs.
// =============================================================================

use crate::indexer::resolve::engine::support::file_path_matches_module;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::types::EdgeKind;

pub struct ComponentImportRule;

impl LookupRule for ComponentImportRule {
    fn name(&self) -> &'static str {
        "component_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if ctx.edge_kind() != EdgeKind::Calls || !is_component_tag_target(target) {
            return LookupResult::Pass;
        }
        let Some(head) = component_tag_head(target) else {
            return LookupResult::Pass;
        };
        let edge_kind = ctx.edge_kind();

        for import in &ctx.file_ctx.imports {
            let matches_direct = import.imported_name == head;
            let matches_alias = import.alias.as_deref() == Some(head);
            if !matches_direct && !matches_alias {
                continue;
            }
            let Some(module_path) = import.module_path.as_deref() else {
                continue;
            };

            let lookup_name = if matches_alias {
                import.imported_name.as_str()
            } else {
                head
            };

            for sym in ctx.lookup.by_name(lookup_name) {
                if (ctx.kind)(edge_kind, &sym.kind)
                    && file_path_matches_module(&sym.file_path, module_path)
                {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_component_import"),
                    );
                }
            }

            if let Some(res) = resolve_component_import_module_symbols(ctx, module_path) {
                return res;
            }

            if let Some(rewritten) = ctx
                .lookup
                .resolve_path_alias(ctx.ref_ctx.file_package_id, module_path)
            {
                if rewritten != module_path {
                    for sym in ctx.lookup.in_file(&rewritten) {
                        if is_component_file(&sym.file_path)
                            && is_component_symbol_kind(&sym.kind)
                            && (ctx.kind)(edge_kind, &sym.kind)
                        {
                            return LookupResult::Resolved(
                                ctx.resolved(sym.id, "default_component_import"),
                            );
                        }
                    }
                }
            }
        }
        LookupResult::Pass
    }
}

/// Resolves via the module's symbols when there is exactly one component-kind
/// candidate — ambiguity suppresses the result.
fn resolve_component_import_module_symbols(ctx: &BinderContext<'_>, module_path: &str) -> Option<LookupResult> {
    let edge_kind = ctx.edge_kind();
    let mut compatible = ctx
        .lookup
        .in_module_from(&ctx.file_ctx.file_path, module_path)
        .into_iter()
        .filter(|sym| {
            is_component_file(&sym.file_path)
                && is_component_symbol_kind(&sym.kind)
                && (ctx.kind)(edge_kind, &sym.kind)
        });
    let first = compatible.next()?;
    if compatible.next().is_some() {
        return None;
    }
    Some(LookupResult::Resolved(
        ctx.resolved(first.id, "default_component_import"),
    ))
}

fn component_tag_head(target: &str) -> Option<&str> {
    let head = target
        .split(['.', ':', '/'])
        .next()
        .unwrap_or(target)
        .trim();
    (!head.is_empty()).then_some(head)
}

fn is_component_tag_target(target: &str) -> bool {
    component_tag_head(target)
        .and_then(|head| head.chars().next())
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn is_component_file(path: &str) -> bool {
    path.ends_with(".vue") || path.ends_with(".svelte")
}

fn is_component_symbol_kind(kind: &str) -> bool {
    matches!(kind, "class" | "component")
}

#[cfg(test)]
#[path = "component_import_tests.rs"]
mod tests;
