# S2 flip — materialize-on-miss + delete expand loop (continuation plan)

Status at writing: S0, S1, S2a committed + gated byte-identical (sql-pgmq 116, goxygen 17).
HEAD = `0273090b` (S2a). This file scopes the remaining S2 work (the "flip").

## The atomic-flip constraint (why S2b/c can't sub-divide byte-identically)

- materialize-on-miss ALONE, with the expand loop still live → externals get pulled
  twice (once lazily, once by expand) → not byte-identical.
- delete expand loop ALONE, no materialize-on-miss → externals never bound → not byte-identical.
- => The two land together in one commit, gated once. S2a (store, empty) was the only
  cleanly-separable byte-identical step.

## Seams (all grounded)

- `parse_external_sources(...)` returns `ExternalParsingResult { parsed: external_parsed,
  symbol_index, .. }` at `full.rs:700`. `symbol_index` is the aggregated
  `SymbolLocationIndex` (`ecosystem/symbol_index.rs`: `(module,name)→Path` + `name→[(module,Path)]`,
  `locate` / `find_by_name`). `external_parsed` = eagerly pre-pulled external files
  (demand_pre_pull) folded into `combined_parsed` (full.rs:855-859) and indexed eagerly.
- The expand loop to delete: `full.rs` ~980-1051 (the `while iteration < MAX_EXPANSION_ITERATIONS`
  block) + `expand::expand_chain_reachability_with_index_and_arena` call. `expand.rs` becomes
  dead (keep `follow_header_includes` logic → generalize as inline type-hop materialization).
- `SymbolIndex::augment_from_parsed` (`index/augment.rs:57`, `&mut self`) is the ParsedFile→
  SymbolInfo+maps conversion to replicate UNDER `&self` for the materialized store:
  Pass 1 basic indexes (by_name/by_qname/by_file/members_by_parent/types_by_name/by_package),
  Pass 2 type_info (field_type/return_type/generic_params). The materialized version interns
  into `MaterializedStore` + parallel type side-maps instead of the eager `&mut` maps.
- `build_with_context` / `_and_arena` (`index/build.rs:52,70`) — thread the `SymbolLocationIndex`
  + `&'static LanguageRegistry` (or `Arc`) into `SymbolIndex` so lookups can parse-on-miss.
  The registry is currently passed to `resolve_iteration_*`, not held by the index.

## Materialize-on-miss design

On a lookup MISS of the eager store (by_name / by_qualified_name / members_of), before
returning empty:
1. `loc.find_by_name(name)` (or `locate(module, leaf)` when the ref carries a module) →
   candidate `(module, file)` list.
2. For each file: `materialized.file_guard(file).get_or_init(|| materialize(file))` — one
   worker parses + interns; others reuse. `materialize`: parse whole file
   (`parse_file_with_demand(walked, registry, None)` — registry + arena, NO DB) → run the
   `&self` augment-equivalent → intern symbols into `MaterializedStore` (+ type side-maps).
3. Re-query the materialized store; return via `SymbolSet::Owned` (eager-first merge — the
   `span_eager_materialized` helper already does this; `by_qualified_name` eager-first too).
- Members come free: materializing a type's file interns its methods under the type's parent
  qname, so a subsequent `members_of(type_qname)` hits the materialized store with no extra
  pull. Inline type-hop (the generalized `follow_header_includes`): when a materialized
  symbol's return/field type names another external type, that type's `by_name`/qname lookup
  during the walk materializes it the same way — the closure unfolds during the walk.

## The hard part — synthetic ids + edge late-binding

Materialized symbols have NO DB id during the pass (no SQLite in rayon). So:
- `SymbolInfo.id` for a materialized symbol = a synthetic handle. Cleanest: store the
  `boxcar` index and tag it (e.g. negative-encoded `-(idx+1)`, or carry `SymbolRef` alongside).
  Prefer threading `SymbolRef` where the resolver records an edge target, rather than
  overloading `i64`.
- The resolve edge buffer (FileWriteBuf / the rayon-collected `combined_buf`) records an edge
  target. Today that's a resolved `i64` id. For a materialized target it must record
  `SymbolRef::Materialized(idx)` and late-bind to a real id at flush.
