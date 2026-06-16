# Refactor — unified, lazy, cached external resolution (Roslyn model, single pass)

Goal (architect): externals are not special. A reference in the file being resolved binds the same way whether its target is internal or external. External symbols are materialized **lazily, on first reference, symbol-granular**, and **cached** so any later reference — another file, another worker, another reindex — reuses the already-resolved symbol. **One resolve pass.** No `expand` fixpoint, no second pass, no per-language external code.

This is Roslyn's metadata model. The reason it never re-resolves and never has a separate external phase is that materialization is lazy + cached behind a unified symbol space. The borrow concern from the earlier draft is dissolved by the cache: a materialized symbol has a stable address and is reused, not re-derived.

---

## 1. Roslyn → BearWisdom (the exact correspondence)

| Roslyn | BearWisdom |
|---|---|
| `Compilation` = source + `MetadataReference`s | one `SymbolIndex` = parsed internal + an external **location index** |
| `MergedNamespaceSymbol` — one lookup over source+metadata | one `SymbolLookup`, **no origin branch** |
| `MetadataReader` — name→TypeDef, O(1) random access | `ExternalLocationIndex` — `(module,name)→file(+span)`, cheap header scan |
| `PENamedTypeSymbol` materialized lazily; `GetMembers()` decodes on first touch, **cached** | external symbol materialized on first reference (parse-once + symbol-granular extract), **cached in a stable store** |
| `MetadataDecoder` resolves a TypeRef token mid-bind | external type-hop materialized while walking the chain |
| `AssemblyMetadata` shared across compilations (`FileKey`) | persistent parsed-external cache shared across reindexes/projects |
| bind each node once | resolve each ref once; materialize each external symbol once |

## 2. Architecture — one pass, lazy + cached

```
build internal SymbolIndex (eager)               internal symbols, contiguous, as today
build ExternalLocationIndex (cheap header scan)  name/qname → file(+span), ALL ecosystems   [= MetadataReader]
attach the materialization cache to the index    append-only stable store + concurrent name/qname/member maps

ONE resolve pass  (rayon par_iter over internal files — the existing loop, unchanged in shape)
   chain walker / resolver calls lookup.by_name / members_of / return_type_name / … with NO origin knowledge:
      lookup(name):
         hit in eager-internal or materialization-cache  → return it
         miss, but ExternalLocationIndex has it          → MATERIALIZE(file): parse-once (cache) →
                                                            symbol-granular extract → intern into the
                                                            stable store + update the concurrent indices →
                                                            return it          (cached: next reference, any file/worker, is a hit)
   a materialized symbol's members come from the same extract; a cross-file type-hop (its return/field type
   in another file) materializes the SAME way, recursively, inline — the closure unfolds during the walk,
   never as a pass.

flush  materialized external symbols + the edges that bound to them → DB   (once, after the pass)
```

External symbols are resolved **once** (first reference) and reused forever after (cache). There is **one** project pass. No `expand`, no iteration_n, no second bind pass.

## 3. Storage — what makes lazy materialization safe under `&self` (the crux)

`SymbolLookup` returns borrows. To materialize a symbol under `&self` (shared across rayon workers) and hand back a reference that stays valid, the store must be **append-only with stable addresses and thread-safe push**. Then "cache the result" is literally the store: push once, borrow forever.

