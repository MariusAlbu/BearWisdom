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

use super::types::{GenericParamId, PrimKind, TypeArena, TypeId};
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::generics::{substitute, GenericEnv};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::profile::language_profile::{
    AncestorOrder, LanguageProfile, SupertypeDiscovery,
};
use crate::type_checker::subtype::{is_assignable_to_typed_with, SubtypeResult};
use crate::types::{EdgeKind, ParsedFile};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Direct-supertype edges keyed by source TypeId. Each entry holds *direct*
/// parents only; transitive ancestors fall out of repeated lookups during
/// the `walk_up` BFS.
#[derive(Debug, Default)]
pub struct SupertypeGraph {
    edges: FxHashMap<TypeId, Vec<TypeId>>,
    /// Generic arguments supplied on a `child → parent` edge — the
    /// `<User>` in `class UserRepo extends Repository<User>`. Keyed by the
    /// directed edge so an inherited generic method's `T` can bind to the
    /// concrete argument. Absent / empty for non-generic edges. Stored
    /// alongside (not inside) `edges` so the bare-TypeId walk used by
    /// dispatch and subtype checks is unchanged.
    edge_args: FxHashMap<(TypeId, TypeId), Vec<TypeId>>,
    /// Declared generic params of each node (the `<T>` on `class B<T>`), in
    /// declaration order, as `GenericParamId`s. Lets `walk_up_with_args`
    /// bind a node's params to the args that reached it and substitute the
    /// next edge's args through that binding — composing generic arguments
    /// across multiple inheritance hops.
    node_params: FxHashMap<TypeId, Vec<GenericParamId>>,
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

    /// Add a `child → parent` edge that supplies generic arguments — the
    /// `Repository<User>` in `class UserRepo: Repository<User>`. The edge
    /// itself is recorded the same as `add_edge`; `args` are stored on the
    /// side so a member resolved on `parent` can bind the parent's generic
    /// parameters positionally. Empty `args` is identical to `add_edge`.
    pub fn add_edge_generic(&mut self, child: TypeId, parent: TypeId, args: Vec<TypeId>) {
        self.add_edge(child, parent);
        if !args.is_empty() {
            self.edge_args.insert((child, parent), args);
        }
    }

    /// Record a node's declared generic params (declaration order) so the
    /// arg-carrying walk can compose substitutions through it. Empty params
    /// are ignored — a non-generic node composes as the identity.
    pub fn record_node_params(&mut self, node: TypeId, params: Vec<GenericParamId>) {
        if !params.is_empty() {
            self.node_params.insert(node, params);
        }
    }

    /// BFS like `walk_up`, but each yielded ancestor is paired with the
    /// generic arguments that reach it — **composed across hops**. At each
    /// node the args that reached it bind the node's declared params
    /// (`node_params`); the next edge's args are substituted through that
    /// binding before being queued. So `A: B<X>, B<T>: C<T>` reaches `C` with
    /// `[X]`, not the unbound `[Generic(B::T)]`. The start node and
    /// non-generic edges carry empty args; a node with no recorded params
    /// composes as the identity (the single-level `Subclass: Generic<Concrete>`
    /// case is unchanged — its edge args are already concrete).
    pub fn walk_up_with_args(&self, start: TypeId, arena: &TypeArena) -> Vec<(TypeId, Vec<TypeId>)> {
        let mut out = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back((start, Vec::new()));
        let mut seen = FxHashSet::default();
        seen.insert(start);
        while let Some((node, node_args)) = queue.pop_front() {
            // Bind node's params to the args that reached it, so a deeper
            // edge's args (which may name node's params) resolve to the
            // concrete root arguments.
            let mut env = GenericEnv::new();
            if let Some(params) = self.node_params.get(&node) {
                if !node_args.is_empty() {
                    env.bind_positional(params, &node_args);
                }
            }
            for parent in self.parents_of(node) {
                if seen.insert(*parent) {
                    let composed: Vec<TypeId> = self
                        .edge_args
                        .get(&(node, *parent))
                        .map(|raw| raw.iter().map(|&a| substitute(a, &env, arena)).collect())
                        .unwrap_or_default();
                    queue.push_back((*parent, composed));
                }
            }
            out.push((node, node_args));
        }
        out
    }

