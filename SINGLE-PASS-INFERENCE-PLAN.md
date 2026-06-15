# Single-pass resolution via demand-driven recursive inference

## Goal

Collapse `iteration_0 + return_infer` (whole-file fixpoint, ~52% of index time on
TS, ±15% non-determinism, `MAX=3` factory-depth cap) into **one** dependency-ordered
resolution pass. The dependency order is not computed up front — it **emerges** from
demand-driven, memoized recursion with cycle detection, exactly how Roslyn binds
`var x = Foo()` by recursing into `Foo` on demand and memoizing.

There is no "internal vs external" axis in the target design. There is one rule:

```
return_type_of(f):
    ensure_loaded(f)              # external → materialize file on demand; internal → no-op
    if f has a declared return → read it          # annotation, .d.ts/DLL/jar signature
    else                       → recurse into f's body, infer from its `return <expr>` sites
    memoize(f → result); on-stack(f) → Unknown    # cycle = genuinely uninferable
```

## Why it's a rearchitecture, not a patch

Computing `f`'s return needs `f`'s **flow context** (`return r` where `r` is a local
needs `r`'s inferred type). That context — `LocalTypeCache` (forward name→type,
narrowings, discriminants, cfg, cursor) — is built incrementally as the resolver
walks `f`'s body and lives in a **thread-local installed per file and wiped after**
(`engine/index/mod.rs:LOCAL_TYPE_CACHE`, `install_local_cache`/`clear_local_cache`).

So you cannot recurse into `g`'s body while resolving `f` — `g`'s context would
clobber `f`'s. That single transient slot is what forces the current whole-file
re-resolve fixpoint.

## The enabler — make the flow context a stack

```
LOCAL_TYPE_CACHE: RefCell<LocalTypeCache>   →   RefCell<Vec<LocalTypeCache>>
  install_local_cache(...)  → PUSH a scope
  clear_local_cache()       → POP
  record_local_type/local_type/lookup_union/set_cursor → operate on TOP of stack
```

The main per-file loop already pairs `install` (loop_body:499) with `clear`
(loop_body:1057), so push/pop is balanced. Recursive inference pushes a fresh scope
for the callee and pops back — `f`'s context is preserved across the recursion.

Same treatment for the miss accumulator (`CURRENT_FILE_MISSES`): a nested inference
scope must not pollute the host file's frontier.

## Inference engine

```
struct ReturnInferrer {
    returns: Vec<FunctionReturns>,            // step 1 — DONE (return_inference.rs)
    memo:    Mutex<FxHashMap<String, Inferred>>,  // qname → Resolved(TypeId) | Unknown
    on_stack: thread_local set,               // cycle detection per worker
}
Inferred infer_return(qname):
    if let Some(m) = memo.get(qname) { return m }       // memoized
    if on_stack.contains(qname) { return Unknown }       // cycle (SCC) → uninferable
    on_stack.insert(qname); push flow scope
    resolve fn's body refs in source order (seeds locals), then its return refs
        via type_engine.resolve(...)  — SAME resolver, no reimplementation
    result = conflict-join(yield types)                  # disagreeing returns → Unknown
    pop flow scope; on_stack.remove(qname); memo.insert(qname, result)
```

The cycle set + memo table + recursion stack **are** the SCC handling — no explicit
call graph or topo-sort. A mutual-recursion cycle resolves to `Unknown` (correct: no
concrete return exists), arbitrary factory depth works (no `MAX=3`).

## Integration

- chain walker yield-unknown bail (`core/chain.rs:769`) → call `infer_return(fn_qname)`
  instead of recording a chain miss; on `Resolved(t)` continue the chain, on `Unknown`
  bail as today.
- delete the `return_infer` fixpoint loop (`full.rs:978`); `iteration_0` becomes the
  single pass.

## Migration steps (each independently compilable + validated)

```
1. return-refs index                         ✅ DONE (return_inference.rs, tested)
2. flow-context stack enabler                 ← NEXT. unit-test push/pop isolation.
3. ReturnInferrer + infer_return (memo+cycle), behind a flag, NOT yet wired
4. wire into yield-unknown bail; keep the batch loop as fallback; compare edge counts
5. delete the batch fixpoint; iteration_0 = single pass
6. validate: edges hold (~78k react-tanstack-query), return_infer phase gone,
   determinism (same edge count across runs)
```

## Validation gates

- **Correctness:** edge count ≥ current (~78k on react-tanstack-query), `unresolved`
  not inflated. Edge count is the deterministic signal; never accept a drop.
- **Perf:** `return_infer` phase eliminated; total index time down materially (the
  52% is the budget).
- **Determinism:** repeated reindex of unchanged source yields the same edge count
  (the current ±5% edge / ±15% time variance is a symptom of the fixpoint; it should
  vanish).

## Risk

The enabler (step 2) touches the hot resolve path's flow state. It is the
highest-risk step; it gets its own focused change + tests before anything is wired.

---

# FINDINGS — the prepass approach is REJECTED (2026-06-15)

