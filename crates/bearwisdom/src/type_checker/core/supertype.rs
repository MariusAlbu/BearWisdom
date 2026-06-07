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
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::str::FromStr;

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
                build_explicit(&mut graph, parsed, arena, lookup, profile.blanket_impl_resolution);
            }
            SupertypeDiscovery::Structural => {
                build_structural(&mut graph, arena, members, symbol_types, lookup, profile.primitive_mapping);
            }
            SupertypeDiscovery::Both => {
                build_explicit(&mut graph, parsed, arena, lookup, profile.blanket_impl_resolution);
                build_structural(&mut graph, arena, members, symbol_types, lookup, profile.primitive_mapping);
            }
        }
        graph
    }

    /// Nodes that have at least one direct parent edge — the set of types the
    /// graph knows as a subtype of something. The blanket-impl pass uses this as
    /// its candidate space: a concrete C that satisfies a nominal `impl Bound
    /// for C` necessarily carries a `C → Bound` edge, so any bound-satisfying
    /// candidate is already a child key here. Bounded by the graph's node count.
    fn child_keys(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.edges.keys().copied()
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

/// A deferred blanket-impl container collected in pass 1: the trait it
/// implements (`Trait` in `impl<U: Bound> Trait for U {}`) and the set of bound
/// trait names every candidate must satisfy. Drained in pass 2 against the
/// fully-built concrete graph.
struct BlanketImpl {
    /// The `Implements` edge's target — the trait whose default members the
    /// satisfying candidates should gain.
    trait_name: String,
    /// The bound traits the impl param is constrained by (`Bound` in `<U:
    /// Bound>`, every conjunct of `<U: A + B>`). A candidate must reach ALL of
    /// them. Empty ⇒ an unconditional `impl<T> Trait for T` — declined (see the
    /// pass-2 skip), never populated.
    bound_names: Vec<String>,
}

fn build_explicit(
    graph: &mut SupertypeGraph,
    parsed: &[ParsedFile],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    blanket_impl_resolution: bool,
) {
    // Pass 1: every non-blanket inheritance edge, exactly as the single-pass
    // builder did. A blanket-impl container is deferred (when the gate is on) so
    // its candidate edges can be added against the complete concrete graph.
    let mut blankets: Vec<BlanketImpl> = Vec::new();
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
            // A blanket impl `impl<U: Bound> Trait for U {}` — the source is an
            // impl-container whose self-TypeRef names its OWN generic param `U`.
            // Rerouting that self-TypeRef would resolve `U` to no concrete type
            // (a dead edge), so when the gate is on, defer this container: pass 2
            // adds a `C → Trait` edge for each concrete C that satisfies Bound.
            // Only an `Implements` ref carries the trait; an `Inherits` ref on
            // the same blanket container is not the trait edge and is skipped.
            if blanket_impl_resolution && r.kind == EdgeKind::Implements {
                if let Some(bound_names) =
                    blanket_bound_names(source_sym, r.source_symbol_index, &pf.refs, arena, lookup)
                {
                    blankets.push(BlanketImpl {
                        trait_name: r.target_name.clone(),
                        bound_names,
                    });
                    continue;
                }
            }
            // An inheritance edge whose source symbol is an impl-container
            // (`impl Trait for C` emits a Namespace symbol, not C itself) must
            // attach to the IMPLEMENTING type C, not the container — otherwise
            // the edge keys on a dead node nothing is ever typed to and C never
            // gains the supertype. A Namespace can't be a subtype, so any
            // inheritance edge from one is structurally an impl/extension
            // container whose real child is the type it implements FOR. That
            // type is carried structurally by the container's self-`TypeRef`
            // edge (the implementing-type node text), read here without parsing
            // any language's `impl` syntax. Absent the marker, key on the
            // source's own qname (every other language's classes/traits land
            // here unchanged).
            let child_qname =
                impl_container_child_qname(source_sym, r.source_symbol_index, &pf.refs, arena, lookup)
                    .unwrap_or_else(|| source_sym.qualified_name.clone());
            let child = arena.class(&child_qname);
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
                let child_params = child_param_map(&child_qname, arena, lookup);
                if child_params.is_empty() {
                    parent_args
                } else {
                    parent_args
                        .iter()
                        .map(|&a| arena.rebind_class_params(a, &child_params))
                        .collect()
                }
            };
            let parent_qname =
                resolve_target_qname(&parent_base_qname, lookup, KindPreference::for_supertype(r.kind));
            let parent = arena.class(parent_qname.as_str());
            graph.add_edge_generic(child, parent, parent_args);

            // Record the child's declared params so `walk_up_with_args` can
            // compose this edge's args through the child's binding at deeper
            // hops.
            if let Some(ids) = lookup.generic_param_type_ids(&child_qname) {
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

    // Pass 2: drain the deferred blanket impls against the now-complete concrete
    // graph. Runs only under the gate (the collection is empty otherwise), so a
    // language without blanket-impl resolution is byte-identical to pass 1.
    drain_blankets(graph, &blankets, arena, lookup);
}

/// A blanket impl with its bound/trait names resolved to graph node ids — the
/// graph-independent part of `drain_blankets`, computed once and reused across
/// every fixpoint round. An empty bound set is dropped here (an unconditional
/// `impl<T> Trait for T` is a deliberately-deferred fan-out, never populated).
struct ResolvedBlanket {
    /// The bound traits a candidate must ALL reach (`Bound` ids, the conjunction
    /// of `<U: A + B>`). Each is the canonical node a nominal `impl Bound for C`
    /// edge points at, so `walk_up` reachability lines up.
    bound_ids: Vec<TypeId>,
    /// The trait whose default members satisfying candidates gain.
    trait_id: TypeId,
}

/// Add a `C → Trait` edge for every candidate concrete C that provably
/// satisfies a blanket impl's bounds. C satisfies the bound set when its
/// supertype graph (`walk_up`) reaches EVERY resolved bound trait — a real
/// nominal `impl Bound for C` edge populated in pass 1. An unhydrated or
/// unsatisfied bound yields no reachability, so no edge forms (widening-only,
/// never a coincidental same-name bind).
///
/// Layered blanket impls compose: `impl<U: Display> Greet for U` +
/// `impl<V: Greet> Loud for V` give `Loud` to every Display-bound type, but
/// `Loud`'s `Greet` bound is itself supplied by the first blanket's edge. A
/// single ordered pass would resolve this only when the provider blanket happens
/// to precede the dependent one in collection order; processed the other way the
/// dependent blanket runs while `C → Greet` does not yet exist and is never
/// revisited. The drain therefore runs to a FIXPOINT: re-snapshot the candidate
/// set and re-evaluate reachability each round, halting when a full round adds no
/// edge. Each round resolves at least one further dependency layer, so the loop
/// terminates in at most `blankets.len()` rounds (edges are only ever added, over
/// a finite candidate × blanket space). Soundness is unchanged per round — an
/// edge still forms only through real reached bounds, so transitivity widens
/// strictly through structurally-formed edges, never a coincidental bind.
fn drain_blankets(
    graph: &mut SupertypeGraph,
    blankets: &[BlanketImpl],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) {
    if blankets.is_empty() {
        return;
    }
    // Resolve bound/trait names to node ids once — name→id is graph-independent,
    // so it never changes across rounds. Empty-bound impls are dropped here.
    let resolved: Vec<ResolvedBlanket> = blankets
        .iter()
        .filter(|b| !b.bound_names.is_empty())
        .map(|blanket| {
            let bound_ids: Vec<TypeId> = blanket
                .bound_names
                .iter()
                .map(|name| {
                    let qname = resolve_target_qname(
                        name,
                        lookup,
                        KindPreference::Supertype(EdgeKind::Implements),
                    );
                    arena.class(qname.as_str())
                })
                .collect();
            let trait_qname = resolve_target_qname(
                &blanket.trait_name,
                lookup,
                KindPreference::Supertype(EdgeKind::Implements),
            );
            ResolvedBlanket {
                bound_ids,
                trait_id: arena.class(trait_qname.as_str()),
            }
        })
        .collect();
    if resolved.is_empty() {
        return;
    }

    // At most one new dependency layer resolves per round; cap the fixpoint at
    // the blanket count so a pathological cycle can't loop unbounded.
    for _ in 0..resolved.len() {
        // Re-snapshot each round — an edge added last round can make a node a
        // child key (a candidate) and grows the edge set `walk_up` reads.
        let candidates: Vec<TypeId> = graph.child_keys().collect();
        let mut added = false;
        for blanket in &resolved {
            for &candidate in &candidates {
                // Don't attach a trait to itself or to its own bound traits, and
                // skip a candidate that already carries the edge (idempotent
                // add, but the explicit check keeps the `added` flag honest).
                if candidate == blanket.trait_id
                    || blanket.bound_ids.contains(&candidate)
                    || graph.parents_of(candidate).contains(&blanket.trait_id)
                {
                    continue;
                }
                let reached: FxHashSet<TypeId> = graph.walk_up(candidate).collect();
                if blanket.bound_ids.iter().all(|b| reached.contains(b)) {
                    graph.add_edge(candidate, blanket.trait_id);
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
}

/// The bound trait names of a blanket impl `impl<U: Bound> Trait for U {}`, or
/// `None` when `sym` is not a blanket impl-container.
///
/// A blanket impl is structurally an impl-container (a `Namespace` source on an
/// inheritance edge) whose self-`TypeRef` names one of the container's OWN
/// declared generic params — `impl<U: …> … for U`, where the implementing type
/// IS the param `U`, not a concrete type. A normal `impl Trait for Concrete` has
/// a self-TypeRef target that is a concrete type name, never in the impl's param
/// list, so it returns `None` and takes the existing reroute path unchanged.
///
/// The bounds are the container's sibling `TypeRef`s (same source symbol) MINUS
/// the self-TypeRef (the one naming the param). Both inline `<U: Bound>` and
/// `where U: Bound` forms emit these sibling TypeRefs, so both are captured.
/// `?Sized` emits no TypeRef (it widens, not restricts) and is correctly absent.
fn blanket_bound_names(
    sym: &ExtractedSymbol,
    source_index: usize,
    refs: &[ExtractedRef],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) -> Option<Vec<String>> {
    if sym.kind != SymbolKind::Namespace {
        return None;
    }
    // The container's declared generic params (`["U"]` from the parsed
    // `impl<U: Bound> U` signature). A non-generic impl has none → not blanket.
    let params = lookup.generic_params(&sym.qualified_name)?;
    if params.is_empty() {
        return None;
    }
    // The self-TypeRef names the implementing type — for a blanket impl its base
    // IS one of the container's own params. Identify it independent of emission
    // order so a bound TypeRef emitted before the self-TypeRef can't be mistaken
    // for it.
    let self_ref = refs.iter().find(|r| {
        matches!(r.kind, EdgeKind::TypeRef)
            && r.source_symbol_index == source_index
            && params.contains(&type_base_qname(&r.target_name, arena))
    })?;
    // Every OTHER sibling TypeRef is a bound on the param. The self-TypeRef is
    // matched by-identity so a second TypeRef that coincidentally shares the
    // param's surface form (a bound literally named `U`) is not also dropped.
    let bounds: Vec<String> = refs
        .iter()
        .filter(|r| {
            matches!(r.kind, EdgeKind::TypeRef)
                && r.source_symbol_index == source_index
                && !std::ptr::eq(*r, self_ref)
        })
        .map(|r| type_base_qname(&r.target_name, arena))
        .filter(|b| !b.is_empty())
        .collect();
    Some(bounds)
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

/// The resolved qname of the type an impl-container implements/inherits FOR,
/// or `None` when `sym` is not an impl-container with a recorded implementing
/// type.
///
/// `impl Trait for C` is extracted as a `Namespace` symbol that ALSO emits a
/// `TypeRef` edge from itself to the implementing type (`C`, or `C<T>` when the
/// impl block declares its own params). A Namespace can't itself be a subtype,
/// so an inheritance edge sourced from one is structurally an impl/extension
/// container — its real child is that implementing type, carried by the
/// self-`TypeRef`'s `target_name`, not the container's own `<impl C@N>` qname.
/// The implementing type's base is decomposed through the arena (the same
/// `Apply { base, args }` decode the parent type uses), so a parameterized
/// `C<T>` — including bounded-generic forms — yields the bare base `C` with no
/// `impl`-syntax string parsing here. The base is resolved through
/// `types_by_name` so the edge attaches to C's canonical class node.
fn impl_container_child_qname(
    sym: &ExtractedSymbol,
    source_index: usize,
    refs: &[ExtractedRef],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    if sym.kind != SymbolKind::Namespace {
        return None;
    }
    // The container's self-`TypeRef` (same source symbol as this inheritance
    // edge) names the implementing type. Match by source index so a file with
    // several impl blocks attaches each edge to its own implementing type.
    let impl_type = refs.iter().find(|r| {
        matches!(r.kind, EdgeKind::TypeRef) && r.source_symbol_index == source_index
    })?;
    let base = type_base_qname(&impl_type.target_name, arena);
    if base.is_empty() {
        return None;
    }
    // The implementing type is always a concrete type (class/struct/enum),
    // never the interface/trait it implements — prefer that kind-class if the
    // bare name collides with a same-named interface in the pool.
    Some(resolve_target_qname(&base, lookup, KindPreference::ConcreteType))
}

/// Decompose a type node's text into its base type name, dropping any generic
/// argument list. `C<T>` → `C`, `path::C<T, U>` → `path::C`; a bare `C` is
/// returned unchanged. Uses the arena's `Apply { base, args }` decode so the
/// generic-argument split is the same structural one the parent type uses —
/// no manual angle-bracket scanning that would mishandle nested generics.
fn type_base_qname(text: &str, arena: &TypeArena) -> String {
    let raw = arena.intern_type_str(text);
    match arena.get(raw) {
        crate::type_checker::core::types::Type::Apply { base, .. } => match arena.get(base) {
            crate::type_checker::core::types::Type::Class(q) => q,
            _ => text.to_string(),
        },
        crate::type_checker::core::types::Type::Class(q) => q,
        _ => text.to_string(),
    }
}

/// Which kind-class a `resolve_target_qname` site expects, used to break a
/// bare-name tie among same-short-named type-like candidates. `Supertype`
/// resolves an inheritance edge's parent (an `Implements` parent is a
/// trait/interface; an `Inherits` parent is a base class/struct);
/// `ConcreteType` resolves the implementing type of an impl-container (never an
/// interface). `Any` keeps the prior tie-blind behavior.
#[derive(Clone, Copy)]
enum KindPreference {
    Any,
    Supertype(EdgeKind),
    ConcreteType,
}

impl KindPreference {
    fn for_supertype(edge: EdgeKind) -> Self {
        KindPreference::Supertype(edge)
    }

    /// True when `sym_kind` (a snake_case `SymbolKind`) is the kind-class this
    /// preference selects. `Any` admits every candidate (no tie-break).
    fn matches(self, sym_kind: &str) -> bool {
        let Ok(kind) = SymbolKind::from_str(sym_kind) else {
            return true;
        };
        match self {
            KindPreference::Any => true,
            // `class C: Base` (Inherits) → base is a class/struct; `class C: I`
            // / `impl Trait for C` (Implements) → parent is an interface/trait.
            // Rust supertraits (`trait Sub: Super`) extract as Inherits between
            // traits, so a trait is admitted under Inherits too.
            KindPreference::Supertype(EdgeKind::Implements) => {
                matches!(kind, SymbolKind::Interface | SymbolKind::Trait)
            }
            KindPreference::Supertype(_) => matches!(
                kind,
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Trait
            ),
            KindPreference::ConcreteType => matches!(
                kind,
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Enum
            ),
        }
    }
}

/// Resolve a ref's `target_name` to the qualified name of an indexed type
/// when possible. Returns `target_name` unchanged when no type symbol resolves
/// it — the chain walker / resolve loop treats the unmatched edge as an
/// external supertype, the same fallback the legacy `inherits_map` builder used.
///
/// A unique candidate wins outright. When several type-like symbols share the
/// bare short name (a trait and a same-named struct, the common cross-dep
/// shape), `prefer` narrows to the kind-class the edge expects; a unique
/// survivor of that narrowing wins. This keys the supertype edge under the
/// SAME canonical qname the trait's default-method body is filed under, so the
/// member walk aligns. Still ambiguous after narrowing → the bare-name
/// fallback (declines to a structural hit, never a coincidental bind).
fn resolve_target_qname(
    target_name: &str,
    lookup: &dyn SymbolLookup,
    prefer: KindPreference,
) -> String {
    let candidates = lookup.types_by_name(target_name);
    if candidates.len() == 1 {
        return candidates[0].qualified_name.clone();
    }
    if candidates.len() > 1 {
        let mut preferred = candidates.iter().filter(|c| prefer.matches(&c.kind));
        if let Some(first) = preferred.next() {
            if preferred.next().is_none() {
                return first.qualified_name.clone();
            }
        }
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
/// An external interface participates as a target: `MembersIndex` admits the
/// members of an `ext:` Trait/Interface, so its shape enumerates here and a
/// project type satisfying it gains the edge. The candidate side is project
/// types (external Class/Struct members stay skipped), so this links project
/// type → external interface, not external → external.
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
