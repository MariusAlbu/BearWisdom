// =============================================================================
// engine/parallel_pass — the per-file parallel resolution sweep
//
// Files are independent units of work; refs WITHIN a file stay ordered so
// forward flow inference (a local's type recorded by an earlier ref is
// visible to a later ref) is deterministic. The tree is read-only here,
// shared across workers by ref.
//
// `only_kinds` narrows the sweep to a ref-kind subset. The inherits pre-pass
// uses it to bind `Inherits`/`Implements` refs ahead of the main sweep, so
// their RESOLVED parent ids can feed the inheritance map the member walks
// climb — identity from resolution, not string re-derivation.
// =============================================================================

use std::collections::HashMap;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::languages::LanguagePlugin;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::indexer::write::SymbolIds;
use crate::types::{EdgeKind, ParsedFile};

use super::compilation::Compilation;
use super::flush::{Edge, RefLog, Unresolved};
use super::pipeline::resolve_one_file;
use super::semantic_model::SemanticModel;
use crate::indexer::plugin_state::PluginStateBag;

/// The ref kinds whose resolutions feed the inheritance map.
pub(super) const INHERIT_KINDS: &[EdgeKind] = &[EdgeKind::Inherits, EdgeKind::Implements];

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    parsed: &[ParsedFile],
    tree: &Compilation,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
    plugins: &FxHashMap<&'static str, &'static dyn LanguagePlugin>,
    plugin_state: Option<&PluginStateBag>,
    solver: &SemanticModel,
    symbol_id_map: &SymbolIds,
    only_kinds: Option<&[EdgeKind]>,
) -> (Vec<Edge>, Vec<Unresolved>, Vec<RefLog>) {
    let per_file: Vec<(Vec<Edge>, Vec<Unresolved>, Vec<RefLog>)> = parsed
        .par_iter()
        .filter(|pf| !pf.path.starts_with("ext:"))
        .map(|pf| {
            resolve_one_file(
                pf,
                tree,
                profiles,
                plugins,
                plugin_state,
                solver,
                symbol_id_map,
                only_kinds,
            )
        })
        .collect();

    let mut edges: Vec<Edge> = Vec::new();
    let mut unresolved: Vec<Unresolved> = Vec::new();
    let mut ref_log: Vec<RefLog> = Vec::new();
    for (e, u, r) in per_file {
        edges.extend(e);
        unresolved.extend(u);
        ref_log.extend(r);
    }
    (edges, unresolved, ref_log)
}
