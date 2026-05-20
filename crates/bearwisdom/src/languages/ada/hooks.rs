use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct AdaHooks;

impl LanguageEngineHooks for AdaHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let root = target.split('.').next().unwrap_or(target);

        // Ada standard library packages.
        if matches!(root, "Ada" | "System" | "Interfaces" | "GNAT" | "Standard") {
            return Some(root.to_string());
        }

        // Ada language-defined predefined numeric and Wide_* types.
        if !target.contains('.')
            && matches!(
                root,
                "Long_Integer"
                    | "Long_Long_Integer"
                    | "Short_Integer"
                    | "Short_Short_Integer"
                    | "Integer_8"
                    | "Integer_16"
                    | "Integer_32"
                    | "Integer_64"
                    | "Unsigned_8"
                    | "Unsigned_16"
                    | "Unsigned_32"
                    | "Unsigned_64"
                    | "Long_Float"
                    | "Long_Long_Float"
                    | "Short_Float"
                    | "Duration"
                    | "Wide_Character"
                    | "Wide_Wide_Character"
                    | "Wide_String"
                    | "Wide_Wide_String"
            )
        {
            return Some("Standard".to_string());
        }
        None
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
}

pub static ADA_HOOKS: AdaHooks = AdaHooks;
