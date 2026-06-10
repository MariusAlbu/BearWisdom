# BearWisdom Master Plan

Operationalizes RESEARCH-DIRECTION.md and reconciles it with the IDE goal. Companion doc: RESEARCH-DIRECTION.md (direction), this file (analysis + execution).

## Verdict on RESEARCH-DIRECTION.md

Agree with the core thesis: eval-first, one high-level tool, evidence packets, token economy as a product feature. Three amendments:

1. **The semantic moat is the resolution engine, not the router.** Grep, engram, LightRAG, and every embedding-RAG tool can fuse retrieval strategies. None of them have a 95%+ cross-language resolved reference graph with confidence scores, typed edges, and Roslyn-style symbol identity. `bw_research` is a *presentation layer* over that graph — evidence quality (precise citations, "what calls this", "what breaks") is downstream of resolution quality. The current `feat/resolution-engine` work is not a detour from the research direction; it is the input to it. Sequencing below keeps it first.

2. **IDE is not secondary — it's the same substrate with a second frontend.** RESEARCH-DIRECTION.md demotes the IDE; the actual product goal keeps it. The reconciliation: one semantic model (symbols, edges, types, identity), two consumers:
   - **LLM frontend**: MCP evidence packets (`bw_research`).
   - **IDE frontend**: LSP facade / direct Tauri FFI (AlphaT already consumes the crate by path).
   The deltas for IDE-grade are small and enumerable (rename, hover, document symbols, signature help, sub-second incremental) — see Workstream E. An LSP server prototype already exists in the `cmake-resolution` worktree (`crates/bearwisdom/src/lsp/`).

3. **Extend, don't rebuild.** The codebase already contains most of the proposed machinery in embryo:

| RESEARCH-DIRECTION.md proposal | Already exists | Gap |
|---|---|---|
| Eval harness | `benchmarks/` bw-bench: task schema, 3-condition runner (MCP/CLI/native), deterministic scoring, token/tool-call/wall-clock metrics, md+json reports | judge scoring, `must_mention`/`exclude_paths`, hand-authored tasks, regression gate |
| `bw_research` pipeline | `query/context.rs:223` `smart_context`: FTS seed → BFS graph expand → 5-weight composite score → token-budget prune → per-symbol `reason` | query-shape routing, grep/vector/refs as candidate sources, evidence-packet output, scope filter, cursors |
| Retrieval fusion | `search/hybrid.rs:80` RRF (FTS5+KNN) with `text_rank`/`vector_rank` provenance | RRF over *all* sources (grep, symbol, refs, graph), not just FTS+vector |
| Evidence packets | `InvestigateResult` (composite), `RankedSymbol.reason`, compact-v1 formatter with file registry + `truncated:true` | `match_sources`, unified confidence, line spans, snippets, omission counts, cursors |
| Token budgets | `bw_context` budget param + `estimate_tokens` per kind (`context.rs:89`) | budget on every tool / on `bw_research`; omission accounting |
| Semantic search | chunker (AST-aligned, 512-tok), CodeRankEmbed ONNX, sqlite-vec KNN, graceful FTS-only fallback | query instruction prefix NOT applied (bug), no status observability, no regression suite |

## Current-state inventory (surveyed 2026-06-10)

**Strong (production-ready):**
- Structured type system: `TypeId` + `TypeArena`, algebraic `Type` enum (Apply/Generic/Union/Optional/Literal), interned identity — `type_checker/core/types.rs`.
- Symbol identity Stages 1–2: stable key persisted (`symbol_key.rs`), survivor-matching incremental write, containment + inheritance as id edges.
- IDE query primitives: `goto_definition`, `find_references` (bulk IN query, edge-kind + confidence), `complete_at` (3-tier, nucleo-ranked).
- Resolution engine: single-tier generic ladder (`DefaultResolver` + chain walker), LanguageProfile data / LanguageEngineHooks code split, corpus ~95.5%.
- Incremental indexing: git-diff / hash-diff / watcher-event paths, blast-radius re-resolve, debounced `notify` watcher in `IndexService`.
- MCP surface: 18+ tools, compact-v1 (40–60% token savings), slim defaults, audit log with per-call token estimates.
- SCIP import (no export).

