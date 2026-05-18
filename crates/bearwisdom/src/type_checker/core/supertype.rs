// =============================================================================
// type_checker/core/supertype.rs — TypeId-keyed supertype graph
//
// Single graph for all "this type extends / implements / structurally
// supersets" edges. Member lookup walks this graph BFS; subtype check
// consults it for class-to-class assignability. One graph instead of
// separate inherits / implements maps because every consumer that walks it
// needs both kinds of edge at the same time — splitting them just doubles
// the walk cost.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 3
//       research/architecture/04-implementation-phases.html § Phase 3
// =============================================================================

use super::types::{TypeArena, TypeId};
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::profile::language_profile::{LanguageProfile, SupertypeDiscovery};
use crate::types::{EdgeKind, ParsedFile};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Direct-supertype edges keyed by source TypeId. Each entry holds *direct*
/// parents only; transitive ancestors fall out of repeated lookups during
/// the `walk_up` BFS.
#[derive(Debug, Default)]
pub struct SupertypeGraph {
    edges: FxHashMap<TypeId, Vec<TypeId>>,
}

impl SupertypeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.values().map(|v| v.len()).sum()
    }

    pub fn node_count(&self) -> usize {
        self.edges.len()
    }

    /// Add a direct supertype edge `child` → `parent`. Idempotent — repeats
    /// don't duplicate entries, so callers that run multiple discovery
    /// passes (e.g. Both = Explicit + Structural) don't need to dedupe
    /// before inserting.
    pub fn add_edge(&mut self, child: TypeId, parent: TypeId) {
        let parents = self.edges.entry(child).or_default();
        if !parents.contains(&parent) {
            parents.push(parent);
        }
    }

    /// Direct parents of `ty`. Empty slice when none recorded.
    pub fn parents_of(&self, ty: TypeId) -> &[TypeId] {
        self.edges.get(&ty).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// BFS over the supertype graph from `start`. Yields `start` first, then
    /// every transitive ancestor in breadth-first order with duplicates
    /// suppressed. Caller stops at the first match for any "find on
    /// nearest supertype" query.
    ///
    /// The iterator is allocation-bounded by the visited set's size, which
    /// is bounded by the graph's node count.
    pub fn walk_up(&self, start: TypeId) -> SupertypeWalk<'_> {
        let mut queue = VecDeque::new();
        queue.push_back(start);
        let mut seen = FxHashSet::default();
        seen.insert(start);
        SupertypeWalk {
            graph: self,
            queue,
            seen,
        }
    }

    /// Build the supertype graph from extraction output.
    ///
    /// Dispatches on `profile.supertype_discovery`:
    /// - `Explicit` — reads `Inherits` and `Implements` refs from every
    ///   parsed file. The vast majority of nominal-typed languages.
    /// - `Structural` — for each declared interface, marks every class
    ///   whose direct member set superset-matches the interface's
    ///   member set as a structural subtype. Go.
    /// - `Both` — runs explicit then structural; idempotent inserts mean
    ///   the two passes coexist without duplication. TypeScript.
    pub fn build(
        parsed: &[ParsedFile],
        arena: &mut TypeArena,
        profile: &LanguageProfile,
        members: &MembersIndex,
        lookup: &dyn SymbolLookup,
    ) -> Self {
        let mut graph = SupertypeGraph::new();
        match profile.supertype_discovery {
            SupertypeDiscovery::Explicit => {
                build_explicit(&mut graph, parsed, arena, lookup);
            }
            SupertypeDiscovery::Structural => {
                build_structural(&mut graph, arena, members);
            }
            SupertypeDiscovery::Both => {
                build_explicit(&mut graph, parsed, arena, lookup);
                build_structural(&mut graph, arena, members);
            }
        }
        graph
    }
}

/// BFS iterator returned by `walk_up`. Yields each TypeId exactly once in
/// breadth-first order, starting at the queried node.
pub struct SupertypeWalk<'a> {
    graph: &'a SupertypeGraph,
    queue: VecDeque<TypeId>,
    seen: FxHashSet<TypeId>,
}

