# Resolution engine — string-elimination plan

Goal: type identity is a `SymbolId` end-to-end (allocate → materialize → walk → persist),
deterministic, with no qname string carried as *stored identity*. A string used as an
arena interning key or rendered for display is fine; a string re-searched at use time is not.

## Keystone correction

The keystone is **NOT** `Type::Class(String) → Type::Class(SymbolId)`. That cut is wrong:
`TypeArena.class(qname)` dedups one qname → one TypeId, but `qname → SymbolId` is
many-to-one (overloads, declaration-merging, same name across monorepo packages), and the
correct id is **use-site dependent**. A single id in the interned leaf would collapse
`List<pkgA>` and `List<pkgB>`. The id already lives where it belongs — on the per-use-site
`Receiver { ty: TypeId, id: Option<i64> }`.

The walk is **already id-based** (`members_by_id`, `inherits_by_id`, id-keyed supertype BFS).
The only gap: `expand_receiver` **re-derives** the id every hop via
`head_qname → by_qualified_name(qname)` — a string re-search that (a) is the per-hop string
dependency and (b) loses import scope, which is the determinism leak.

**Keystone = kill the per-hop re-search by threading/caching the use-site id through the walk.**
`Class(String)` stays the interning key. ~3 call sites, not a leaf migration.

## Decisions (defaults taken; override any)

- **A. Identity** — numeric `i64` surrogate, assigned deterministically from a sorted
  `symbol_key` pass (not insert order). `symbol_key` (`qname#kind#arity[#params]`, already
  built + persisted) is the durable serialization key. **Add `AUTOINCREMENT`** to `symbols.id`
  so a deleted-then-reused rowid can never *alias* a persisted id (silent corruption); a stale
  id must dangle, not alias.
- **B. Keystone** — thread/cache the use-site id (above). Keep a string-bearing leaf arm for
  external / ambient / primitive / unresolved heads.
- **C. Persistence** — store the structured type graph durably (normalized type table or
  id-resolvable wire form); migration = bump `user_version` → force reindex (pointers can only
  come from re-materialize).
- **D. Phase-2 evaluators** — keep Union/Intersection/Keyof capture-only; revive only Typeof,
  literal IndexedAccess, transparent Conditional/Mapped. Cycle-safety stays bounded-local
  recursion + self-ref short-circuit (no global fixpoint). Generic params keep name keying
  (scoped, never persisted → not "storage").
- **E. Externals** — bounded Phase-2 sub-fixpoint (status quo) + seed the closure from
  materialized externals' return-type heads so chains walk one hop deeper into library APIs.

## Sequence (each step independently verifiable; dependency-ordered)

**Step 0 — Determinism gate (RED).**
Resolve-twice-identical test: index the engine twice into in-memory DBs, diff qname-keyed
`EdgeKey` sets, assert empty symmetric difference. RED today (≈37 refs flip). Reuses
`indexer/resolve_diff.rs` (currently legacy-vs-engine; legacy is gone → make it engine-vs-engine).

**Step 1 — Phase-1 foundation: deterministic + complete identity on the FULL path.**
The full path today assigns rowids in parallel-completion order, never collapses mergeable
symbols, and never resolves cross-file containment — so ids churn AND `members_by_id` misses
cross-file internal members (Rust `impl`, C# partial). Run the global `symbol_key → id`
pass (today incremental-only) on the full path too: deterministic stable ids + mergeable
collapse + cross-file `containing_id`. Plus decision A's `AUTOINCREMENT`.
*Gate: full reindex twice → identical id per `symbol_key`; cross-file members present in `members_by_id`.*

**Step 2 — Keystone: kill the per-hop re-search.**
`resolve_root` yields `(TypeId, SymbolId)` at every root; `expand_receiver` prefers the carried
id; member hops walk by id. The qname re-search runs only for genuinely-unresolved heads —
and Step 1 made internal `members_by_id` complete, so retiring the internal fallback no longer
regresses cross-file members.
*Gate: Step-0 test GREEN; new test "carried id is correct when `by_qname` first-winner is wrong" GREEN.*

**Step 3 — TypeId flow (remove string round-trips + corruption).**
`local_type` flow cache → `TypeId` (kills the `format_type → String → intern_type_str` round-trip),
which also fixes the silent corruption where `Primitive(Int)`/`Optional`/`Generic`/`Literal` get
nominalized to `Class("Int")` etc.
*Gate: those yields stop nominalizing; determinism still green; edge-count delta = GAINS only.*

**Step 4 — Persistence: store ids, drop the string twins.**
Structured durable type encoding (decision C); incremental type-info persist + **type-surface
invalidation** (today blast-radius is `symbol_key`-keyed = name surface only, so a stable-key
changed-return-type leaves dependents stale); external-inheritance durability (latent gap:
external inherits never become edge rows, may already break after first incremental save).
*Gate: persist→reload reproduces the full edge set; type-only incremental change invalidates dependents.*

**Step 5 — Phase-2 depth (quality).**
Revive the targeted alias evaluators (D); formalize the cycle/ordering contract; external
return-type-head closure seeding (E).
*Gate: targeted smoke-corpus gains; single full `baseline-all.json` recapture at closeout only.*

## Standing constraints

- Schema changes **additive only** — never rename/drop `symbols`/`edges` columns (AlphaT/Lynx
  use raw string-literal SQL; no external reader touches the type-string surface, so this is an
  internal refactor otherwise).
- One `cargo` invocation at a time. Targeted `-p bearwisdom --lib <module>` per step.
- Resolve-twice-identical (Step 0) is the **standing gate** for every step after.
- Perf/memory budget gate on the largest smoke project (ts-nextjs): the deterministic-id
  route must not reintroduce the peak-RAM pressure the streaming write was built to avoid.
