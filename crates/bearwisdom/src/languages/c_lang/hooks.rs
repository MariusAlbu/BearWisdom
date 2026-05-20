use super::predicates;
use super::resolve::R_PACKAGE_SENTINEL;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct CHooks;

impl LanguageEngineHooks for CHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // R C API symbols inside an R package.
        if file_ctx.file_namespace.as_deref() == Some(R_PACKAGE_SENTINEL)
            && predicates::is_r_c_api_symbol(target)
        {
            return Some("r.c.api".to_string());
        }

        // Include directives — system headers / boost / gtest.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let header = target.trim_matches(|c| c == '<' || c == '>' || c == '"');
            if predicates::is_system_header(header) {
                return Some("stdlib".to_string());
            }
            if header.starts_with("boost/")
                || header.starts_with("gtest/")
                || header.starts_with("gmock/")
            {
                return Some("external".to_string());
            }
            return None;
        }

        // Template parameters get their own namespace.
        if predicates::is_template_param(target) {
            return Some("template_param".to_string());
        }

        // `std::` prefixed names.
        if target.starts_with("std::") || target.starts_with("::std::") {
            return Some("std".to_string());
        }

        // Other known-external namespace prefixes.
        let root = target
            .strip_prefix("::")
            .unwrap_or(target)
            .split("::")
            .next()
            .unwrap_or(target);
        if predicates::is_external_c_namespace(root) {
            return Some(root.to_string());
        }

        None
    }
}

pub static C_HOOKS: CHooks = CHooks;