impl<'a> Iterator for SupertypeWalk<'a> {
    type Item = TypeId;

    fn next(&mut self) -> Option<TypeId> {
        let next = self.queue.pop_front()?;
        for parent in self.graph.parents_of(next) {
            if self.seen.insert(*parent) {
                self.queue.push_back(*parent);
            }
        }
        Some(next)
    }
}

fn build_explicit(
    graph: &mut SupertypeGraph,
    parsed: &[ParsedFile],
    arena: &mut TypeArena,
    lookup: &dyn SymbolLookup,
) {
    for pf in parsed {
        for r in &pf.refs {
            if !matches!(r.kind, EdgeKind::Inherits | EdgeKind::Implements) {
                continue;
            }
            let Some(source_sym) = pf.symbols.get(r.source_symbol_index) else {
                continue;
            };
            let child = arena.class(&source_sym.qualified_name);
            let parent_qname = resolve_target_qname(&r.target_name, lookup);
            let parent = arena.class(parent_qname.as_str());
            graph.add_edge(child, parent);
        }
    }
}

/// Resolve a ref's `target_name` to the qualified name of an indexed type
/// when possible. Returns `target_name` unchanged when no unique type symbol
/// exists for it — the chain walker / resolve loop will treat the unmatched
/// edge as an external supertype, which is the same fallback the legacy
/// `inherits_map` builder used.
fn resolve_target_qname(target_name: &str, lookup: &dyn SymbolLookup) -> String {
    let candidates = lookup.types_by_name(target_name);
    if candidates.len() == 1 {
        return candidates[0].qualified_name.clone();
    }
    target_name.to_string()
}

/// For each `Interface` symbol, mark every class whose direct member set is a
/// superset of the interface's member set as a structural subtype. Go and
/// Crystal-style structural typing.
///
/// The check is on member names + kind only (not parameter shapes) — that
/// matches what the engine can recover from extraction without re-running
/// signature parsing. False positives are extremely rare in practice
/// because interface names tend to be intentional API boundaries with
/// few methods.
fn build_structural(graph: &mut SupertypeGraph, arena: &TypeArena, members: &MembersIndex) {
    // Collect every type that has direct members. We need O(types) work
    // here, and the loop is bounded by the workspace's type count.
    let typed_ids: Vec<TypeId> = collect_typed_ids(arena, members);

    for iface in &typed_ids {
        let iface_members = members.direct_of(*iface);
        if iface_members.is_empty() {
            continue;
        }
        if !is_interface_like(iface_members) {
            continue;
        }
        let required: Vec<(&str, &str)> = iface_members
            .iter()
            .map(|m| (m.name.as_str(), m.kind.as_str()))
            .collect();

        for candidate in &typed_ids {
            if candidate == iface {
                continue;
            }
            let candidate_members = members.direct_of(*candidate);
            if candidate_members.is_empty() {
                continue;
            }
            if !candidate_satisfies(candidate_members, &required) {
                continue;
            }
            graph.add_edge(*candidate, *iface);
        }
    }
}

/// A type is "interface-like" for structural matching when its direct
/// members are all method / function kinds. Classes with fields can't be
/// matched structurally because the field signature isn't surfaced.
fn is_interface_like(syms: &[SymbolInfo]) -> bool {
    syms.iter()
        .all(|s| matches!(s.kind.as_str(), "method" | "function"))
}

fn candidate_satisfies(candidate_members: &[SymbolInfo], required: &[(&str, &str)]) -> bool {
    required.iter().all(|(name, kind)| {
        candidate_members
            .iter()
            .any(|m| m.name == *name && m.kind == *kind)
    })
}

/// Candidate space for structural matching: every TypeId with at least one
/// direct member. Bounded by the workspace's typed-symbol count.
fn collect_typed_ids(_arena: &TypeArena, members: &MembersIndex) -> Vec<TypeId> {
    members.direct_keys().collect()
}

#[cfg(test)]
#[path = "supertype_tests.rs"]
mod tests;
