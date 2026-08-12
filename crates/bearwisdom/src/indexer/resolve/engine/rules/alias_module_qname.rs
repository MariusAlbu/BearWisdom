// =============================================================================
// engine/rules/alias_module_qname — bare import alias bound by module qname
//
// A namespace-qualified import (`alias MyApp.Foo`) brings the bare name `Foo`
// into scope bound to the module symbol whose qname IS the import's full module
// path. When an import's `imported_name` equals the bare target, the answer is
// the symbol keyed by the import's `module_path` in the qname index.
//
// Gated on `profile.imports.alias_module_qname`; `false` (the default) passes
// immediately. Only fires for bare (non-dotted) targets.
// =============================================================================

use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct AliasModuleQnameRule;

impl LookupRule for AliasModuleQnameRule {
    fn name(&self) -> &'static str {
        "alias_module_qname"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.imports.alias_module_qname {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty()
            || target.contains('.')
            || target.contains("::")
            || target.contains('/')
        {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        for import in &ctx.file_ctx.imports {
            if import.imported_name != target {
                continue;
            }
            let Some(full_module) = import.module_path.as_deref() else {
                continue;
            };
            if let Some(sym) = ctx.lookup.by_qualified_name(full_module) {
                if (ctx.kind)(edge_kind, &sym.kind) {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_alias_module_qname"),
                    );
                }
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "alias_module_qname_tests.rs"]
mod tests;