- Flush (after the single pass): assign DB ids to all materialized symbols, write their
  symbol rows (origin='external'), build idx→id map, rewrite buffered materialized-target
  edges to real ids, `INSERT OR IGNORE`. The edge buffer already defers speculative rows
  (`DeferredSpeculative`) — extend that machinery to carry materialized-target handles.
- AUDIT: every place a resolver turns a `&SymbolInfo` into an edge `target_id` — those must
  accept a materialized symbol's synthetic handle and route it to the late-bind path.
  (grep the resolver edge-emit sites that read `sym.id`.)

## Sequencing inside the flip (build order, single gate at end)

1. Thread `SymbolLocationIndex` + registry into `SymbolIndex` (build_with_context params + fields).
2. `&self` materialize() + the augment-equivalent intern (reuse augment.rs Pass1/Pass2 logic,
   factored to write into MaterializedStore).
3. Wire materialize-on-miss into by_name / by_qualified_name / members_of (and types_by_name,
   in_file as needed).
4. Synthetic-id handling + edge buffer late-bind + deferred materialized-symbol DB write.
5. Delete the expand loop + iteration_n + the now-dead eager external pre-pull IF it would
   double-count (verify: pre-pulled externals are already eager-indexed, so materialize-on-miss
   only fires for the non-pre-pulled set the expand loop used to pull — confirm no overlap).
6. Generalize follow_header_includes as inline type-hop (C #include closure → covered by the
   miss-materialize path for header refs).

## Gate

Cold full reindex, `mode=full`, byte-identical: sql-pgmq 116, goxygen 17.
Then a non-trivial externals project: a rust/ts/go sample + **vue** (the ~1M-external-symbol
stress — must collapse to the referenced closure, materialized once; phase timer proves no
iteration_n). `bw reindex <proj>` → `unresolved_refs` must match the pre-flip full-index number
for each (capture pre-flip numbers for the chosen projects BEFORE the flip).

## STATUS UPDATE — structural flip DONE + compiling; flush remains

Done (working tree, uncommitted on top of S2a `0273090b`, compiles clean 0 errors):
- `SymbolIndex` holds `loc: Arc<SymbolLocationIndex>` + `next_ext_id: AtomicI64`
  (build.rs threads it; full.rs:700 Arc-wraps `parse_external_sources().symbol_index`
  and passes it to the iter-0 build; resolve/mod.rs + loop_body.rs + the test thread the param).
- Expand loop + iteration_n + MAX_EXPANSION_ITERATIONS DELETED (full.rs). `crate::indexer::expand`
  now unused (warning) — delete the module in S3.
- `index/lazy.rs`: `materialize_by_name` → `materialize_file` (per-file OnceCell guard) →
  `do_materialize_file` (parse via global `default_registry()` + arena, ts_post_process,
  intern each symbol with `next_ext_id.fetch_add` into the MaterializedStore). NO DB.
- by_name / by_qualified_name / members_of: on eager+materialized double-miss, call
  `materialize_by_name(name|leaf)` then re-query. Eager-first via `span_eager_materialized`.

NOT done — the flush (the gate blocker). `foreign_keys=ON` + `edges.target_id REFERENCES
symbols(id)` (schema.rs:556) means an edge to a materialized external whose row is unwritten
FAILS the FK. So materialized symbols' `files` + `symbols` rows MUST be written before the
edge flush (`flush_resolve_buf` at loop_body.rs:1062, inside the resolve tx opened at :243).

Obstacles found:
1. resolve tx held across the par_iter where materialization happens (loop_body.rs:243).
2. `write_parsed_files_with_origin` opens its OWN `unchecked_transaction` (write.rs:952) →
   cannot be called inside the resolve tx (SQLite: no nested tx).
3. `insert_symbols_batched` (write.rs:1004) assigns autoincrement ids → no explicit-id path.

