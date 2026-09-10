//! Per-file extraction metadata retained through resolution.
use crate::types::{DiscriminantNarrowing, Narrowing};
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct FlowMeta {
    /// Syntax-derived lexical identity. None means this language/form is not migrated.
    pub lexical: Option<crate::indexer::lexical::LexicalBindings>,
    /// Callback-only lexical identity for languages that do not yet opt into
    /// the full source lexical graph. It contains only lambda parameter
    /// declarations and extracted callback-body root references.
    pub callback_lexical: Option<crate::indexer::lexical::LexicalBindings>,
    pub namespaces: Option<crate::indexer::namespaces::NamespaceData>,
    pub narrowings: Vec<Narrowing>,
    pub discriminant_narrowings: Vec<DiscriminantNarrowing>,
    pub flow_binding_lhs: HashMap<usize, usize>,
    /// Destructured bindings of an RHS expression: `const { a, b: c } = f()`.
    /// Maps the RHS `ref_idx` to each binding's `(lhs_symbol_idx, field_key)` —
    /// `a` → `(idx_a, "a")`, `b: c` → `(idx_c, "b")`. Distinct from
    /// `flow_binding_lhs` because each binding types from the named FIELD on the
    /// RHS's yield type (`R["a"]`), not from the whole object `R`. A single RHS
    /// ref carries one entry per destructured binding.
    pub flow_binding_destructure: HashMap<usize, Vec<(usize, String)>>,
    /// Set of `ref_idx` (the destructure RHS's own ref, the same key
    /// `flow_binding_destructure` uses) whose initializer is an `await`
    /// expression: `const { data } = await p.refetch()`. Unlike
    /// `flow_binding_await` — keyed per LHS symbol, one binding — a destructure
    /// RHS is shared across every bound field, so the await flag is recorded
    /// once per ref rather than per binding. The resolver strips one
    /// async-wrapper layer off the RHS's yield type before projecting each
    /// destructured field, the same peel `flow_binding_await` drives for a
    /// single-identifier binding.
    pub flow_binding_destructure_await: std::collections::HashSet<usize>,
    pub flow_binding_decl_type: HashMap<usize, String>,
    pub flow_binding_unwrap: std::collections::HashSet<usize>,
    pub flow_binding_await: std::collections::HashSet<usize>,
    pub flow_return_lhs: HashMap<usize, usize>,
    /// `(fn_symbol_idx, identifier)` for a `return <bare-identifier>` whose
    /// expression carries no ref — `return queryClient` / `return client`. The
    /// ref-based `flow_return_lhs` misses these (a bare param/local read emits no
    /// ref), so the resolver types the identifier against the function's
    /// parameters / locals and records the result as a return-type candidate.
    pub flow_return_ident: Vec<(usize, String)>,
    /// `(fn_symbol_idx, member_names)` for a function whose body returns an object
    /// literal (`return { info, error }`). A synthetic `{fn}$Ret` object type
    /// carrying these members is materialized post-extract, and a call to the
    /// function yields that type — so `fn().info` resolves to the synthesized member.
    pub flow_return_object: Vec<(usize, Vec<String>)>,
    pub ref_byte_offsets: Vec<u32>,
    /// Per-function control-flow graphs for the file, built at extract time
    /// from the same tree the query runner uses. Empty when the language has
    /// no `CfgNodeKinds` table wired yet — the consumer falls back to the
    /// interval `narrowings` path. Queried by `LocalTypeCache::lookup` via
    /// `fact_string_at(name, cursor)`.
    pub cfg: crate::indexer::flow_cfg::FileCfg,
}

#[cfg(test)]
#[path = "flow_meta_tests.rs"]
mod tests;