    /// Arg-carrying ancestor walk under a chosen `order`. The `Bfs` arm is the
    /// existing `walk_up_with_args` verbatim — every breadth-first / default
    /// language is byte-identical. The `C3` arm yields the SAME
    /// `(TypeId, args)` pairs (same node set, same composed args) reordered
    /// into C3 linearization (Python's MRO), so an asymmetric diamond resolves
    /// the override the runtime would.
    ///
    /// The walk stays TOTAL under either order: `C3` yields exactly the node
    /// set `Bfs` reaches. When the C3 merge cannot select a head (an
    /// inconsistent hierarchy), the unresolvable remainder is appended in BFS
    /// order rather than dropped — the safe degrade-to-BFS. A parent present
    /// as an edge target but absent as a graph node (an external / unindexed
    /// base) participates as a leaf, matching BFS reachability.
    pub fn linearize_with_args(
        &self,
        start: TypeId,
        arena: &TypeArena,
        order: AncestorOrder,
    ) -> Vec<(TypeId, Vec<TypeId>)> {
        let bfs = self.walk_up_with_args(start, arena);
        match order {
            AncestorOrder::Bfs => bfs,
            AncestorOrder::C3 => {
                // Composed args per node come from the BFS pass (first-arrival),
                // so C3 changes only the visit ORDER, never how generic args
                // compose across hops.
                let mut args_of: FxHashMap<TypeId, Vec<TypeId>> = FxHashMap::default();
                for (node, args) in &bfs {
                    args_of.entry(*node).or_insert_with(|| args.clone());
                }
                let order = self.c3_order(start, &bfs);
                order
                    .into_iter()
                    .map(|node| {
                        let args = args_of.remove(&node).unwrap_or_default();
                        (node, args)
                    })
                    .collect()
            }
        }
    }

    /// C3 linearization order over the nodes `bfs` reached. `bfs` fixes the
    /// total node set (so the result is total) and supplies the BFS-order
    /// fallback for any node the C3 merge can't place. Direct parents are read
    /// from `parents_of` in declaration (`add_edge` push) order — exactly the
    /// local precedence list C3 merges against.
    fn c3_order(&self, start: TypeId, bfs: &[(TypeId, Vec<TypeId>)]) -> Vec<TypeId> {
        let bfs_order: Vec<TypeId> = bfs.iter().map(|(n, _)| *n).collect();
        let mut memo: FxHashMap<TypeId, Vec<TypeId>> = FxHashMap::default();
        let mut on_stack = FxHashSet::default();
        let lin = self.c3_linearize(start, &bfs_order, &mut memo, &mut on_stack);
        // Totality guard: append any BFS-reachable node the merge dropped
        // (only possible on an inconsistent hierarchy) in BFS order.
        let mut seen: FxHashSet<TypeId> = lin.iter().copied().collect();
        let mut out = lin;
        for n in bfs_order {
            if seen.insert(n) {
                out.push(n);
            }
        }
        out
    }