**Gaps (ranked by leverage):**
1. No retrieval router / no `bw_research` — callers pick backends manually.
2. CodeRankEmbed query prefix missing at `search/hybrid.rs:105` — semantic recall silently degraded. Cheapest high-impact fix in the plan.
3. bw-bench ground-truth circularity — tasks are auto-sampled from BW's own `blast_radius`/`references` output, so the BW condition is graded against its own answers. Invalidates the headline claim until fixed.
4. No cursors/pagination anywhere; truncation is a binary flag.
5. No omission accounting, no `match_sources`, confidence signals scattered (edge.confidence vs rrf_score vs BM25) and uncalibrated.
6. No rename, hover, document-symbols, signature-help; LSP loop not in main branch.
7. Full-index perf on large TS projects (ts-nextjs ~41 min) — enterprise blocker for both frontends.
8. No semantic-search observability (`bw status` doesn't report chunk/vector/model state).

## Strategic frame

```
                    ┌────────────────────────────────────┐
                    │  Semantic substrate (the moat)     │
                    │  resolution engine · type system   │
                    │  symbol identity · flow edges      │
                    └──────────────┬─────────────────────┘
              ┌────────────────────┼────────────────────┐
              ▼                    ▼                     ▼
     LLM frontend          IDE frontend           Eval harness
     bw_research           LSP facade / FFI       bw-bench v2
     evidence packets      rename·hover·refs      (gates everything)
```

Every workstream below strengthens the substrate or one of the three consumers. Nothing builds a parallel system.

## Workstreams

### A. Close the resolution-engine branch (substrate) — IN FLIGHT, finishes first

Current branch work continues to its existing definition of done (COMPILER-RESOLUTION.md: 99%-through-one-engine gate, symbol-identity Stage 3 survivor-matching bulk write). No new scope from this plan. One corpus recapture at closeout, per standing rule.

**Why first:** evidence-packet confidence, rename safety, and the benchmark's citation precision all read from this graph. Shipping `bw_research` over a 95% graph then re-shipping over a 99% graph means benchmarking twice.

**Verify:** corpus recapture ≥ baseline floor; symbol ids survive incremental reindex on a TS + C# smoke project.

### B. Eval harness v2 — two tiers, model-free gate

The eval mirrors the engine: the engine is model-free (except local embeddings), so the **gate** is model-free too. Splitting "eval" into two tiers is what keeps any API/subscription cost off contributors and adopters — only one tier ever touches a model, and it's never the gate.

**Tier 1 — Engine eval [model-free · free · runs in CI on every commit · THE GATE].**
Call `bw_research(question)` / `bw_read` / navigation queries *directly* — no agent driving them — and score the returned packet against hand-verified ground truth. IR-style deterministic scoring: recall@budget, precision, citation accuracy (file/symbol/line overlap), packet token volume, `must_mention` hit, exclude-path violations. Zero API, zero subscription, runs offline. This directly answers "did retrieval get better" — the leading indicator, no agent noise — and it measures token efficiency *per call* (packet size, recall-within-budget). This is the everyday development gate.

**Tier 2 — Agent benchmark [needs a frontier model · occasional · the comparative artifact].**
The grep-vs-BW claim: an agent solves tasks through each condition, measuring session-level behavior (tool calls, total tokens, wall-clock, fallback-grep rate). Inherently needs an agent, so it costs API/CLI — but it is **not** a per-commit gate and **not** something an adopter or contributor runs. You run it rarely, to produce the headline number.
- Subject model: remote API / Claude Code CLI — your cost, occasional.
- **Record-replay makes regression free:** capture real agent trajectories once, store as fixtures, replay + re-score deterministically. Re-capture only when the tool contract changes. After the one-time capture, even Tier 2 regression costs nothing.
- **No judge in the default gate.** Deterministic scoring is the line in both tiers. A judge (a *different* model family — GPT/Gemini — to avoid Claude-judging-Claude self-preference) is an opt-in flag for the answer-quality number only.

**Model dependency, explicit:**

| Role | Where it runs | Cost to run eval | Who runs it |
|---|---|---|---|
| none (Tier 1 scoring) | engine output vs ground truth | free, deterministic, offline | CI, every commit, anyone |
| Embeddings (CodeRankEmbed) | local ONNX, semantic tasks only | free after `bw models install`, no API | anyone with BW installed |
| Subject model (Tier 2) | remote API / Claude Code CLI | API/subscription, occasional | you, when producing the claim |
| Judge (opt-in) | remote API, different family | API, opt-in only | you, answer-quality number only |

**Shared plumbing:**
1. **Fix ground-truth circularity.** Hand-author 30–50 tasks across 4–6 corpus projects (ts-rallly, SimplCommerce, a Python and a Java project, BW self-host). Ground truth verified by reading the code, not by querying BW. Keep auto-sampled tasks as a separate "structural" suite (regression-useful, not headline-valid).
2. **Schema additions** (backward compatible): `must_mention`, `exclude_paths`, `difficulty` on `BenchmarkTask` (`benchmarks/src/task.rs:56`).
3. **Conditions (Tier 2 only):** add `UseBwResearch` as a fourth condition; add explore-agent baseline (subagent with native tools).
4. **Regression gate:** Tier 1 runs every commit (free); Tier 2 replays fixtures (free) → `bw-bench report --baseline <prior.json>` per-metric deltas, fail on composite regression.

**Verify:** Tier 1 runs with no API key set and emits md+json; Tier 2 reproduces native-vs-BW deltas within IQR on `--repeat 3`; replayed fixtures score identically to live capture.

### C. `bw_research` — thin first, then router

**C1. Thin version.** Generalize `smart_context` rather than writing a new pipeline:
- Input: `{question, scope, mode: "auto", filters, budget: {max_tokens, max_results}, cursor?}`.
- `scope` = path prefix + the existing `search/scope.rs` filters; `filters` = tests/docs/generated/vendor exclude-by-default with counts.
- Candidate sources for v1: existing seed search + graph expansion + `find_references` + grep (exact-string fallback). Each candidate carries `match_sources` (the `text_rank`/`vector_rank` provenance pattern from `HybridSearchResult`, extended to all sources).
- Output: evidence packet — `summary` field **omitted** (see Open Questions), `evidence[]` with `file`, `line_start/end`, `symbol`, `kind`, `reason`, `match_sources`, `confidence`, `snippet`; `relations[]` from edges among evidence symbols; `omitted: {tests, generated, vendor, low_confidence, budget_dropped}`; `next_actions[]` with cursors.
- Render via `CompactFormatter` — packets get compact-v1 for free.

**C2. Cursors.** The MCP server already has `session_id` + an audit table. Cursor = persisted ranked-candidate set keyed `(session_id, query_hash)`; expansion replays from the stored set (no re-query). TTL-evict on reindex.

**C3. Router.** Deterministic query-shape classification (quoted/error-text → grep+FTS; identifier-shaped → symbol/defs/refs; "what happens when" → entry-points+flow+call-graph; vague NL → hybrid+concepts; "what breaks" → blast-radius+refs). Fuse all selected sources with the existing RRF, then re-rank with the 5-weight scorer extended by: exact-hit bonus, path-class penalty, graph distance, relation strength. Log the chosen plan into the packet (`plan` field) — explainability is a debugging and trust feature.

**C4. Unified confidence.** One calibrated 0–1 per evidence item, derived from (source mix, resolution edge confidence, rank). Calibrate against eval-harness outcomes, not intuition.

**Verify (each sub-stage):** B-suite composite improves vs prior stage; packet token volume ≤ budget on every task; zero packets without citations.

### D. Semantic path reliability — small, parallel-friendly

**D0. Pick the embedding backend from the eval, not by assumption.** Embeddings in BW are the *seed* stage — `smart_context` does seed → graph expansion → 5-weight rerank, so structure refines recall and a weaker first-stage seed is tolerable end-to-end. Run three rows on the D5 semantic regression set before committing:
- **A — lexical+graph only** (FTS5/BM25 + resolution graph, 0 MB, 0 deps). The floor. If semantic's lift over this is small, the easiest model is *no model*.
- **B — static token embeddings** (Model2Vec/potion: tokenize → vector lookup → mean-pool; pure Rust over the existing `tokenizers` crate or `model2vec-rs`; ~8–30 MB; **no ONNX runtime**). Lower raw quality, but graph rerank likely absorbs most of it.
- **C — CodeRankEmbed** (today: 132 MB quantized ONNX, best raw recall).

Target architecture: tier the backend — **default = B** (dependency-free, tiny, makes `cargo install bw` genuinely easy), **opt-in = C** via `bw models install coderankembed` for teams wanting max recall, **floor = A** when no model present (hybrid already degrades to FTS). Rejected runtimes: hosted embedding APIs (reintroduce the API-key/network/privacy burden); SPLADE/learned-sparse (still needs a transformer at index → same ONNX friction); **sentence-transformers / PyTorch (C via Python)** — same 548 MB weights as the ONNX export plus ~0.8–1 GB of Python+torch runtime, and unshippable in-process in a Rust/Tauri engine (forces a Python subprocess or PyO3 bridge). The ONNX-in-Rust path already runs CodeRankEmbed *directly*; sentence-transformers adds a runtime, not a capability. (Legitimate only as a throwaway offline script to validate quality while deciding A/B/C — never in the product.) The eval rows decide A-vs-B-vs-C; everything below in D applies to whichever wins.

1. **Bug fix first:** apply the CodeRankEmbed query instruction prefix in `embed_query` (`hybrid.rs:105`) — document side stays unprefixed. Add a regression test asserting the prefix. (Applies only if C wins; static embeddings have no query/doc asymmetry.)
2. `bw status` reports: chunk count, vec count, model name/dim, backend state, embedded coverage %.
3. `bw embed` failure surfacing: missing model / ORT dylib / dim mismatch / zero-vector → explicit errors.
4. **Model + runtime packaging (tiered, not one-size).** Two distinct frictions:
   - **ONNX runtime:** static-link the `ort` crate (static feature) so the runtime compiles into the binary — kills the `ORT_DYLIB_PATH` setup failure for everyone. ~15–40 MB binary cost. Do regardless of the weight decision.
   - **Model weights:** only the **quantized int8 model (132 MB)** is a bundling candidate — the embedder prefers it; the 522 MB fp32 is fallback-only and never shipped. Don't `include_bytes!` it into the core binary (taxes the structural/FTS path — the majority use — and couples model updates to releases). Instead: **auto-fetch on first semantic use** for the standalone CLI/MCP (`cargo install bw` stays lean; FTS-only degrades gracefully until the one-time pull); **bundle in the installer** for AlphaT/Lynx (Tauri bundles already, 132 MB is noise, offline-first matters); **offline artifact** (`bw --with-model`) for air-gapped enterprise.
   - `bw models list|install|status` drives the fetch path; no model in the core binary.
   - ⚠️ Validate quantized int8 recall vs fp32 in the D5 semantic regression suite before committing to quantized-only — if int8 recall drops materially, fp32 (522 MB) is the only present alternative (no fp16 ONNX on disk). Regression numbers decide.
5. Semantic regression mini-suite inside bw-bench: vague-query → expected chunk/symbol.
6. Hybrid responses already carry `vector_rank`; surface "vector-backed: yes/no" in tool output meta.

**Verify:** fresh-clone → `bw models install` → `bw embed` → vague-query benchmark cases pass; `bw status` distinguishes FTS-only from vector-backed instantly.

### E. IDE frontend — runs parallel to C after A lands

1. **Document symbols / hover / signature help** — thin queries over existing data (`symbol_info` + file lookup); near-zero new logic.
2. **Rename** — `find_references` (id-based, post-Stage-3) → source-edit plan `{file, line, col, len, new_text}[]`. BW returns the edit plan; the IDE applies it. Conflict detection via symbol-key collision check.
3. **LSP facade** — promote the worktree `lsp/` prototype into a `bearwisdom-lsp` crate: textDocument/definition, references, documentSymbol, hover, completion, rename, publishDiagnostics. All methods delegate to `query/`; no new semantics in the facade (library-layer rule).
4. **Incremental latency budget** — define the IDE SLO now: keystroke-file reparse < 100 ms, single-file re-resolve < 500 ms, blast-radius re-resolve async. Measure via `bearwisdom-bench`; optimize only what misses.

**Verify:** AlphaT consumes defs/refs/hover/documentSymbol via FFI on a real project; rename round-trips on the TS smoke corpus with zero broken references afterward.

### F. Enterprise hardening — continuous, measured not vibes

1. **Full-index perf:** diagnose ts-nextjs ~41 min (known open issue). Profile before optimizing.
2. **Daemon mode:** `IndexService` already owns watcher + pool; expose it as a long-lived process the CLI/MCP/LSP all attach to (one index, N consumers) instead of per-process pools.
3. **Multi-repo workspaces:** ServiceCache already does multi-project; add cross-project queries only when a consumer needs them (AlphaT will).
4. **SCIP export** — interop credibility + migration path for teams with existing SCIP tooling. Import exists; export is the inverse walk.

### G. Agent substrate verbs — beyond search

RESEARCH-DIRECTION.md optimizes one verb (search → evidence). Agents spend most tokens elsewhere: reading files, validating edits, deciding what to re-test. These extend BW from "better search" to the agent's read/write substrate — capabilities with no grep equivalent.

**G1. `bw_read` — skeleton-first file reading.** The largest token sink in agent sessions is `Read`, not search. Return a file as a foldable skeleton: signatures + doc heads, bodies elided, per-symbol expansion cursors. `bw_file_symbols` mode="outline" is ~80% of this; missing pieces are body-expansion handles (reuse C2 cursor infra) and range-read ergonomics. Expose as MCP tool + CLI; promote alongside `bw_research` as the second primary verb.
- **Verify:** benchmark task set re-run with `bw_read` replacing native Read in the BW condition; returned-token reduction on file-comprehension tasks ≥ 3x with equal answer accuracy.

**G2. Post-edit structural verification — the agent inner loop.** After an edit, incremental reindex + diagnostics diff answers "you broke N call sites" sub-second, no compile, every language. Mechanism: `reindex_files` (already event-driven) → snapshot unresolved-refs + edge set before/after → compact delta packet (new-unresolved, broken-edges, touched-symbols). Symbol-identity survivor matching makes the before/after join precise.
- **Verify:** scripted edit that breaks 3 known call sites on the TS smoke project → delta packet names exactly those 3; round-trip < 1 s on a single-file edit.

**G3. Symbol-level git temporality.** Stable symbol keys + git enable per-symbol history ("when did this signature change, with what else") and structural diff between refs — survivor matching applied across commits instead of reparses, classifying changes as rename/move/edit. Replaces multi-call blame-and-read archaeology with one query.
- **Verify:** `bw history <symbol>` on a symbol with a known rename across commits reports the rename as one identity, not delete+add.

**G4. Test-impact mapping.** Entry points already classify test functions; reverse reachability from changed symbols to test entry points yields `bw affected-tests <symbol|diff>`. Pairs with G2: edit → verify → run only the tests that reach the change.
- **Verify:** on a project with a known test suite, a single-function edit maps to the exact test set that exercises it (validated against actual test runs).

**G5. Zero-result telemetry flywheel.** The audit table already logs every call with params and result size. Mine it: zero-result queries and queries followed immediately by fallback grep are the product backlog, measured from real usage. `bw audit report` → ranked failure patterns.
- **Verify:** report runs over this repo's own dogfood sessions and surfaces ≥ 1 actionable retrieval gap.

**G6. `bw map` — token-budgeted repo priming.** Generate the repo map an agent needs in its system prompt: top-central symbols, package graph, entry points — under a token budget. Composition of `architecture_overview` + `workspace_graph` + the context scorer; structural where aider's equivalent is text-rank.
- **Verify:** 2k-token map of a corpus project; an agent primed with it solves benchmark tasks in fewer tool calls than unprimed.

## Answers to RESEARCH-DIRECTION.md open questions

| Question | Answer | Reason |
|---|---|---|
| Summarize inside `bw_research`? | **No.** Return structured evidence only. | Keeps core model-free (no API key, deterministic, testable); the frontier model is better at synthesis than anything BW would embed; summaries would be the one un-benchmarkable field. |
| Default token budget | 6,000, capped hard | Matches the doc's sketch; eval harness tunes it empirically. |
| Default filters | tests/generated/vendor **exclude**, docs **auto** — always with omission counts | Recoverable via filters param; counts prevent "covered everything" illusions. |
| Continuations | Session-keyed persisted candidate sets (C2) | Reuses existing session_id + audit infra; no re-query cost. |
| Reranker model? | Heuristic-first; revisit only if eval shows a ceiling | Same gate as TurboVec — measured bottleneck or it doesn't happen. |
| Command surface visibility | MCP: `bw_research` + `bw_read` + `bw_reindex` + 4–6 navigation tools (defs/refs/hierarchy/file_symbols) promoted; rest demoted to CLI-only | Navigation tools serve the IDE/debugging path and precise lookups where the question is already structural; everything exploratory routes through `bw_research`, everything file-comprehension through `bw_read`. |
| TurboVec | Deferred, criteria unchanged | sqlite-vec is not a measured bottleneck. |

## Execution model — two parallel tracks

Two tracks run concurrently on **isolated git worktrees** (separate `target/` dirs → no cargo-lock collision; the hard no-parallel-cargo rule is satisfied by isolation, not serialization). Each track is driven by workflows; the architect reviews at join points.

- **Track R — Resolution** (RESOLUTION-FIX-PLAN.md) on `feat/resolution-engine` (main checkout). Drains unresolved refs through the generic engine. Owns the corpus recapture.
- **Track P — Product** (this plan, Workstreams B–G) on a `feat/research-tool` worktree. Builds eval harness, `bw_research`, semantic, IDE frontend, agent verbs against the live graph.

There is no line-ending precondition: the worktree noise was investigated and is **not** CRLF (0 pure-CRLF files; the churn was real content + a workspace `cargo fmt`, now committed in `cadbd7ae`). Nothing to normalize.

### Track R (resolution) — order per RESOLUTION-FIX-PLAN.md

| Step | Work | Risk |
|---|---|---|
| R0 ✅ | checkpoint `cadbd7ae` + nim test fix `6e63ce51`, workspace green (6748/0) | done |
| R1 | 4b corpus-scope accounting + B2 namespaceless flips (~14k) | near-zero |
| R2 | C profile-data sweep (~55k), one-agent-per-family | near-zero |
| R3 | B1 import/alias rebind + B3 implicit-prelude qualify (~200k) | regression-bearing — the big levers |
| R4 | 4a vendoring manifests; B4 receiver projection; B5 + A extractor wave; E externals install | mixed |
| R★ | **symbol-identity Stage 3** (survivor-matching bulk write) | **= J1** |
| Rc | corpus recapture (the one recapture) | closeout |

### Track P (product) — Workstreams B–G against the live graph

| Step | Work | Gate |
|---|---|---|
| P1 | B harness Tier-1 (model-free) + ground-truth authoring; D0/D1 backend eval + prefix bug | none — start now |
| P2 | C1 thin `bw_research` + B Tier-2 fourth condition | none |
| P3 | C2 cursors + C3 router; D2–D6 semantic reliability; G1 `bw_read`; G5 telemetry; G6 `bw map` | none |
| PE | IDE frontend: hover · document-symbols · signature-help · LSP facade · Tauri↔CodeMirror wiring | none for these |
| P4 | C4 confidence calibration | **J2** |
| PE★ | E rename · G2 edit-verification · G4 affected-tests · G3 git temporality | **J1** |
| Pc | Tier-2 publication numbers | **J2 + Rc** |

### Join points (R → P dependencies)

- **J1 = R symbol-identity Stage 3** → unblocks P: rename, edit-verification, affected-tests, git temporality (all need stable ids surviving reindex).
- **J2 = R near-final graph** (post B1/B3) → unblocks P: confidence calibration + the published benchmark number (calibrate/publish against a stable graph, not a moving one).

### Cadence

- Track P rebases onto Track R at each R milestone (R1, R2, R3…). Pin the engine commit when measuring a pure-retrieval delta so graph-drift isn't misattributed to a retrieval change.
- Tier-1 eval (free, model-free) runs continuously on P as the dev signal. Tier-2 publication is frozen until J2 + Rc.
- Bottleneck split — yours: ground-truth authoring (P1), router taxonomy review (P3), AlphaT integration (PE), methodology sign-off (Pc). Mine: everything else, driven by workflows.

## Risks

1. **Ground-truth circularity (B1)** — if unfixed, every benchmark claim is attackable; per the 2026-05-23 decisions, transparent methodology *is* the marketing. Fix before publishing any number.
2. **Router becomes a regex pile** — mitigate: router decisions logged in packets, eval-gated, and capped at query-*shape* classification (never per-library or per-framework rules — same discipline as the no-predicate-stuffing rule).
3. **Indexing perf debt (F1)** — both frontends inherit it; ts-nextjs diagnosis should not slip past phase 3.
4. **Scope creep into LightRAG territory** — BW's graph is structural, not LLM-extracted. No LLM calls inside the engine. The model-free Tier-1 eval is the gate; the only model in eval is the occasional Tier-2 subject (your cost, not adopters'), and embeddings are local ONNX.