Steps 1–2 are committed (`e0ad9cc2`) and sound. Steps 3–5 were implemented and
**measured to fail**, three times, on react-tanstack-query (baseline ~78k edges):

| variant | inferred | edges | total | verdict |
|---|---|---|---|---|
| unsound prepass (seeds locals) | 674/819 | 66,882 | 687s | edges −11k, slower |
| sound prepass (no seeding, context-independent only) | 750/819 | 67,459 | 703s | edges −11k, slower |

**Root cause — resolution is STATEFUL.** The index evolves during a pass
(externals materialize on-miss; inferred returns accumulate). A per-function
prepass runs at an *earlier* state than `iteration_0`, so it resolves return
expressions differently — and `set_inferred_return` is set-first-wins, so the
prepass's (different) returns lock in and *block* the correct ones, corrupting
resolution. No amount of "context-independence" fixes this: even
`return new X()` can resolve differently before vs after other files
materialize their externals. **You cannot pre-compute a stateful process.**

**The plan was also mis-scoped.** The batch `return_infer` pass logs
`"83,506 edges resolved, 33 inferred returns applied"` — its cost is the
**full frontier re-resolve**, not the 33 returns. That re-resolve does *double
duty*: (1) apply inferred returns, and (2) **externals-ordering catch-up** —
`iteration_0` resolves file F before file G materializes external X, and the
re-resolve is what closes those chains. Narrowing it (the reverted "Fix 1")
drops ~16k edges. So the 52% is dominated by **(2), externals catch-up — not
return inference.** This plan optimized the minor half.

## The actual single-pass requires TWO rearchitectures

```
A. EXTERNALS-MATERIALIZATION COMPLETENESS  (the dominant half)
   iteration_0 materializes externals on-miss, but inherits_map / supertype graph
   / members index were built BEFORE materialization, so a materialized external's
   relationships aren't wired in → chains through them miss until a re-resolve.
   Fix: wire a materialized external's inherits/supertypes/members into the live
   structures at materialize time → iteration_0 completes in one pass, order-
   independent → the externals-catch-up re-resolve disappears.

B. INLINE demand-driven return inference  (the minor half; the prepass's CORRECT form)
   NOT a pre-pass. During the full resolve, the chain walker's yield-unknown bail
   recurses into infer_return(fn) AT THE CURRENT STATE (correct context), memoized,
   cycle-detected. Obstacle: the walker↔engine↔inferrer circularity — the walker
   (inside engine.resolve) needs the engine+parsed to infer. Break it with a
   thread-local context (raw pointers set before the par_iter, since the borrowed
   data outlives it) read by the bail. Steps 1–2 (return-refs index + flow stack)
   are the foundation for this.
```

Both are major, multi-session, and **A is the prerequisite** (without it, removing
the batch loses the catch-up edges regardless of inference). The committed
foundation stands; the prepass (steps 3–5) is abandoned.

---

# CONCLUSION — per-function return inference is INFEASIBLE (2026-06-15)

After the prepass, the **inline** form of B was also implemented and measured:
a `InferReturn` trait threaded into the chain walker, so a yield-unknown bail
resolves the member's body **at the live state, recursively, memoized**, on the
rayon workers (big stacks — no overflow). It is the structurally-correct version
of the plan's intent. Result on react-tanstack-query (batch still present as a
safety net):

```
edges 73,211 (−5k)   total 980s (1.9× baseline)   return_infer STILL 2 full passes
iteration_0 304s     (inline body-resolutions nearly doubled it)
```

**Both independent implementations — prepass and inline — corrupt resolution.**
The cause is fundamental and shared: a function's return type is computed by
resolving its `return <expr>`, and that resolution needs the **complete file
context** — every local forward-inferred in source order, plus the evolving
materialized-externals / inferred-return state — which the whole-file pass builds
and which **cannot be reproduced by resolving a function's body in isolation**, at
any state, with or without recursion. Per-function inference is not a tuning
problem; it is the wrong unit of work.

## The only sound single-pass is dependency-ordered resolution

Return inference inherently needs two touches of a function — resolve to harvest
its return, then resolve its callers with that return known — UNLESS the whole
resolve pass is **ordered callee-before-caller** (topo-sort the call graph; iterate
within SCCs). Then every caller resolves after its callees and reads their returns
in one pass. That is not an inference change; it is a rearchitecture of the resolve
loop's *scheduling* — from the current unordered rayon `par_iter` to a
dependency-scheduled execution. Large, separate, and the genuine "true compiler
shape" applied to resolution as a whole.

## Net state

```
✅ kept (committed): ChainMiss refactor (+19% edges, neutral speed) + the
   foundation (flow-context stack, return-refs index). All sound.
❌ single-pass via return inference: PROVEN infeasible (prepass + inline both fail).
   The plan's premise — replace the whole-file return re-resolve with a per-function
   pass — does not hold.
→ open levers, both major: (A) externals-materialization completeness; or a
   dependency-ordered resolve schedule. Neither is a return-inference patch.
```