    /// L(node) = node :: merge(L(p1), …, L(pk), [p1, …, pk]). `bfs_order` is the
    /// fallback when the merge stalls; `on_stack` breaks cycles (a node already
    /// being linearized contributes only itself, matching `walk_up`'s
    /// cycle-suppression).
    fn c3_linearize(
        &self,
        node: TypeId,
        bfs_order: &[TypeId],
        memo: &mut FxHashMap<TypeId, Vec<TypeId>>,
        on_stack: &mut FxHashSet<TypeId>,
    ) -> Vec<TypeId> {
        if let Some(cached) = memo.get(&node) {
            return cached.clone();
        }
        if !on_stack.insert(node) {
            return vec![node];
        }
        let parents = self.parents_of(node);
        let mut seqs: Vec<Vec<TypeId>> = parents
            .iter()
            .map(|p| self.c3_linearize(*p, bfs_order, memo, on_stack))
            .collect();
        if !parents.is_empty() {
            seqs.push(parents.to_vec());
        }
        let mut result = vec![node];
        result.extend(c3_merge(seqs, bfs_order));
        on_stack.remove(&node);
        memo.insert(node, result.clone());
        result
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
    /// - `Structural` — for each declared interface, adds a `candidate →
    ///   interface` edge for every class whose direct member set
    ///   structurally satisfies the interface under the sound INFER-5
    ///   assignability check (member name + kind + assignable param/return
    ///   types). Go.
    /// - `Both` — runs explicit then structural; idempotent inserts mean
    ///   the two passes coexist without duplication. TypeScript.
    ///
    /// `symbol_types` backs the structural arm's member-type comparison; the
    /// profile's `primitive_mapping` lets it recognize nominal primitive names
    /// as disjoint. Both are inert for `Explicit`.
    pub fn build(
        parsed: &[ParsedFile],
        arena: &TypeArena,
        profile: &LanguageProfile,
        members: &MembersIndex,
        symbol_types: &SymbolTypeMap,
        lookup: &dyn SymbolLookup,
    ) -> Self {
        let mut graph = SupertypeGraph::new();
        match profile.supertype_discovery {
            SupertypeDiscovery::Explicit => {
                build_explicit(&mut graph, parsed, arena, lookup);
            }
            SupertypeDiscovery::Structural => {
                build_structural(&mut graph, arena, members, symbol_types, lookup, profile.primitive_mapping);
            }
            SupertypeDiscovery::Both => {
                build_explicit(&mut graph, parsed, arena, lookup);
                build_structural(&mut graph, arena, members, symbol_types, lookup, profile.primitive_mapping);
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
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) {
    for pf in parsed {
        // EXT-3: external files ARE included. A project class extending an
        // external base (`class UserRepo extends Repository<User>`) needs the
        // external base's OWN supertype edges here so `walk_up_with_args`
        // climbs the external hierarchy and composes the `<User>` arg across
        // those hops — resolving inherited members on deep external bases. The
        // set is reachability-bounded (only externals the resolve loop pulled),
        // so this is not the eager whole-dep-tree walk an `ext:` skip would
        // guard against.
        for r in &pf.refs {
            if !matches!(r.kind, EdgeKind::Inherits | EdgeKind::Implements) {
                continue;
            }
            let Some(source_sym) = pf.symbols.get(r.source_symbol_index) else {
                continue;
            };
            let child = arena.class(&source_sym.qualified_name);
            // Decompose a generic parent (`Repository<User>`) into its base
            // class + arguments so the args can bind the parent's params on
            // an inherited generic method. A bare parent decodes to `Class`
            // with no args — identical to the previous behavior.
            let parent_raw = arena.intern_type_str(&r.target_name);
            let (parent_base_qname, parent_args) = match arena.get(parent_raw) {
                crate::type_checker::core::types::Type::Apply { base, args } => {
                    match arena.get(base) {
                        crate::type_checker::core::types::Type::Class(q) => (q, args),
                        _ => (r.target_name.clone(), Vec::new()),
                    }
                }
                _ => (r.target_name.clone(), Vec::new()),
            };
            // Rebind any arg that names the CHILD's own generic param to the
            // canonical `Type::Generic`, so multi-level inheritance composes:
            // in `class B<T> extends C<T>` the arg `T` is B's param, not a
            // class. `walk_up_with_args` then substitutes it through B's
            // binding. Concrete args (`Repository<User>`) are untouched —
            // "User" isn't one of the child's params.
            let parent_args = if parent_args.is_empty() {
                parent_args
            } else {
                let child_params = child_param_map(&source_sym.qualified_name, arena, lookup);
                if child_params.is_empty() {
                    parent_args
                } else {
                    parent_args
                        .iter()
                        .map(|&a| arena.rebind_class_params(a, &child_params))
                        .collect()
                }
            };
            let parent_qname = resolve_target_qname(&parent_base_qname, lookup);
            let parent = arena.class(parent_qname.as_str());
            graph.add_edge_generic(child, parent, parent_args);

            // Record the child's declared params so `walk_up_with_args` can
            // compose this edge's args through the child's binding at deeper
            // hops.
            if let Some(ids) = lookup.generic_param_type_ids(&source_sym.qualified_name) {
                let params: Vec<GenericParamId> = ids
                    .iter()
                    .filter_map(|&id| match arena.get(id) {
                        crate::type_checker::core::types::Type::Generic { param } => Some(param),
                        _ => None,
                    })
                    .collect();
                graph.record_node_params(child, params);
            }
        }
    }
}

/// The C3 merge: repeatedly take the head of the first sequence that does NOT
/// appear in the tail (any non-head position) of any sequence, remove it from
/// every sequence, and append it. `_bfs_order` is unused once a good head
/// exists; it documents that an inconsistent merge degrades to BFS order
/// upstream. Returns the merged prefix and stops at the first stall — the
/// caller appends the unmerged remainder in BFS order, keeping the walk total.
fn c3_merge(mut seqs: Vec<Vec<TypeId>>, _bfs_order: &[TypeId]) -> Vec<TypeId> {
    let mut out = Vec::new();
    loop {
        seqs.retain(|s| !s.is_empty());
        if seqs.is_empty() {
            return out;
        }
        // A valid head is the front of some sequence that is not in the tail
        // (index > 0) of any sequence.
        let head = seqs.iter().find_map(|seq| {
            let candidate = seq[0];
            let in_some_tail = seqs
                .iter()
                .any(|other| other.iter().skip(1).any(|&t| t == candidate));
            if in_some_tail {
                None
            } else {
                Some(candidate)
            }
        });
        let Some(head) = head else {
            // Inconsistent hierarchy — no candidate is a valid head. Stop;
            // the caller appends the remainder in BFS order.
            return out;
        };
        out.push(head);
        for seq in &mut seqs {
            seq.retain(|&t| t != head);
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

/// `{param-name → Type::Generic id}` for a type's own declared generic params,
/// from the canonical `generic_param_type_ids`. Used to rebind an inheritance
/// edge's args (`extends C<T>`) where `T` names the child's own param.
fn child_param_map(
    child_qname: &str,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) -> FxHashMap<String, TypeId> {
    let Some(ids) = lookup.generic_param_type_ids(child_qname) else {
        return FxHashMap::default();
    };
    ids.iter()
        .map(|&id| (arena.format_type(id), id))
        .collect()
}

/// For each interface-like type, add a `candidate → interface` edge for every
/// class whose direct member set structurally satisfies the interface. Go and
/// Crystal-style structural typing.
///
/// Satisfaction is the sound INFER-5 check ([`is_assignable_to_typed_with`]):
/// the candidate must carry every interface member with a matching name, a
/// matching kind, AND a member type assignable in the correct variance (method
/// returns covariant, params contravariant, fields covariant). The edge is
/// added ONLY on `SubtypeResult::Yes`. A matched member whose param/return
/// TypeIds the extractor didn't record leaves the check at `Unknown`, so no
/// edge forms — declaring satisfaction on name+kind alone (ignoring signatures)
/// would falsely link two structs whose same-named method has incompatible
/// types, which this check exists to avoid.
///
/// Internal-only falls out for free: `MembersIndex` skips `ext:` files, so an
/// external interface enumerates to an empty member set → `Unknown` → no edge.
/// External structural satisfaction stays gated on member hydration.
fn build_structural(
    graph: &mut SupertypeGraph,
    arena: &TypeArena,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    lookup: &dyn SymbolLookup,
    prims: &[(&str, PrimKind)],
) {
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

        for candidate in &typed_ids {
            if candidate == iface {
                continue;
            }
            if members.direct_of(*candidate).is_empty() {
                continue;
            }
            // candidate is the source, iface the target: candidate satisfies
            // iface when it's assignable to iface's shape.
            if is_assignable_to_typed_with(
                *candidate,
                *iface,
                arena,
                lookup,
                members,
                symbol_types,
                prims,
            ) == SubtypeResult::Yes
            {
                graph.add_edge(*candidate, *iface);
            }
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

/// Candidate space for structural matching: every TypeId with at least one
/// direct member. Bounded by the workspace's typed-symbol count.
fn collect_typed_ids(_arena: &TypeArena, members: &MembersIndex) -> Vec<TypeId> {
    members.direct_keys().collect()
}

#[cfg(test)]
#[path = "supertype_tests.rs"]
mod tests;
