# BearWisdom — Competitive Analysis & Strategy

**Date:** 2026-06-12
**Scope:** Honest engineering comparison of BearWisdom against the 50k+ star "codebase knowledge graph" tier, the deeper static-analysis landscape, an asymmetric bypass strategy, and a survey of differentiating features buildable from existing data.
**Method:** Four parallel research agents — two deep-diving the named competitors against source, one mapping the landscape, one profiling BearWisdom from its own index and corpus. Every claim verified against source files, issue trackers, or the live GitHub API; README marketing treated as adversarial.

---

## Executive verdict

**Neither 50k★ project is a code-intelligence engine.** Both `graphify` (66k★) and `Understand-Anything` (58k★) are LLM-graph skills — tree-sitter pre-chunking that feeds LLM transcription/clustering, then queried by name matching. On every axis with a mechanism behind it (reference resolution, type inference, measured accuracy, determinism, data egress, scale), BearWisdom is categorically ahead. On the one axis that has no mechanism — distribution — they are categorically ahead: 66k★ and 58k★ versus BearWisdom's **1 star, not on crates.io**.

The real technical threats are not these two. They are `codegraph` (48k★ — BearWisdom's pipeline minus the type system) and `codebase-memory-mcp` (3.4k★ — the only project anywhere attempting owned-index multi-language type inference with a published eval).

**The core landscape finding is a clean inversion: popularity and depth are disjoint.** Everything above 20k stars either has no resolution (agent harnesses, RAG, wiki generators) or name-level resolution without a type system. Everything with compiler-grade resolution is archived, corporate-internal, or under 4k stars. BearWisdom's measured ref-level resolution corpus (86.94% application-class over 257 projects, 8.5M edges, engine-only at confidence 1.0) **has no published peer at any star level** — and is invisible at 1 star.

---

## The mechanism gap, on one call site

```ts
const repo = new UserRepository();
repo.findOne().email;
```

```rust
// BearWisdom        ✅ local_type(repo)→UserRepository → MembersIndex.lookup(findOne)
//                      → return_type map → walk .email — 86.94% measured over 8.5M edges,
//                      engine-only, confidence 1.0, zero heuristic tier
```
```python
# graphify           ❌ symbol_resolution.py: `if raw_call.get("is_member_call"): continue`
#                       — member calls SKIPPED BY DESIGN. Bare calls resolve only when
#                       exactly one node in the whole graph shares the lowercase name.
```
```ts
// Understand-Anything ❌ typescript-extractor.ts records callee as raw text;
//                       file-analyzer.md tells the host LLM to link it "when confident".
//                       Maintainer's own comment: agents drop ~25% of even the
//                       deterministic import edges (merge-batch-graphs.py re-injects them).
```

---

## Head-to-head

| Axis | BearWisdom | graphify (66k★) | Understand-Anything (58k★) |
|---|---|---|---|
| What it mechanically is | resolution engine → SQLite symbol graph | GraphRAG summarizer (Leiden communities, NetworkX→JSON) | prompt-orchestration skill → one `knowledge-graph.json` |
| Reference resolution | ✅ ref-level, cross-file, externals | ❌ unique-global-name match; member calls skipped | ❌ file-level imports only; calls = LLM-linked text |
| Type inference | ✅ `type_checker/` — aliases, generics, inheritance, chains | ❌ none (type annotations stored as strings) | ❌ none (no extends/types captured at all) |
| Accuracy measured | ✅ 86.94% app-class, 257 projects, regression floors | ❌ "71.5x tokens" is synthetic: `corpus_words = nodes × 50` | ❌ none; optional LLM grading an LLM |
| LLM needed to index | ✅ zero (local ONNX embeddings optional) | ⚠️ code-only path is LLM-free; docs/images/video require API | ❌ mandatory — ~157k tokens for scan phase alone |
| Code leaves machine | ✅ never | ⚠️ only if LLM pass used (Ollama escape hatch) | ❌ by design, to whatever backs the host agent |
| Scale | ⚠️ 930k-edge repo ✅ but ~60-70 min (known weakness) | ❌ in-memory NetworkX → JSON; ghost nodes on `--update` (#1152) | ❌ ~100-file comfort zone; git-lfs advised at 10MB JSON |
| Query surface | 20 MCP tools + 30 CLI cmds, compact format | MCP: BFS + shortest-path over substring match | no MCP; fuse.js over LLM prose, 1-hop expansion |
| Tests / langs | ~7,150 tests / 95 plugins | 83 test files / ~36 grammars, uniform-shallow | 59 test files / 12 grammars |
| Engineering shape | one engine, profile-driven | 11.6k-line `extract.py` god file; 8 branch rewrites in 10 weeks | live supply-chain attack in tracker (#432) |
| Distribution | ❌ 1★, build-from-source, no registry | ✅ 16+ assistants, ~1.2M PyPI downloads | ✅ 15+ platforms, viral dashboard |

All three are MIT, all bus-factor 1, all born March/April 2026.

**Star-count caveat.** graphify went 0→66k in 10 weeks as a Karpathy-wish fast-follow on X (HN: 2 points). Understand-Anything gained +24k in one week, with HN commenters alleging purchased stars and a `karpathy-llm-wiki` SEO topic Karpathy never endorsed. tree-sitter — the parser both ride on — took 13 years to reach 25.8k. Treat the tier's counts as marketing signal, not adoption evidence. The trustworthy organic datapoint in the niche is Serena (25k★ over 15 months).

---

## Per-competitor mechanics (verified against source)

### Understand-Anything (Egonex-AI, ex-Lum1104)
- **Repo relationship:** `Lum1104/Understand-Anything` was *transferred* (not forked) to the `Egonex-AI` org — old URL 301-redirects. License: "MIT © Yuxiang Lin and Infinite Universe, Inc." Bus factor ≈1 (Lum1104: 458 commits; #2 contributor: 10).
- **Parsing:** real CST pre-parse (`web-tree-sitter`, 12 extractors in `packages/core/src/plugins/extractors/`) → emits names + line ranges + a `callGraph` of raw text tokens. Extractors capture **no** `extends`/`implements`, no property types, no generics, no scope tree. The host LLM (`agents/file-analyzer.md`, 33KB prompt) transcribes this into a concept graph with summaries.
- **Reference resolution:** file-level imports only, but genuinely good — `skills/understand/extract-import-map.mjs` (66KB) does per-ecosystem path resolution (tsconfig aliases, Go module prefixes, PSR-4, Rust mod probing) across ~10 ecosystems. **Calls are not resolved**; the LLM links them "when confident" against a neighbor map. `merge-batch-graphs.py`'s `recover_imports_from_scan()` exists because, verbatim, agents "drop ~25% of them on real projects."
- **Type inference:** none.
- **Accuracy:** unmeasured. No benchmark, no fixture corpus. Quality control is an optional LLM reviewer grading LLM output.
- **Scale:** `SKILL.md` warns at >100 files. One flat `knowledge-graph.json`; README recommends git-lfs at 10MB+. Incremental design exists but is buggy (#402: silently loses unchanged nodes).
- **Query surface:** slash-command skills, not tools. fuse.js fuzzy match over LLM-written summaries + 1-hop expansion. **No MCP server.** Advertised "semantic search" is dead code (`embedding-search.ts` — store.ts admits nothing computes embeddings).
- **LLM dependency:** mandatory. Scan phase alone ~157k tokens. All source code egresses to the host agent's model. No offline mode possible.
- **Enterprise:** MIT; 59 test files + CI; `SECURITY.md` exists but private reporting disabled; **live supply-chain attack in issue #432** (decoy PR appending obfuscated payload to `astro.config.mjs`).
- **Adoption:** +24k stars in the single week of May 21–28 (Chinese dev-media wave + `karpathy-llm-wiki` keyword-riding). HN thread (169 pts) is largely critical; multiple commenters allege bought stars.

### graphify (safishamsi)
- **Parsing:** all 45 language extractors in **one 11,581-line `extract.py`** (maintainer's own issue #1212 asks to split it). Top-level + direct class members only — flat `nodes` list, no scope tree, no symbol table. Calls recorded as raw callee strings. "Video ingestion" = faster-whisper audio transcription (`transcribe.py`); "image ingestion" = vision-LLM captioning (`llm.py`).
- **Reference resolution:** `symbol_resolution.py` is the whole resolver. Resolves top-level `from X import Y` on unique `(module_stem, name)` match; **skips all member calls by design** (`if is_member_call: continue`); resolves a bare call only if exactly one node in the entire graph shares the lowercase name. Cross-file inheritance breaks (#1186). Everything else is LLM-extracted edges (0.55–0.95 confidence rubric) + Leiden community clusters (`cluster.py`, `graspologic.partition.leiden`). No embeddings. Has a `scip_ingest.py` escape hatch — i.e., real resolution is outsourced to compiler-grade indexers when present.
- **Type inference:** none. Type annotations stored as string edges, never inform dispatch.
- **Accuracy:** none. `benchmark.py` measures token compression only and *invents* corpus size when not supplied (`corpus_words = nodes × 50`). The marketed "71.5x fewer tokens" is this estimate on a 52-file demo; the same table admits ~1x on 6 files.
- **Scale:** NetworkX in-memory → `graph.json`, optional Neo4j. Incremental is grow-only and corrupts (#1152 ghost nodes, #1283 loses re-extracted nodes). Only published perf: "1.66x less time than sequential" on 84 files. No monorepo numbers.
- **Query surface:** substring scoring over labels → BFS. MCP server (`serve.py`): `query_graph`, `get_node`, `get_neighbors`, `shortest_path`, PR-impact tools. Real product is ~16 per-host skill bodies.
- **LLM dependency:** split-brain — code-only corpora skip the LLM (offline); docs/PDFs/images/transcripts require it. 9+ backends incl. local Ollama. Semantic half is nondeterministic.
- **Enterprise:** MIT; 83 test files + CI (ruff at syntax-error level only); genuine `security.py` threat model; "353 open" = 150 issues + 203 PRs (maintainer saturated). Bus factor 1 (81/100 recent commits by Safi); commercializing via Penpax (YC S26). Default branch `v8` = eight generational rewrites in ten weeks; consumers pin a branch, not an API.
- **Adoption:** Karpathy posted a wish on X 2026-04-01; repo shipped ~48h later; 22k stars in 10 days. Grew via X/LinkedIn/YouTube + Chinese AI-coding community. HN traction near zero (2 points).

---

## The landscape (live GitHub API, 2026-06-12)

Categories: **(a)** real static-analysis · **(b)** RAG/embedding retrieval · **(c)** LLM-summarization/wiki · **(d)** agent harness.

| Project | Stars | Category | Resolution | Accuracy measured | LLM for index | License |
|---|---|---|---|---|---|---|
| OpenHands/OpenHands | 76,593 | (d) harness | none | no | n/a | MIT |
| safishamsi/graphify | 66,129 | (c)+(b) | name-match | no | partial | MIT |
| cline/cline | 63,122 | (d) harness | none | no | n/a | Apache-2.0 |
| Egonex-AI/Understand-Anything | 57,993 | (c) | file-level | no | yes | MIT |
| **colbymchenry/codegraph** | 48,101 | **(a)** | real (no types) | partial (file-level %) | **no** | MIT |
| Aider-AI/aider | 46,055 | (d) (repo-map, PageRank) | none | no | n/a | Apache-2.0 |
| abhigyanpatwari/GitNexus | 42,016 | (a)+(b) | real (heuristic) | no | no (wiki only) | PolyForm NC |
| continuedev/continue | 33,664 | (d) | none | no | n/a | Apache-2.0 |
| yamadashy/repomix | 26,212 | (d) context packer | none | no | no | MIT |
| tree-sitter/tree-sitter | 25,806 | infrastructure | n/a | n/a | n/a | MIT |
| oraios/serena | 25,289 | **(a)** via LSP | real (borrowed) | no | no | MIT |
| AsyncFuncAI/deepwiki-open | 16,866 | (c) wiki | none | no | **yes** | MIT |
| ast-grep/ast-grep | 14,477 | structural search | none | no | no | MIT |
| zilliztech/claude-context | 11,828 | **(b)** | none (similarity) | retrieval only | embeddings req | MIT |
| sourcegraph (OSS snapshot) | 10,287 | **(a)** — **archived 2024-09-30** | real (SCIP) | no | no | mixed |
| github/codeql | 9,699 | **(a)** deep | real (dataflow) | no public rate | no | MIT (libs) |
| **DeusData/codebase-memory-mcp** | 3,378 | **(a)** | real **+ type inference** (10 langs) | **yes (arXiv)** | **no** | MIT |
| kythe/kythe | 2,130 | **(a)** deep (Google) | real | no | no | Apache-2.0 |
| facebookincubator/Glean | 1,355 | **(a)** deep (Meta) | real | no | no | BSD |
| github/stack-graphs | 876 | **(a)** deep — **archived 2025-09-09** | real | no | no | MIT/Apache |
| sourcegraph/scip-{java,ts,python} | 125/98/91 | **(a)** deep indexers | real | no | no | Apache-2.0 |

**The two genuine technical threats:**

- **codegraph (48k★)** — the architectural near-twin: tree-sitter → SQLite+FTS5 → MCP, deterministic, FS-watch incremental, 23 languages, LLM-free index. Publishes a file-level coverage % (weakest-possible accuracy metric, but the only high-star project publishing *any*). **No type system at all** — no generics, no alias expansion, no chain walking. Proof that the BearWisdom architecture wins stars when distributed; commercializing via hosted waitlist.
- **codebase-memory-mcp (3.4k★)** — the only project attempting BearWisdom's actual problem: 158 vendored grammars in one static C binary, owned-index type inference for 10 languages (generics, chaining, traits, LINQ), an arXiv eval (2603.27277), an openCypher query subset, "Linux kernel 28M LOC in 3 min." Single zero-dependency binary = the distribution story BearWisdom lacks, attached to real mechanism.
- **Serena (25k★)** — real inference by delegating to LSP servers (pyright/gopls/rust-analyzer). The architecture BearWisdom's "replace LSP" thesis competes against; adds symbolic editing (rename, replace-body) BearWisdom lacks.

**Where compiler-grade resolution actually lives:** built by FAANG, stagnant or dead, 0.9k–10k stars. Sourcegraph's OSS repo archived 2024 after going closed-source; its SCIP indexers sit at 30–125 stars. CodeQL is deep but a batch security-query engine (no incremental agent-facing index, no MCP). Kythe/Glean are corporate-internal exhaust. stack-graphs — the closest prior art to BearWisdom's profile-driven generic engine — was archived 2025 at 876 stars.

**Nobody at any star level publishes a per-language ref-level resolution-rate corpus benchmark.** codegraph's file-level table and codebase-memory-mcp's arXiv eval are the only public accuracy attempts.

---

## Strategy — asymmetric domination, not symmetric warfare

"Bypass on all fronts" is a trap. BearWisdom has already won the only front with a mechanism behind it (resolution, types, measured accuracy, zero-egress) and lost the only front it's fighting (activation: 1 star). Fighting symmetrically — building their dashboard, video-ingestion, LLM-graph — burns the asymmetric advantage chasing vanity metrics on their turf.

```
Front                  BW     Them   Action
────────────────────────────────────────────────────────────────
Mechanism (resolve)    ████   ░░░░   already won — PUBLISH it, don't rebuild
Proof / measurement    ████   ░░░░   weaponize: only public benchmark in the space
Enterprise / egress    ████   ░░░░   plant the flag they structurally can't reach
Distribution           ░░░░   ████   THE gap — close it, it's packaging not rewrite
Capability parity      ███░   ██░░   steal 2 (edit, schema), ignore the LLM-graph 5
Demo-ability / UI       █░░   ███░   polish existing D3 view; do NOT enter the arms race
```

### Front 1 — Distribution (the only true gap; highest ROI)

The Rust single-binary is a *better* activation primitive than anything they ship — graphify is Python + 36 grammar wheels + an 11.6k-line god file; UA needs a host LLM even to build the graph. The engine was just never packaged. Days, not a rewrite.

```bash
# A — one static binary, every platform: cargo-dist → GitHub Releases. Bundle/auto-fetch ONNX so
#     embeddings degrade gracefully instead of hard-failing. Kills "build-from-source only."
cargo binstall bearwisdom-mcp

# B — the channel they PROVED works: skill-install into 16+ agent hosts (the door codegraph grew through).
npx bearwisdom install   # writes the Claude Code / Codex / Cursor / Gemini-CLI skill + MCP config

# C — publish: crates.io (not even listed yet) + a thin npm/uvx shim that downloads the binary.
```

**B is the lever** — a skill wrapping the already-working MCP server is ~1 session and puts BearWisdom in the install flow that took codegraph 0→48k. Do **not** rewrite anything for distribution; the engine ships as-is behind a binary.

### Front 2 — Proof (turn the invisible moat into the headline)

```
A — publish the corpus as THE public benchmark: reproducible `bw quality-check` over N OSS repos →
    a resolution-rate leaderboard. First public ref-level benchmark in the space. Makes "unmeasured"
    their defining trait (it is).
B — head-to-head harness (marketing that's also honest): same repo, same agent task →
    measure {answer correctness, tool-call count, tokens, wall-clock, $cost} across BW vs graphify vs
    UA vs codegraph. Extend benchmarks/bw-bench (already does API-task eval). Their mechanism cannot
    survive this: skipped member-calls, 25% edge-drop, $/index.
C — coin the category split publicly: LLM-extracted edge @0.55 (a guess) ≠ resolved edge @1.0 (a fact).
    "They mix facts and 0.55 guesses in one namespace" is their own trackers' failure mode. Quote them.
```

Wedge sentence: *"graphify's headline 71.5× is a token-compression estimate where corpus size is `nodes × 50`. Ours is a resolution rate measured against ground truth over 257 real projects. One is a benchmark; one is arithmetic."*

### Front 3 — Steal 2 capabilities that fit the engine; refuse the other 5

```
✅ STEAL — symbolic editing (Serena's real differentiator). rename / replace-symbol-body / safe-delete
   THROUGH the resolved graph. Serena borrows resolution from LSP to do this; BW OWNS it → can do it
   better, offline, cross-language. ~2-3 sessions on find_references.
✅ STEAL — app-code ↔ SQL-schema edges (graphify's one legit multi-modal win for code). Deterministic,
   no LLM. Link query strings + ORM models → table/column nodes. "What breaks if I drop this column."
❌ REFUSE — LLM concept-graph, guided tours, persona dashboards, video/image/paper ingestion, Leiden
   clustering. Building these means becoming them and abandoning the moat.
⚠️ SELECTIVE — per-language barbell (matlab 15%, lua 40%, cpp 59% drag the 86.94% headline). Fix THROUGH
   the generic engine only, never quick wins. Lower priority than Fronts 1-2.
```

### Front 4 — Enterprise flag they cannot follow

```
A — lead every enterprise message with the triad they fail: ZERO-EGRESS · DETERMINISTIC · NO PER-INDEX COST.
    graphify's docs/images path egresses; UA mandates LLM (~157k tokens/scan). Caveat to fix first:
    godot_api.rs:226 curls extension_api.json — kill/vendor it so "zero-egress" is literally true.
B — the feature only determinism unlocks: CI gate. `bw ci` → fail PR on blast-radius breach / new dead
    code / resolution-rate regression / new cycle. An LLM graph CANNOT be a CI gate (nondeterministic).
    Architecturally barred from this, not just behind.
C — team-shared committed index (codebase-memory-mcp's move): zstd graph artifact in-repo, so a
    teammate/agent gets the graph with zero index time.
```

### Sequencing

```
Session 1-2   Front 1-B + 1-A   skill-install + prebuilt binary       ← decision: which hosts/channel first
Session 2-3   Front 2-A         publish corpus benchmark + leaderboard ← decision: which OSS repos, where
Session 3-4   Front 2-B         head-to-head harness (extend bw-bench)   mostly execution
Session 4     Front 4-A/B       zero-egress flag + `bw ci` gate          mostly execution; 4-A is a 1-line fix
Session 5-7   Front 3 ✅        symbolic edit + schema↔code edges        execution; review on edit semantics
ongoing       Front 3 ⚠️        per-language draggers via generic engine
cheap/now     docs drift        README 31→95 langs, MCP 18→20, tests 3777→7150 — undersells by half
```

**Highest-leverage single hour: Front 1-B** — a skill manifest wrapping the existing MCP server, in the install flow that made three competitors famous. Everything else compounds off being installable.

---

## Features readily available from existing data

Every competitor answers codebase questions by *guessing*. BearWisdom's edges are *facts* at confidence 1.0. That makes a class of questions — "what is definitely true about this codebase right now" — computable from data already in the index and structurally impossible for them to answer correctly. Most are graph algorithms and index-diffs over existing edges, not new engine work.

Markers: ✅ data + tool already exist · ⚠️ data exists, needs glue · ❌ needs new extraction.

| Feature | Computed from (existing) | Ready | They can't copy because | Online appeal |
|---|---|---|---|---|
| Breaking-change / semver diff | visibility + signatures + return_type | ⚠️ | no signatures, no stable IDs | ★★★★★ (lib authors) |
| GitHub Action / CI gate | blast_radius + quality_check + dead_code | ⚠️ | nondeterministic → can't gate | ★★★★★ (discovery channel) |
| Architecture cycle / layering | resolved import edges | ⚠️ | edges are guesses, cycles lie | ★★★★☆ (screenshot-viral) |
| Resolvability badge | quality_check per-file rate | ⚠️ | unmeasured by construction | ★★★★☆ (badges self-spread) |
| Endpoint→SQL reach (injection) | flow route nodes + SQL nodes + edges | ⚠️ | no resolved path, no flow | ★★★★☆ (security framing) |
| Dead-code (zero-FP) | dead_code + entry_points + edges | ✅ | name-match → false positives | ★★★☆☆ |
| Token-measured agent context | smart_context + compact format | ✅ | BFS over prose, not symbols | ★★★☆☆ (benchmark fuel) |
| Cross-service graph | HTTP call URLs + route emissions | ⚠️ | no resolved call sites | ★★★★☆ (microservices) |
| Schema↔code blast radius | ORM models → table/column | ❌ | — | ★★★★☆ |

### Top 3 — free or near-free, copy-proof

```rust
// 1 — BREAKING-CHANGE ADVISOR  ⚠️ (index-diff; zero new extraction)
//     Data present today: visibility(pub/priv) + signature + return_type + generic_params + qname identity.
//     Mechanism: diff index@base vs index@head over PUBLIC symbols.
//        removed pub symbol            → MAJOR
//        signature/return_type changed → MAJOR
//        new pub symbol                → MINOR
//   ✅ pure data diff. graphify/UA have no signatures and no stable IDs → cannot compute this at all.
//   Ships as a GitHub Action + PR comment; library authors adopt semver bots virally.
//   ONE real risk: symbol identity across commits (qname is the key — mostly stable, verify on renames).

// 2 — CI GATE  ⚠️ (orchestration over 3 existing tools)
//     `bw ci --base origin/main` → fail PR on:
//        resolution-rate regression (quality_check delta) · new dead code (dead_code delta)
//        new import cycle (Tarjan over edges, ~30 lines) · blast-radius breach (blast_radius on diff)
//   ✅ DETERMINISM is the moat: an LLM-built graph CANNOT gate a build — nondeterministic runs flap.
//   The Marketplace Action IS the distribution channel (Front 1) AND the feature. Two birds.

// 3 — ARCHITECTURE CYCLE / LAYERING VIOLATIONS  ⚠️ (graph algo over existing import edges)
//     Tarjan SCC over module-level import edges → cycles. Declared layers + edges → "ui/ imports db/".
//   ✅ on edges at conf 1.0 a cycle is a FACT; on name-match edges a "cycle" is noise.
//   Point the existing D3 web view at this query — the screenshot that made them viral, except true.
```

### Two more

```
4 — RESOLVABILITY BADGE  ⚠️  quality_check already computes per-file rate.
    README badge [code resolvable: 94%] → links to the unresolved-ref report. Badges self-propagate
    (see coverage badges). Nobody else can mint this number because nobody else measures resolution.

5 — ENDPOINT→SQL REACHABILITY  ⚠️  flow detectors already emit route nodes (Axum/Actix) and SQL nodes (SQLx).
    Path query route→…→raw-SQL over resolved edges = injection surface, deterministic, CodeQL-lite.
    HONEST CAVEAT: reachability, not proven taint. Frame as "endpoints that reach raw SQL," not
    "exploitable." VERIFY FIRST that both ends land as path-connected graph nodes (unconfirmed).
```

### Skip

```
❌ Schema↔code blast radius   — needs ORM-model→table extraction. High value, NOT readily available. Later.
❌ Change-coupling / ownership — needs git-history mining, outside the resolved graph. Not the moat.
❌ Video/image/concept graph  — that's becoming them. Refuse.
```

### The pick

**`bw ci` as a GitHub Action (#2 wrapping #1+#3).** The rare move where feature and distribution channel are the same artifact: discoverable in the Marketplace, posts PR comments (organic reach), and does the one thing an LLM-graph can never do — gate a build deterministically. The breaking-change diff (#1) is the sleeper inside it: library authors are a viral, vocal adopter class, and "semver-correct API diff" is a clean, copy-proof claim. All three top items are index-diffs and graph algorithms over edges already stored at confidence 1.0 — no new extractor, no engine change. The only genuinely new code is a stable-symbol-identity check across commits (the #1 risk) and Tarjan SCC (#3, trivial).

---

## Open assumptions to verify before building

1. **Symbol identity stability** across an index re-run / across commits (gates feature #1 — breaking-change diff).
2. **Flow node connectivity** — whether route nodes and SQL nodes are actually path-connected in the edge table (gates feature #5 — endpoint→SQL).

Both are read-only investigations over the existing index.

---

*Sources: GitHub MCP + live API (star counts, file contents, issue trackers) 2026-06-12; competitor source files cited inline per section; BearWisdom figures from `CORPUS-2026-06-11.md`, `Cargo.toml`, `crates/bearwisdom-mcp/src/server.rs`, and the project memory store. Full per-claim citations retained in the originating research agents' reports.*