```
SymbolIndex {
   // internal: eager, contiguous (today's Vec<SymbolInfo>) — unchanged
   internal: Vec<SymbolInfo>,

   // external: lazily appended, stable address, &self push, thread-safe
   materialized: elsa::sync::FrozenVec<Box<SymbolInfo>>,        // or boxcar::Vec / hand-rolled append-only
   mat_by_name:   DashMap<Box<str>, SmallVec<[SymbolRef; 2]>>,  // name  → refs (internal idx | materialized idx)
   mat_by_qname:  DashMap<Box<str>, SymbolRef>,
   mat_members:   DashMap<Box<str>, SmallVec<[SymbolRef; 4]>>,  // parent qname → members
   mat_types:     side-maps for field_type/return_type/generic_params of materialized symbols

   loc: ExternalLocationIndex,                                  // name/qname → file(+span)  [the MetadataReader]
   mat_files: DashMap<PathBuf, Arc<OnceCell<()>>>,              // per-file single-materialization guard
   cache: Arc<ExternalParseCache>,                              // persistent, content-keyed (§6)
}
```
`SymbolRef = enum { Internal(u32), Materialized(u32) }` — a stable handle into either store. `elsa::sync::FrozenVec<Box<_>>` is the canonical Rust "append under `&self`, get back stable `&T`" type; `boxcar::Vec` or a `Mutex<Vec<Box<_>>>` are alternatives. (`elsa` is ~one small dep; evaluate vs hand-roll in S1.)

**The borrow problem is gone**: `by_name` returns refs that point into `internal` (stable, built once) or `materialized` (boxed, stable forever after push). Nothing moves; a reference handed to one worker stays valid while another worker appends.