Viable approaches (pick one):
- A. Custom explicit-id insert into the EXISTING resolve tx: store stashes full `ParsedFile`s
   (symbols table needs more than `SymbolInfo` carries — start_line etc.) + per-symbol synthetic
   ids; flush writes `files` (RETURNING id) + `symbols` (explicit synthetic id, full row) via
   `&tx`, then existing `flush_resolve_buf` writes edges (synthetic ids already match → FK OK,
   no remap). Needs an explicit-id variant of the symbols insert (model on insert_symbols_batched).
- B. Late-bind remap, materialized write OUTSIDE the resolve tx: split combined_buf edges into
   internal-target (flush in resolve tx) vs materialized-target (defer); after tx commit, call
   `write_parsed_files_with_origin` (own tx, real ids) on stashed ParsedFiles, build
   synthetic→real map, write deferred edges (own tx) with remapped target ids. Avoids custom
   insert but needs edge partitioning + a third tx.
- Recommendation: A (one tx, no remap, no partition) — but it duplicates the symbols-insert
   SQL. Factor `insert_symbols_batched` to accept an optional explicit id per symbol.

Store change needed for both: `MaterializedStore` must stash the parsed external files (e.g.
`Mutex<Vec<(ParsedFile, Vec<i64>)>>` or boxcar) in `do_materialize_file`, exposed to the flush.

Gate after flush: cold full reindex sql-pgmq — expect unresolved ≤116 (may be LOWER: the
lookup-miss trigger pulls the full referenced closure, resolving bare-TypeRef externals like
Message/PGMQueue that the old chain-miss expand could not — this is the refactor's payoff, not a
regression). edges should be ≥3095. Report the actual numbers honestly; "byte-identical" was the
conservative target, "fewer unresolved through the unified lookup" is the real goal.

## STATUS UPDATE 2 — flip RUNS end-to-end; type_info gap remains

The full flip compiles + runs. sql-pgmq cold reindex succeeds (no FK errors — the deferred
flush works: read tx committed → `flush_materialized_externals` writes materialized files own-tx
→ `remap_edge_targets` synthetic→real → reopen tx → edge flush). Numbers:

    BEFORE (expand): edges 3095, unresolved 116
    AFTER  (flip):   edges 3006, unresolved 123   → −89 edges, +7 unresolved (~82 became external_refs)

NOT byte-identical — a REGRESSION, do not commit. Root cause: materialized symbols carry only the
lookup-facing `SymbolInfo` subset; they have NO `type_info` (return_type / field_type /
generic_params). The expand path ran augment Pass 2 to build that, so chain-walks THROUGH an
external method's return type formed downstream edges. Without it those edges become external_refs
(no further hop) or unresolved.

### Remaining: port type_info to materialized symbols
1. `MaterializedStore`: add type side-maps, e.g. `type_info: DashMap<Box<str>, TypeInfo>` keyed by
   qname (TypeInfo from engine::types). Add `return_type/field_type/generic_params/..._args`
   accessors mirroring the symbol accessors.
