use super::resolve::is_delphi_namespaced_file;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct PascalHooks;

impl LanguageEngineHooks for PascalHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Pascal is case-insensitive — keyword check first.
        let target = &ref_ctx.extracted_ref.target_name;
        let target_lower = target.to_lowercase();
        let keywords = super::keywords::KEYWORDS;
        if keywords.iter().any(|k| k.to_lowercase() == target_lower) {
            return Some("primitive".to_string());
        }

        // Probe FPC runtime walker's ext:pascal: virtual paths with multiple
        // casings (by_name is case-sensitive; FPC source canonical casing
        // varies — `NativeInt`, `HRESULT`, `integer`).
        let target_upper = target.to_uppercase();
        let target_title: String = {
            let mut c = target.chars();
            match c.next() {
                None => String::new(),
                Some(f) => {
                    f.to_uppercase().collect::<String>() + &target_lower[f.len_utf8()..]
                }
            }
        };
        for probe in [
            target.as_str(),
            target_lower.as_str(),
            target_title.as_str(),
            target_upper.as_str(),
        ] {
            for sym in lookup.by_name(probe) {
                if sym.file_path.starts_with("ext:pascal:")
                    && sym.name.to_lowercase() == target_lower
                {
                    return Some("fpc-runtime".to_string());
                }
            }
        }

        // Delphi namespaced files: any unresolved ref is a Delphi SDK or VCL
        // symbol.
        if is_delphi_namespaced_file(file_ctx) {
            return Some("delphi-vcl".to_string());
        }

        None
    }
}

pub static PASCAL_HOOKS: PascalHooks = PascalHooks;
