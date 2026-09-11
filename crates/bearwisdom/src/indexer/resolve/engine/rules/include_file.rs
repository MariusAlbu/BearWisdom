// =============================================================================
// rules/include_file — an include that names an indexed file is bound to that
// file, not to a symbol
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::types::EdgeKind;

/// `#include "curl_setup.h"` denotes a file. When the owning plugin resolves
/// the spelling to an indexed file, the ref is bound at the file level (the
/// include closure carries that edge) and leaves the symbol ladder drained,
/// so no later rung fabricates a declaration out of a file name. An include
/// the plugin cannot place stays on the ladder and dies honestly.
pub struct IncludeFileRule;

impl LookupRule for IncludeFileRule {
    fn name(&self) -> &'static str {
        "include_file"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let reference = ctx.r();
        if !reference.is_include || reference.kind != EdgeKind::Imports {
            return LookupResult::Pass;
        }
        let spec = reference
            .module
            .as_deref()
            .unwrap_or(reference.target_name.as_str());
        if ctx
            .lookup
            .include_spec_resolves(&ctx.file_ctx.file_path, spec)
        {
            LookupResult::Drained
        } else {
            LookupResult::Pass
        }
    }
}

#[cfg(test)]
#[path = "include_file_tests.rs"]
mod tests;