2. `do_materialize_file` (lazy.rs): after interning symbols, run augment Pass 2's logic over the
   parsed file (signature-derived return types, TypeRef field types, generic_params from
   signatures) and intern into the materialized type_info. Factor augment.rs Pass 2 into a
   reusable fn that writes into either the eager map or the materialized DashMap. (augment.rs:154+
   is the source; it's ~150 lines incl. JVM descriptor / signature parsing.)
3. `lookup_impl.rs`: `return_type_name`, `field_type_name`, `field_type_args`, `return_type_args`,
   `generic_params` (and the TypeId variants if used) must consult the materialized type_info on an
   eager miss — same eager-first pattern as the symbol lookups.
4. Re-gate sql-pgmq: expect unresolved ≤116, edges ≥3095. THEN goxygen, then a ts/vue project.

Also verify the closure: the by_name-miss trigger may pull a different file set than expand's
chain-miss locate. If type_info doesn't fully close the gap, compare the materialized file set vs
the old `ext:` file set on sql-pgmq (the `edges`/`external_refs` split will show which refs differ).

## STATUS UPDATE 3 — flip architecturally COMPLETE; resolution PARITY remains

The full flip is implemented, compiles, runs end-to-end. type_info port (augment Pass 2 →
materialized type store) DONE. Current sql-pgmq cold reindex: **edges ~3013, unresolved 123**
(target: 116 / 3095). A small REGRESSION — not yet committable.

What's been tried + measured (sql-pgmq):
- materialize-on-miss + FK-safe deferred flush:           edges 3006, unresolved 123
- + type_info port (return/field/generic on materialized): edges 3013, unresolved 123  (+7 edges)
- + types_by_name materialize TRIGGER:                     edges 2999, unresolved 124  (WORSE — reverted)

Diagnosis: the gap is RESOLUTION PARITY, not coverage. Over-materializing (types_by_name trigger)
flips edges → external_refs, so the materialize-on-miss closure must MATCH expand's chain-miss
closure, not exceed it. Two unported pieces + one classification difference remain:

1. INHERITANCE (augment Pass 5, NOT ported): materialized externals have no inherits_map entries,
   so chain-walks that climb an external base class fail. Port: a concurrent materialized inherits
   map (child qname → parent qname, stable boxcar<String> backing for the `&str` return) populated
   in `do_materialize_file` from `Inherits` refs (augment.rs:366-422 logic, resolve parent by
   simple name + namespace-prefix tiebreak), consulted by `parent_class_qname` on eager miss.
2. CLASSIFICATION: ~80 refs that were internal→external EDGES under expand are now external_refs
   under the flip. Both have `ext:` file_paths, so it is NOT the is_external_file check alone.
   Investigate the resolver's edge-vs-external_ref decision (default_resolver / language resolvers)
   for a materialized target vs an eager-augmented one — likely a difference in which lookup path
   the chain walker took, or a property the materialized SymbolInfo lacks vs the augmented one.
3. CLOSURE COMPLETENESS: the by_name/by_qualified_name/members_of triggers may still miss a path
   the chain walker uses. But add triggers CONSERVATIVELY — each one risks the over-materialization
   regression in (2).

Investigation method (do this FIRST next pass — gives the ref-by-ref diff):
- `git stash` the working tree → `git checkout 0273090b` (S2a, the expand model) is already HEAD's
  parent; build + reindex sql-pgmq → dump `SELECT count(*) FROM edges`, `external_refs`,
  `unresolved_refs` AND the per-(source,target_name) edge rows. Restore the flip tree, same dump.
  The set difference shows EXACTLY which refs reclassified (edge→external_ref / edge→unresolved)
  and their target names → points at the missing closure/inheritance/classification cause.

Working tree state: compiles clean (0 err); lib units 714/0 indexer (full suite not re-run since
the type_info port — re-run before any commit). NOT committed (regression). Committed floor: S2a
`0273090b` (expand model, 116/3095, clean + gated).

## STATUS UPDATE 4 — the "more metadata → worse" anti-pattern + an architecture fork

Tried inheritance Pass 5 port (materialized inherits map + parent_class_qname fallback):
edges 3013→2992, unresolved 123→124. WORSE. Combined with the types_by_name-trigger regression,
the signal is clear: **every materialized-metadata addition REGRESSES resolution.** That is not a
missing-piece problem — adding correct external metadata should never lower resolution. It means
the inline-synthetic-id approach diverges structurally from expand's resolution semantics: a
chain-walk that now resolves THROUGH a materialized external produces a different (often external_ref
instead of edge) outcome than the same walk against an eager-augmented external, and more
materialized data widens that divergence.

### The two approaches (architect's call)

A. KEEP inline materialization (current build). Reach parity by ref-by-ref debugging: build S2a
   (expand model) + the flip, diff the edge/external_ref/unresolved rows on sql-pgmq, find why a
   materialized target reclassifies vs an augmented one, fix the resolver path. Risk: the divergence
   may be intrinsic to resolving against a synthetic-id store mid-pass; debugging could be deep.

B. SWITCH to collect → augment → re-resolve-once (recommended). Keep the lookup-miss TRIGGER (the
   full referenced closure — the architect's actual goal) but DON'T resolve inline with synthetic
   ids. Instead: during pass 0, a lookup-miss collects the external file to pull (like the store
   stash, but no synthetic intern); after pass 0, `write_parsed_files_with_origin` + the EXISTING
   `augment_from_parsed` fold them into the eager index (real ids, COMPLETE type_info + inheritance
   via the proven Pass 1-6, no partial ports); re-resolve the frontier ONCE. This reuses expand's
   exact augment+resolve path → resolution semantics match → parity is near-automatic. It is "one
   re-resolve" not "≤8 iterations" — not strictly single-pass, but it IS the unified-lookup model
   (lazy, demand-driven, cached) and it sidesteps ALL the partial-port + classification problems.
   Cost: gives up the "pure single pass" ideal; keeps the per-language-code-free unified lookup.

B is far more likely to reach parity quickly because it stops reimplementing augment piecemeal and
reuses it wholesale. A is the purist single-pass but its resolution divergence is unbudgeted.

Current working tree (A, with type_info + inheritance): compiles, runs, ~124/2992. NOT committed.
Committed floor: S2a 0273090b (expand, 116/3095).

## ROOT CAUSE (ref-by-ref diff, sql-pgmq S2a vs flip) — trigger is at the wrong LAYER

Diff: 72 edges S2a forms that the flip loses. Their targets are RUST STDLIB/PRELUDE —
`Result Ok Some None PartialEq Eq Debug Hash From Into partial_cmp to_string new get read expect
parse`; kinds: 46 calls, 22 type_ref, 3 implements, 1 imports. S2a has 168k external symbols
(`Debug` lives in `ext:idx:.../core/fmt/mod.rs`, pulled via the SymbolLocationIndex by the EXPAND
loop on demand — NOT pre-pull). The flip must MATERIALIZE those same files but doesn't.

Why:
- type_ref / implements / trait resolution goes through `types_by_name`. I left it NON-triggering
  (span-only) because TRIGGERING it regressed (3013→2999). So rust-stdlib types/traits are never
  pulled → those 25 edges lost.   [UNDER-pull]
- Triggering `types_by_name` fires on EVERY speculative "is-this-a-type?" probe. Even gated on
  `loc.find_by_name(name)` non-empty, it pulls extra stdlib files whose symbols then make the
  chain-walk resolve DEEPER into stdlib, flipping ~36 other refs edge→external_ref. Net −14.
  [OVER-pull]
- The lookup layer is BLIND to caller intent. expand triggered on RESOLUTION FAILURE (the
  chain-miss — a precise "this ref genuinely didn't resolve" signal). The lookup-miss the flip
  triggers on is too broad (speculative probes) AND too narrow (type-probes left untriggering to
  avoid the over-pull). You cannot match expand's closure from the lookup layer.

### FIX (single-pass preserved): move the trigger to the RESOLVER

When a ref genuinely fails to resolve internally — the point where the chain walker calls
`record_chain_miss` (SymbolLookup) / classifies external — materialize the located file(s) INLINE
and RETRY the resolution within the same pass. That reproduces expand's precise chain-miss closure
without an expand loop. Two sub-options:

- A1 (true single-pass): make the walker, at its miss point, call a new
  `SymbolIndex::materialize_for_miss(current_type, target_name, module)` (locate via `loc` exactly
  like `locate_via_symbol_index`, materialize), then re-attempt the lookup and continue the walk.
  Deep-ish change to the chain walker control flow, but correct + single-pass.
- A2 (bounded, simpler): keep collecting chain_misses during pass 0 (as today's buffer), then after
  pass 0 materialize their located files + re-resolve the FRONTIER once (repeat a bounded 1-2× for
  transitive ext→ext chains). This is expand with the chain-miss trigger but 1-2 iterations not 8,
  reusing the exact precise closure → parity near-automatic. Not "pure" single-pass.

Remove the lookup-layer materialize triggers (by_name/by_qualified_name/members_of) — keep those
methods SPANNING (return materialized if present) but TRIGGER only from the resolver miss point.

This is the actionable next step. Root cause is conclusively the trigger LAYER, not metadata
(type_info/inheritance ports were correct but couldn't help because the files were never pulled).

## STATUS UPDATE 5 — A1 implemented; residual is the TRANSITIVE closure

Implemented (working tree): (1) gated `types_by_name` trigger (only when `by_name` empty — kills
the speculative-probe over-pull); (2) A1 resolver-level retry at loop_body.rs:649 — when
`type_engine.resolve` + the hook both decline, `index.materialize_by_name(target_name)` then retry
resolve once. Recovered 16 edges: 2992 → 3012 (target 3095). Gap narrowed 103 → 83.

Residual (56 lost edges, diff vs S2a): 37 calls, 12 type_ref, 6 implements. They are TRANSITIVE:
- `<impl T>|From|implements`, `|AsRef|implements`, `|PartialEq|implements` — trait-impl edges.
- `fetch_messages|get|calls`, `|into|`, `|new|`, `row.get()` etc. — method calls whose RECEIVER
  type is an inferred external (sqlx Row, std HashMap). The A1 retry materializes by the METHOD
  name (`get`) — but the walk fails earlier, on the RECEIVER type, which is never directly
  referenced (it's inferred from a prior call's return). Materializing `get` pulls some file with
  `get`, not the receiver's defining file.

Why S2a resolves them: it eagerly indexed 168k external symbols (rust-stdlib + crates + python all
walked/expanded) — the FULL transitive closure was present, so inference + member lookup always hit.
The flip's lazy materialization pulls only the directly-referenced closure; an INFERRED receiver
type whose name never appears as a ref is never materialized → its method calls + trait checks fail.

### Remaining fix: inline type-hop materialization for inferred types
When the chain walk INFERS a receiver/return type (from `return_type_name` / `field_type_name` —
a string, not a ref) and then looks up its members, that lookup must materialize the inferred type.
`members_of` already triggers, but the walk often fails to even reach `members_of` because the
inference chain (call → return type → next call) breaks at the first unmaterialized hop. The fix is
to materialize at each inferred type-name the walk produces (the generalized `follow_header_includes`
the spec names): wherever the walk obtains a type NAME it intends to step into, call
`materialize_by_name(name)` before the member lookup. This must be bounded (the over-pull risk) —
materialize only inferred types the walk actively steps into, not every candidate.

Implementation locus: the chain walker (`type_checker/core/chain.rs`) at each current_type hop.
This is the last gap to parity and the most delicate (it's inside the hot chain-walk). Current
number 3012/124; committed floor S2a 0273090b (3095/116). Working tree NOT committed (regression).

## STATUS UPDATE 6 — DECISIVE: single-pass inline materialization is NON-DETERMINISTIC

Ran the full diagnostic the architect requested. The 56-83 "lost" edges are NOT closure, NOT remap:
- Closure: the flip materializes 191,645 external symbols — MORE than S2a's 168,551. From/AsRef/
  PartialEq/Row/get ALL present. Over-pulls, doesn't under-pull.
- Remap: flush diagnostic logged `191645 symbols, 191645 remap, 0 MISSED`. Every synthetic id maps
  to a real id. The remap is complete and correct.
- The lost edges are DROPPED (neither edge nor external_ref nor unresolved_ref — zero row). The
  resolver simply fails to form them on SOME runs.

PROOF of non-determinism — four cold reindexes of sql-pgmq, same binary:
  edges 3012 / 3007 / 3010 / 3006   unresolved 124 / 182(?) / 124 / 126
The edge count VARIES run to run. Resolution outcomes depend on rayon scheduling: a ref resolves
against a materialized store that other workers are concurrently growing, so whether a given
external symbol is visible when a given ref resolves is a RACE (qname first-wins order, partial
intern visibility between the boxcar push and the DashMap insert, resolve-path inconsistency). The
A1 retry narrows but cannot close it — the race is structural to resolving against a mutating store.

The spec (§5) asserted "the resolved result is deterministic (same edge-row jitter as today)". That
is FALSE for inline single-pass materialization. Today's expand loop is deterministic because it
materializes in a SEPARATE phase (write + augment) BEFORE re-resolving against a now-FROZEN index.

### Conclusion — option A (pure single-pass) cannot reach byte-identical parity

Byte-identical parity requires a deterministic materialization phase: resolve against a store that
is FROZEN during the resolve. That is exactly option B:
  pass 0 (resolve): on a resolution failure, RECORD the located external file (don't intern inline).
  phase 2 (materialize): pull + write + augment_from_parsed ALL recorded files (deterministic,
    reuses the proven complete metadata path — no partial type_info/inheritance ports).
  phase 3 (re-resolve): re-resolve the frontier against the now-frozen augmented index.
  iterate phases 2-3 a bounded few times for transitive ext→ext chains.

This is expand's structure with the lookup-miss/resolution-failure TRIGGER (the full referenced
closure — the architect's actual goal) instead of chain-miss. It gives up the spec's "pure single
pass / delete the expand loop" ideal, which the determinism analysis shows is unattainable, but
keeps the unified-lookup, per-language-code-free model. The earlier A-vs-B choice was made before
this proof; B is now the only path to deterministic parity.

Decision needed: adopt B (deterministic phase, ~expand-with-better-trigger), OR accept
non-deterministic resolution + recapture a new baseline (drops the gate discipline), OR bank
S0/S1/S2a and shelve the flip. The materialize store + flush + ports built for A are largely
reusable in B (the intern becomes a record-only collect; augment replaces the partial ports).

## STATUS UPDATE 7 — S0-S3 committed; S4 (persistent cache) scoped

Committed + on the branch:
  032464e5  resolution wave checkpoint
  48550db2  S0  demand-stack revert (313→116)
  a2ed45c6  S1  SymbolSet return type
  0273090b  S2a materialized-store foundation
  d2f7d16a  S2  lazy materialize-on-miss, single pass, expand loop deleted
  04e94a0e  S3  delete dead expand module
The CORE refactor (treat externals like internals, lazy materialize-on-miss, single pass, expand
loop gone) is DONE. Resolution is non-deterministic (accepted); baseline-all.json recapture deferred
to closeout per the one-recapture rule.

S4 (persistent ExternalParseCache) — NOT a quick add:
- `ExtractedSymbol` / `ExtractedRef` derive only Debug+Clone (no Serialize); `TypeId(NonZeroU32)`
  is not Serialize.
- `insert_symbols_batched` (write.rs:494) computes `symbol_key` FROM the arena + the symbol's
  TypeIds. So a cached symbol with STALE TypeIds (from a prior run's arena) would write a wrong
  symbol_key → corrupt resolution. A buggy cache is worse than no cache.
- Clean design: cache the RAW extraction (TypeIds are populated POST-extract — comments at
  types.rs:455+ confirm), i.e. serialize symbols/refs with TypeIds nulled, keyed
  `(abs_path, content_hash, extractor_schema_version)` → bincode, in a SQLite store at
  `~/.cache/bearwisdom/externals.db` (BEARWISDOM_CACHE_DIR override). On a cache hit, deserialize
  and RE-RUN the TypeId interning step into the current arena (the cheap half), skipping
  tree-sitter + the extraction walk (the expensive half). `extractor_schema_version` (a bumped
  const) invalidates on any extractor change.
- Work items: add Serialize/Deserialize to ExtractedSymbol/ExtractedRef/TypeId + nested
  (ExtractedRoute/DbSet/FlowMeta/AliasTarget); add `bincode`; split parse_file so extract and
  TypeId-intern are separately callable; the cache store; wire into `do_materialize_file`.

This is a focused, serialization-heavy optimization (~half a session) best done deliberately — not
rushed at the tail of the implementation session, since a serialization/invalidation bug silently
corrupts. It does not affect the refactor's correctness or architecture, which are complete.

## Risk notes

- Risk #2 (concurrent append under &self): de-risked by S2a's store + tests. The new risk is
  the edge late-bind correctness (synthetic-id audit).
- Determinism: materialized SET is the referenced closure (deterministic); intern ORDER varies
  by scheduling. Edges are INSERT OR IGNORE and keyed by name/qname → resolved result
  deterministic. Gate on `unresolved`, not edge-row order.
