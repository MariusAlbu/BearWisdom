# Indexing observability + instrumentation plan — 2026-06-13

Goal: make single-project `full_index` latency **measurable per phase** and **optimizable**, with the same instrumentation doubling as production observability for the long-running service binaries (MCP, Web). Scope discipline: the recapture loop is a test harness — we optimize the *single-project* index path users actually run; the only cross-unit parallelism in scope is **monorepo-internal** (across a project's own packages).

---

## 0. Current state (measured)

```
tracing                  facade, emitted throughout the core lib (info!, a few instrument sites)
tracing-subscriber fmt   plain stderr formatter, set up PER-BINARY — no shared init:
                           cli/main.rs:589   default "warn"   ← PHASE_TIMER hidden unless RUST_LOG=info
                           mcp/main.rs:58     default "info"
                           web/main.rs:41 · bench default "info"
phase_timer.rs           homegrown atomic cumulative accumulators; dump_and_reset() per index;
                           emits `PHASE_TIMER phase=… total=… calls=… per_call=…` (tracing::info!)
opentelemetry / OTLP     NONE
```

The core lib **only emits**; each binary **owns its subscriber**. That's the correct layering — instrumentation is provider-agnostic (`tracing`), the exporter is a binary-level concern. Adding OTel is a subscriber *layer*, not a rewrite.

**Coverage gap (the reason "each phase" isn't answerable today).** `phase_timer` scopes cover externals (7), resolve (3), engine (3), admit (2), vendored (1) — but NOT: file walk, parse (tree-sitter), per-language extraction, scope-tree build, DB writes, concept discovery, the connector pass, FTS/chunk build. Those are likely the bulk of `full_index` wall-clock and currently land in an unattributed "remainder" (= `index_duration_ms − Σ instrumented`). `perf-measure` is quantifying that remainder now.

---

## Part 1 — Pipeline instrumentation substrate (the shared foundation)

Two complementary mechanisms, by call frequency. **This is the load-bearing distinction.**

| Mechanism | Where | Why |
|---|---|---|
| `tracing` **span** (`#[instrument]` / `span!`) | COARSE stages — once or O(batches) per index | Nestable tree, OTel-exportable, flamegraph-able. Span create/enter cost is negligible at this granularity. |
| `phase_timer` (atomic accumulator) | HOT loops — per-file / per-ref / per-symbol, thousands of rayon calls | A span per call would add measurable overhead in the exact loops we're profiling, and `tracing` context does **not** propagate across rayon work-stealing without explicit capture. Keep the cheap atomic add. |

### 1a. Instrument the coarse pipeline as spans

Stage tree to add (in `indexer/full.rs` + the stage modules), one span each — these are the phases "logs on each phase" means:

```rust
#[tracing::instrument(skip_all, fields(project = %root.display()))]
pub fn full_index(...) {
    let _ = span_stage("walk");          // file discovery + language detect
    let _ = span_stage("parse");         // tree-sitter parse + scope_tree (the likely elephant)
    let _ = span_stage("extract");       // per-language extractors → symbols/refs
    let _ = span_stage("externals");     // ecosystem locate/parse/build_symbol_index (already phase_timer'd inside)
    let _ = span_stage("resolve");       // the iteration-to-convergence loop
    let _ = span_stage("concept");       // concept discovery — audit: needed at index time?
    let _ = span_stage("connectors");    // full.rs:1144 connector_start
    let _ = span_stage("db_write");      // SQLite commit
    let _ = span_stage("chunk_fts");     // code_chunks + FTS index
}
```

Keep the existing `phase_timer::scope(...)` calls INSIDE these stages (resolve.iteration_n, externals.build_symbol_index, etc.) — they nest under the coarse span and give the per-call aggregates spans can't. Net result: a span tree per index whose leaves carry phase_timer aggregates.

To emit **per-phase logs** without a backend, the dev subscriber uses span-close events:

```rust
tracing_subscriber::fmt()
    .with_span_events(FmtSpan::CLOSE)   // logs `<stage> close time.busy=… time.idle=…`
    .with_env_filter(...)
```

That alone turns the span tree into the per-phase log the architect asked for — no OTel, no collector.

### 1b. Pluggable subscriber (shared init, dev vs service)

Replace the four ad-hoc `fmt().init()` sites with one helper in the core lib (or a small `bw-telemetry` crate) that assembles a `Registry` from layers:

```rust
// bearwisdom::telemetry::init(profile)
let registry = tracing_subscriber::registry().with(env_filter);
match profile {
    Dev      => registry.with(fmt_layer().with_span_events(FmtSpan::CLOSE)),
    Profile  => registry.with(tracing_flame::FlameLayer::new(...)),   // → inferno flamegraph
    #[cfg(feature = "otel")]
    Service  => registry.with(tracing_opentelemetry::layer().with_tracer(otlp_tracer())),
}.init();
```

- **Dev** (CLI/bench): fmt + span-close → per-phase logs (today's need).
- **Profile**: `tracing-flame` → flamegraph for the slow projects (serverpod/grails) — the right tool for batch profiling, no infra.
- **Service** (MCP/Web): OTLP layer — Part 2, feature-gated.

### 1c. Dependency boundary (non-negotiable)

The core `bearwisdom` lib is **FFI-embedded in AlphaT and Lynx**. OTLP pulls in `opentelemetry-otlp` + tonic/gRPC + protobuf. That must **not** land in the embedded lib by default.

```
bearwisdom (core lib)     tracing-only. NO opentelemetry dep. Zero-cost facade for the embedded apps.
bearwisdom-mcp / -web     opt-in `otel` cargo feature → opentelemetry + tracing-opentelemetry + otlp
bearwisdom-cli / -bench   fmt + flame only; never otel
```
This is a dependency-boundary decision (keep gRPC out of the desktop apps), not a behavior gate — distinct from the grammar/env-var gate rules. The one env touchpoint, `OTEL_EXPORTER_OTLP_ENDPOINT`, is OTel-standard *config*, not a bug-hiding toggle.

### 1d. rayon caveat (write it down so it isn't relearned)

Coarse stage spans live OUTSIDE the rayon `par_iter`/`join` calls, so propagation is a non-issue. Do **not** `#[instrument]` functions called inside the parallel inner loops — context won't follow work-stealing without `Span::current()` capture + `in_scope`, and the per-call span cost distorts the measurement. Inside the parallel loops, the atomic `phase_timer` is the only safe instrument.

---

## Part 2 — OpenTelemetry on the service binaries (follow-up, its own piece)

Scope: **MCP server + Web API only.** These are long-running, request-driven — OTel's actual sweet spot (the batch indexer is not).

- **Traces.** Per-request root span on each MCP tool call (`bw_search`, `bw_investigate`, …) and each Web route; the indexing span tree (Part 1) nests under a request span when an index is triggered in-process. Export via `opentelemetry-otlp` → Jaeger/Tempo/Honeycomb.
- **Metrics.** Counters/histograms: tool-call latency by tool, index duration by phase, resolution_rate, cache hit/miss, DB pool wait. Export to a Prometheus/OTLP metrics endpoint.
- **Wiring.** `otel` cargo feature on the two service crates assembles the `tracing_opentelemetry` layer in the shared `telemetry::init`. Endpoint via `OTEL_EXPORTER_OTLP_ENDPOINT`; resource attrs (`service.name=bw-mcp|bw-web`, version) set at init. Graceful shutdown flushes the exporter (`opentelemetry::global::shutdown_tracer_provider()`).
- **.NET analogy** (for review): identical shape to instrumenting with `ActivitySource`/`Meter` and wiring `AddOpenTelemetry().WithTracing(...).AddOtlpExporter()` — the code is provider-agnostic, OTel is the exporter.
- **Explicitly out of scope:** OTel in the embedded core lib; OTel as the indexing-profiler (use flame, Part 1b).

---

## Part 3 — Single-project indexing optimization (MEASURED 2026-06-13)

`perf-measure` ran three long-running projects with `RUST_LOG=bearwisdom=info bw reindex --force` (no baseline write). Dominant stages:

```
groovy-nextflow  873 s   80.6% resolve.iteration_0    (one full pass vs 264k sym + 87k external files)
vue-hoppscotch  2076 s   51.9% resolve.iteration_n (calls=3, ~359s each) + 15.7% iter_0 + 21% return-infer*
zig-compiler     983 s   84.8% parse+write*           (17,130 files incl. C/C++ toolchain payload, 6,489 error-files)
   * = UNTIMED today, inferred from log windows — see instrumentation gaps below
```

### Root cause — the resolve fixpoint re-resolves the ENTIRE ref set every iteration

```
resolve/mod.rs:219  resolve_iteration_with_cached_index_and_arena
  └─ loop_body.rs:367  par_iter() over ALL files, resolving EVERY pf.refs vs the whole SymbolIndex — per iteration
engine.augment        calls=3, total=3s   ← augment only touches NEW files; NOT the problem
```
vue expand iter2 (+60 files, +63 edges) cost 369 s; iter3 (+9 files, **+0 edges**) cost 324 s. One index runs up to **~13 full O(all_refs × index) passes** (1 iter_0 + ≤8 expand `MAX_EXPANSION_ITERATIONS` + ≤3 return-infer `MAX_RETURN_ITERATIONS`, full.rs:1059), each re-walking everything regardless of delta.

### Ranked levers

**#1 — Delta-resolve (the −62% win).** Re-resolve only the delta each fixpoint pass: `{chain_misses}` ∪ `{refs whose inferred-return changed}` (the expand loop restricts to misses the new files can answer; the return-inference loop restricts to callers of functions whose inferred return changed — the 112/11 qnames in vue, not the whole project). Plumbing exists (`new_files_slice` threaded, `chain_misses` tracked) — the loop just doesn't restrict itself. Monotonic invariant: adding externals only turns misses→hits, never flips an existing hit ⇒ perf-preserving, not a resolution change. **GATE: `resolution_rate` byte-identical on a sample before/after.** Projected: vue 2076 s → ~700–800 s; helps every resolve-bound project (the per-file-cost outliers driving the corpus total).

**#2 — Internal-only SymbolIndex build passes** (`engine/index/build.rs`). `by_name`/`by_qname`/`inherits`/`type_refs` build passes run serially over ALL parsed files incl. ~87k externals; externals are lookup-targets-only and already have a separate demand-driven index. Restrict these passes to internal files → shrinks the one unavoidable `iteration_0` for every externals-heavy project (groovy's 80%).

**#3 — Reachability-bound C/C++ toolchain parse** (zig-class only). 17,130 files parsed incl. `lib/include`/`libc`/`libcxx`/`libtsan` toolchain payload (6,489 syntax-error files), all parsed+written though admitted as externals. Parse only headers the project's includes actually reach — the demand model TS/Python/Rust already use. Orthogonal to resolve; the right lever only for C/C++-payload projects. (Note: this is adjacent to the just-landed C2 include-driven admission — the admission *pull* is bounded, but the initial *eager* toolchain parse is not.)

**#4 — Monorepo-internal parallelism** (the one parallelism case): a single monorepo project with N packages can parallelize indexing across its own packages — still single-project UX. Lower priority than #1–#3.

### Instrumentation gaps to close FIRST (cheap, validates Part 1a)

1. **Project parse+write phase is untimed** — 85% of zig, invisible in PHASE_TIMER (inferred from a 13.9-min log gap). Add a `parse`/`extract`/`db_write` span+scope.
2. **Return-inference re-resolves (full.rs:1059) untimed** — 439 s / 21% of vue lands in the remainder. Wrap as `resolve.return_infer` so the full fixpoint cost is visible (it's part of the same resolve family being optimized).

Closing these makes the #1 win measurable and is the concrete first slice of Part 1a.

Target: single-project `full_index` latency down (vue −62% projected from #1 alone); the corpus recapture (test harness) benefits as a side effect, not as the goal.

---

## Sequencing

```
NOW        perf-measure (running) → first-cut phase breakdown + remainder size
THEN       Part 1a+1b instrumentation substrate (coarse spans + pluggable subscriber + FmtSpan::CLOSE)  ← rebuild, re-measure for COMPLETE per-phase logs
THEN       Part 3 optimization on the measured dominant phase(s); monorepo-internal parallelism
FOLLOW-UP  Part 2 OTel on MCP/Web (feature-gated), once the substrate is in
```

Cargo discipline: instrumentation lands as one substrate change, verified with a single serialized `cargo test -p bearwisdom --lib`; OTel feature compiled-checked on the two service crates separately.

---

## Part 4 — Implementation results + PIVOT (2026-06-13)

### #1 instrumentation — DONE, works
`parse_write.project` and `resolve.return_infer` scopes landed. They made the real cost visible (and found the pivot below). Suite 7,287/0.

### #2 delta-resolve — DONE, correct, but a MINOR lever (the fixpoint was a symptom)
Landed as a worklist fixpoint (frontier = chain-miss source files via a thread-local `CURRENT_SOURCE_FILE` tag on `ChainMiss.source_path`; `iteration_0` + all incremental paths stay full). **Correct** — `unresolved` byte-identical within rayon nondeterminism noise across an 8-project gate (vbnet/go+ts/rust/java/php small + groovy/vue/zig big): vbnet 132→132, java 55→55, go 20→20, sql-pgmq 171±1, twig 110→108, zig 22355→21888, nextflow 15864±2, vue 11733→11705. (Edge counts jitter ±3 from duplicate-target-ref ordering — the svelte-shadcn class — so the gate is `unresolved`, not edge-rows.)

**But it does NOT reduce latency on the big-3**, because the frontier stays ≈ all files:
```
vue   2306s  resolve.iteration_n 1,100,631ms (3×367s)  resolve.return_infer 723,682ms (2×362s)  — UNCHANGED
nextflow 838s  resolve.iteration_0 689,150ms (one full pass)  — iter0-bound, #2 doesn't touch it
zig   745s   parse_write.project 632,764ms (85%)  — parse-bound (#3/#4 territory)
```
vue has PERMANENT external-chain misses in nearly every file (chains into libraries that can't be walked) → those files stay on the frontier every pass → no shrink. The fixpoint is not the disease.

### THE DISEASE — externals over-pull (the pivot, architect-directed)
vue's index carries **1.02M EXTERNAL symbols** for 2,125 internal files; each resolve pass is O(refs × 1M index) ≈ 360 s, run ~6×. The external set:
```
ext:ts:  551,784 syms  (node_modules .d.ts — pulled EAGERLY / broadly)
ext:idx: 471,382 syms  (DEMAND path — the ENTIRE `windows` Rust crate: Foundation/mod.rs 10,625 syms,
                        Wdk 14,721, Search 12,115, UI/Controls 10,244, ... + the ENTIRE Go stdlib)
```
A TypeScript app is indexing the whole Win32 API + Go stdlib because of ~one incidental `use windows::...`. **Architect's two directives:**
1. **Demand-driven must be symbol-granular** — pull only the actual reference + its type-hop closure, NEVER the whole package/crate re-export tree. The eager `ext:ts:` path must also become reference-bounded.
2. **Index externals once** — version-pinned, deterministic externals are re-pulled+re-parsed+re-symbol-indexed every reindex; build once into a persistent/shared store keyed by package@version (or content-hash), reuse across reindexes and projects.

This aligns with `feedback_reachability_based_externals` (an unfinished principle). It is the real lever: cut the index ~10× → every pass (incl. the unavoidable iter0) drops proportionally → vue plausibly 2300 s → ~400–600 s, and it generalizes to every externals-heavy project.

**Status:** `externals-map` agent investigating the eager-vs-demand paths, the exact over-pull loci (TS whole-node_modules + the Rust re-export over-follow), the (non-)caching state, and designing both fixes. #2 retained as correct substrate; #3 (internal-only index build, for nextflow's iter0) and #4 (zig toolchain parse) remain valid secondary levers. Gate for the externals fixes: `unresolved` byte-identical (bounding must drop only UNREACHED symbols).