### Return-type adaptation (the main mechanical cost — be honest about it)
Today `fn by_name(&self) -> &[SymbolInfo]` assumes one contiguous store. Spanning eager+lazy means the lookup methods return **refs that span both stores**:
```
fn by_name(&self, name: &str) -> SymbolSet<'_>     // SymbolSet = smallvec of &SymbolInfo, or an iterator
```
`SymbolSet` borrows the eager slice when there are no materialized hits (zero-cost, the common internal case) and collects `&SymbolInfo` across stores otherwise. This touches every chain-walker call site (`type_checker/core/chain.rs`, `default_resolver.rs`) — mechanical, but broad. It is the price of the unified lookup and it is unavoidable in Rust for A. (B avoided it by never materializing mid-walk; you've correctly rejected B.)

## 4. Unified lookup — no origin branch `[generic]`

The resolver/chain-walker never asks "is this `ext:`?". `lookup.by_name(x)` returns whatever exists — internal eagerly, external by lazy materialization. The only `ext:` checks that remain are: (a) external files are never resolution *sources* (`loop_body.rs:368` filter — kept), and (b) `SymbolInfo.origin` for edge-writing/metrics. Resolution logic is origin-blind — your Roslyn point, enforced structurally.

## 5. Concurrent materialization during the par_iter

- Worker hits external ref → `mat_files.entry(file).or_default()` → `OnceCell::get_or_init(|| parse_once(file) → extract → intern)`. One worker parses each file; concurrent hitters block briefly then reuse. Parse fan-out is naturally spread across workers (each materializes what it touches).
- Intern = push to `materialized` (FrozenVec, `&self`) + insert into `mat_by_name`/`mat_by_qname`/`mat_members` (DashMap, concurrent). All append-only — no worker invalidates another's borrow.
- **DB write deferred** (today's pattern): materialized externals carry no DB id during the pass; after the pass, assign ids + write symbol rows; edges that bound to a `SymbolRef::Materialized` resolve handle→id at flush. The edge buffer already defers — extend it to late-bind materialized-external targets. **No SQLite writes from inside rayon.**
- Determinism: the materialized *set* is the referenced closure (deterministic); intern *order* varies by scheduling, but lookups are by name/qname and edges are `INSERT OR IGNORE`, so the resolved result is deterministic (same edge-row jitter as today; gate on `unresolved`).

## 6. The cache — resolve once, reuse everywhere AND across runs

Two levels, both are "you already have it resolved":
- **In-run** (the `materialized` store + maps): the moment file A references `windows::HANDLE`, HANDLE is materialized and interned; file B's reference, on any worker, is a map hit. This is the architect's point exactly.
- **Cross-run / cross-project** (`ExternalParseCache`, the `AssemblyMetadata` analog): `parse_once` consults a content-addressed store keyed `(eco, module, version, rel_path, content_hash, extractor_schema_version)` → `bincode(ExtractionResult)` before tree-sitter. So even a fresh reindex (or a different project sharing the dep) skips the parse and re-interns from cache. Persistent store at `~/.cache/bearwisdom/externals.db` (`BEARWISDOM_CACHE_DIR`-overridable); `extractor_schema_version` invalidates on any extractor change (mandatory).

## 7. What's deleted

```
DELETE  the ≤8-iteration expand loop (full.rs 963–1028)            → lazy materialization, inline
DELETE  iteration_n + the external part of re-resolution           → one pass
DELETE  B's idea of a second bind pass                             → one pass (your objection)
DELETE  per_file_demand-from-chain_misses, the demand-filter-as-a-separate-external-pass (the regression source)
DELETE  typescript::extract_with_demand override                   → generic materialize
DELETE  the ext:-vs-internal branch on the resolve path            → unified lookup
GENERALIZE follow_header_includes (C-only) → inline type-hop materialization for all languages
KEEP    ExternalLocationIndex, filter_extraction_to_demand (now per-symbol at materialize time),
        phase timers (#1).  delta-resolve (#2) reduces to the internal return-inference fixpoint only.
```

## 8. Why the regression cannot recur

There is no external phase. Internal `Message` binds during the single pass against the eager-internal store via the same lookup; an external materialization appends to a different store and cannot remove or shadow an internal symbol (lookup checks eager-internal first; origin-blind but internal-first on ties, preserving today's "local beats imported" precedence). The gate (`unresolved` byte-identical) is enforced per step.

## 9. Sequencing — incremental, each gated on `unresolved` byte-identical

```
S0  revert the externals-incremental stack to GREEN (171 baseline; whole-file pull, correct-but-slow).
      keep: #1 instrumentation, #2 delta-resolve, filter_extraction_to_demand as a lib fn.
S1  storage: introduce the append-only materialized store + SymbolRef + the SymbolSet return type;
      port the chain-walker call sites. NO lazy behavior yet — externals still pulled the old way,
      just stored through the new types. ── gate: unresolved byte-identical (pure refactor)
S2  ExternalLocationIndex on the index + lazy MATERIALIZE on lookup-miss, running DURING the single
      resolve pass; delete the expand loop. ── gate: unresolved byte-identical on rust/ts/go sample
      + vue; ext symbols = referenced closure; external bind happens once (phase timer proves no iteration_n)
S3  delete TS override + the ext:-branch + dead expand/demand machinery. ── gate: same
S4  ExternalParseCache (persistent). ── gate: same numbers; --force/repeat near-free on externals
```
S1 is a pure type refactor (riskiest for breadth, zero behavior change — easy to gate). S2 flips to lazy single-pass (the heart). S3 removes the corpse. S4 is the persistent cache.

## 10. Risks / hard parts (ranked)

1. **`&[SymbolInfo]` → `SymbolSet` across all call sites** (S1). Broad but mechanical; gate is a pure refactor (numbers must not move). Biggest *surface*.
2. **Concurrent append-only index during par_iter** (S2). `elsa::sync`/`DashMap` semantics, per-file `OnceCell`, no borrow invalidation. Biggest *subtlety*. Mitigation: materialization is append-only (never mutate/remove), so no reader is invalidated.
3. **Deferred DB ids for materialized externals** (S2). Edges late-bind handle→id at flush. The buffer already defers; extend it.
4. **Closure correctness** = the gate. A reached type-hop must materialize; a bare type_ref to an external type must trigger materialization (the lookup-miss path covers it — that was the sql-pgmq gap, now structural). `unresolved` byte-identical is the guardrail.
5. **Whole-file parse on first touch** (not Roslyn-O(1)). The in-run + persistent cache amortize it (parse each external file at most once, ever). Byte-span extraction below the file is a later optimization only if one mega-file's parse dominates — measure first.

## 11. Outcome

One origin-blind resolve pass; each external symbol parsed+materialized at most once per machine (persistent cache), reused across files, workers, projects, and reindexes. The `expand` fixpoint and every per-language external path are gone. vue's ~1M external symbols become the referenced closure, materialized once; resolve passes run against a small index; the corpus tail collapses proportionally — and a warm cache makes repeat indexing near-free on externals.
