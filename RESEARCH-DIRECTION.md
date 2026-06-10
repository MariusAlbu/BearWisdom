# BearWisdom Research Direction

This document tracks the current product/research direction for BearWisdom. Keep it updated as the direction evolves.

## Goal

BearWisdom started as a structural code intelligence engine intended to serve two related use cases:

- an MCP server for frontier models
- an IDE-grade replacement for finding references and navigating codebases

The IDE use case is now secondary. The main goal is to make BearWisdom best-in-class for LLM-driven codebase research: faster, more accurate, and more token-efficient than normal agentic exploration with grep/glob/read tools.

The target is for BearWisdom to become the single codebase search/research tool exposed to the LLM harness.

## Current Problem

BearWisdom has many strong structural features around symbols, references, edges, resolution, flow, concepts, and hybrid search. In practice, models still favor `BW Grep` heavily because grep is familiar, literal, and easy to trust.

That is a product signal. To beat grep, BearWisdom must return better evidence per token, not just expose more commands.

The model should not need to decide whether to call grep, symbol search, content search, hybrid search, references, blast radius, flow, concepts, or file search. BearWisdom should decide internally.

## North Star

BearWisdom should answer:

> What should I inspect next, and why?

better than a frontier model with grep/glob/read tools can.

Success means:

- fewer tool calls
- lower returned-token volume
- faster wall-clock completion
- higher evidence recall
- more precise file/line/symbol citations
- less need for fallback grep
- better LLM answers on real codebase research tasks

## Target LLM-Facing Tool

Expose one primary MCP/harness tool, tentatively:

```json
bw_research({
  "question": "Where is authentication enforced for API requests?",
  "scope": "apps/server",
  "budget": {
    "max_tokens": 6000,
    "max_results": 12
  },
  "mode": "auto"
})
```

The tool should return a compact evidence packet:

```json
{
  "summary": "Authentication is enforced in ...",
  "evidence": [
    {
      "file": "src/middleware/auth.ts",
      "lines": "42-88",
      "symbol": "requireAuth",
      "why": "Central middleware checks JWT and rejects unauthenticated requests",
      "snippet": "..."
    }
  ],
  "graph": [
    "routes/user.ts -> requireAuth -> verifyJwt -> UserRepository"
  ],
  "confidence": "high",
  "next_queries": []
}
```

The public contract should stay small. Internally, BearWisdom can route across many retrieval systems.

## Retrieval Router

`bw_research` should include an internal router that chooses and fuses retrieval strategies:

- exact grep/string search
- FTS5 content search
- symbol search
- fuzzy file/symbol search
- definition lookup
- reference lookup
- call hierarchy
- import/dependency graph
- cross-language flow edges
- semantic vector search
- unresolved/external reference analysis
- concept clusters
- blast radius / impact analysis

The LLM asks the research question. BearWisdom chooses the retrieval plan.

## Evidence Packets

BearWisdom should return code evidence, but in a compressed and ranked form.

Each result should include:

- file path
- exact line span
- symbol name and kind when available
- short code snippet
- why the result matters
- match source, such as exact, FTS, symbol, reference, vector, call graph, or flow
- confidence
- expansion handle or follow-up query when more context is needed

Avoid returning whole files by default. Return enough context for the LLM to reason, then let it request expansion deliberately.

## Token Compression

Token efficiency is a core product feature, not a side effect.

Prefer:

- signatures before bodies
- symbol skeletons before full source
- call-chain summaries
- import/export summaries
- route/controller/service/DB flow summaries
- deduplicated snippets
- relevant neighboring lines only
- test/generated/vendor/docs filtering by default

Target: common codebase research tasks should require 5-10x fewer returned tokens than grep/glob/read or explore-agent workflows.

## Semantic Search Direction

BearWisdom already has the right semantic-search shape:

- AST-aware `code_chunks`
- CodeRankEmbed embeddings
- `sqlite-vec` vector KNN
- hybrid FTS/vector ranking via reciprocal rank fusion

Immediate semantic reliability work:

- make `bw embed` reliable
- show vector/chunk counts in `bw status`
- make hybrid search report when it is actually vector-backed vs FTS-only
- ensure CodeRankEmbed's query instruction prefix is applied correctly
- benchmark semantic results against exact/FTS/grep routes

Current default embedding recommendation: keep `nomic-ai/CodeRankEmbed`. It is MIT licensed, code-specialized, 137M parameters, supports 8192 context, and reports strong code-retrieval scores on CSN and CoIR.

## TurboVec Direction

TurboVec should be treated as an optional vector-index backend, not the core product move.

Conceptual split:

- CodeRankEmbed creates meaning vectors for code chunks and queries.
- TurboVec stores/searches those vectors compactly and quickly.
- BearWisdom maps vector hits back to symbols, files, line spans, references, calls, and flow edges.

TurboVec becomes useful if:

- vector storage becomes too large
- sqlite-vec KNN latency becomes a bottleneck
- BearWisdom starts indexing millions of chunks across many repos
- the harness needs lower memory use for always-on local semantic search

The competitive advantage is not TurboVec alone. It is semantic code retrieval plus BearWisdom's structural graph plus token-aware evidence packaging.

## Comparison To LightRAG-Style Systems

LightRAG is a full general-purpose RAG framework. It ingests documents, extracts entities and relationships with an LLM, builds a knowledge graph plus vector index, retrieves context, and supports answer generation workflows.

BearWisdom + CodeRankEmbed + TurboVec is different. It is a code-native research engine:

- LightRAG's graph is LLM-extracted from documents.
- BearWisdom's graph is structurally extracted from code: symbols, definitions, references, calls, imports, flow edges, unresolved refs, packages, and language-specific semantics.

For general document RAG, LightRAG is more complete. For codebase research by LLMs, BearWisdom should be stronger because its graph is based on code structure rather than inferred document entities.

## Evaluation Harness

Build the evaluation harness before adding large new retrieval features.

Create 30-50 real codebase research tasks, such as:

- Where is authentication enforced?
- What writes to this database table?
- What happens when this endpoint is called?
- Which symbols break if this function changes?
- Find the implementation despite vague user terminology.
- Explain this subsystem with citations.
- Locate the likely bug source from this error text.
- Find all references, excluding tests/generated/docs.
- Trace route -> handler -> service -> DB.
- Identify why a reference is unresolved.

Measure BearWisdom against grep/glob/read and explore-agent baselines:

- answer accuracy
- evidence recall
- precision of citations
- returned tokens
- number of tool calls
- wall-clock time
- fallback grep rate
- LLM answer quality

This benchmark should drive product decisions.

## Recommended Roadmap

1. Build the research-task evaluation harness.
2. Design and implement the single `bw_research` evidence-packet tool.
3. Add the internal retrieval router and fusion layer.
4. Make semantic search reliable and visible in status/diagnostics.
5. Add token-aware context packing.
6. Add reranking over broad first-stage retrieval candidates.
7. Demote grep to an internal fallback rather than the main LLM-facing affordance.
8. Benchmark against grep/glob/read and explore-agent workflows.
9. Evaluate TurboVec only after semantic search is proven useful and vector scale becomes a measurable bottleneck.

## Execution Plan

This plan turns the six near-term workstreams into concrete implementation steps. The order matters: build measurement first, ship a thin useful `bw_research`, then make the internals smarter until the benchmark proves BearWisdom beats grep/explore workflows.

### 1. Evaluation Harness

Goal: create a repeatable scoreboard for LLM codebase research quality, speed, and token efficiency.

Steps:

- Define a benchmark task schema in JSON or YAML.
- Seed 30-50 tasks across BearWisdom and 2-3 external projects.
- Store expected evidence as files, symbols, line ranges, required concepts, and excluded paths.
- Implement deterministic scoring for file/symbol/line overlap, forbidden context, tool calls, elapsed time, and returned tokens.
- Add optional judge-model scoring for answer correctness and evidence support.
- Record each run as machine-readable JSON plus a human-readable markdown report.
- Add a CLI command such as `bw eval run <suite> --system bw_research|grep_agent|explore_agent`.

Task schema sketch:

```json
{
  "id": "auth-enforcement-001",
  "question": "Where is API authentication enforced?",
  "project": "sample-api",
  "expected_files": ["src/middleware/auth.ts"],
  "expected_symbols": ["requireAuth", "verifyJwt"],
  "must_mention": ["JWT", "middleware"],
  "exclude_paths": ["tests/", "docs/", "generated/"],
  "difficulty": "medium"
}
```

Acceptance criteria:

- A single command runs the suite and emits scores.
- At least 30 tasks exist before optimizing retrieval.
- The harness reports tokens, time, tool calls, evidence recall, and precision.
- Results are stable enough to compare branches.

### 2. LLM-Native `bw_research` MCP Tool

Goal: give the LLM one high-level codebase research tool instead of a menu of low-level search commands.

Steps:

- Define the public input contract with `question`, `scope`, `mode`, `filters`, `budget`, and optional `cursor`.
- Define modes: `auto`, `exact`, `references`, `impact`, `flow`, `architecture`, and `semantic`.
- Implement a thin first version that calls existing BearWisdom primitives and returns evidence packets.
- Add the tool to the MCP server and to the harness as the default code-search interface.
- Keep low-level tools available for developers, but stop presenting them as the primary LLM surface.
- Add continuation support for expanding a result, following callers/callees, or requesting more snippets.

Input sketch:

```json
{
  "question": "What writes to the users table?",
  "scope": "apps/server",
  "mode": "auto",
  "filters": {
    "tests": "exclude",
    "docs": "auto",
    "generated": "exclude",
    "vendor": "exclude"
  },
  "budget": {
    "max_tokens": 6000,
    "max_results": 12
  }
}
```

Acceptance criteria:

- Frontier models can solve benchmark tasks using only `bw_research` plus final-answer text.
- The tool returns cited evidence with file/line/symbol references.
- The tool respects token and result budgets.
- The first implementation is useful even before the router is sophisticated.

### 3. Retrieval Router

Goal: make BearWisdom choose the right internal retrieval plan from the user's natural-language research question.

Steps:

- Add query-shape classification with deterministic heuristics first.
- Route exact strings, error text, config keys, and quoted text to grep/FTS.
- Route symbol-looking queries to definition, references, symbol search, and callers.
- Route "what happens when" and endpoint questions to route/flow/call graph expansion.
- Route vague natural-language questions to semantic search, concepts, FTS, and symbol search.
- Route "what breaks" questions to blast radius, inbound references, tests, and dependency graph.
- Merge candidates from all selected strategies into one scored candidate set.
- Add feature-based ranking: exact hit, symbol hit, semantic score, graph distance, path relevance, language relevance, generated/test/doc penalty, and relation strength.
- Log the chosen retrieval plan in debug output and optionally in the evidence packet.

Acceptance criteria:

- `bw_research` no longer behaves like a single search command.
- Retrieval plans are explainable and visible during debugging.
- Benchmark tasks show improved evidence recall over FTS/grep-only.
- Grep remains available internally but is not the dominant path for every query.

### 4. Evidence Packet Format

Goal: return compact, trustworthy, citation-ready context that helps the LLM answer without reading whole files.

Steps:

- Define stable response types for summary, evidence items, relations, omissions, confidence, and continuations.
- Include `why` on every evidence item.
- Include `match_sources` so the LLM can distinguish exact, FTS, symbol, reference, vector, call graph, and flow hits.
- Include line spans and symbols whenever available.
- Add relation summaries for calls, references, imports, route-flow, DB-flow, and dependency links.
- Add omission counts for tests/docs/generated/vendor/low-confidence results.
- Add cursors for result expansion instead of dumping more context up front.

Response sketch:

```json
{
  "answerable": true,
  "summary": "The request is authenticated in middleware before route handlers run.",
  "confidence": "high",
  "evidence": [
    {
      "id": "e1",
      "file": "src/auth/middleware.ts",
      "line_start": 42,
      "line_end": 88,
      "symbol": "requireAuth",
      "kind": "function",
      "reason": "Rejects requests without a valid JWT before control reaches handlers.",
      "match_sources": ["symbol", "call_graph", "semantic"],
      "snippet": "..."
    }
  ],
  "relations": [
    {
      "from": "registerRoutes",
      "to": "requireAuth",
      "type": "calls"
    }
  ],
  "omitted": {
    "tests": 12,
    "generated": 4,
    "low_confidence": 19
  },
  "next_actions": [
    {
      "label": "expand requireAuth callers",
      "cursor": "..."
    }
  ]
}
```

Acceptance criteria:

- Packets are self-contained enough for the LLM to answer common tasks.
- Packets are smaller than equivalent grep/read exploration.
- Every answerable packet contains precise citations.
- Continuations make deeper exploration possible without front-loading tokens.

### 5. Semantic Path Reliability

Goal: make semantic search trustworthy, observable, and easy to set up.

Steps:

- Add `code_chunks`, `vec_chunks`, embedding model name, embedding dimension, and vector backend status to `bw status`.
- Make `bw hybrid` explicitly report whether vector results participated or whether it fell back to FTS-only.
- Make `bw embed` surface missing model, missing `ORT_DYLIB_PATH`, dimension mismatch, and zero-vector cases clearly.
- Add `bw models list`, `bw models install coderankembed`, and `bw models status` or equivalent setup commands.
- Verify CodeRankEmbed query/document formatting, including any required query prefix.
- Add a semantic regression suite: vague query -> expected file/symbol/chunk.
- Make vector dimensions explicit in schema/config so future models are possible.

Model packaging direction:

- Do not bundle a model in the core release.
- Ship model support and a frictionless install path.
- Keep CodeRankEmbed as the recommended default local model.
- Allow semantic search to degrade cleanly when no model is installed.

Acceptance criteria:

- Users can tell immediately whether semantic search is active.
- Hybrid search cannot silently pretend to be vector-backed.
- `bw embed` is one-command reliable after model setup.
- Semantic benchmark cases improve over FTS-only for vague research questions.

### 6. Token/Time Benchmark Against Grep And Explore Agents

Goal: prove the incentive for using BearWisdom instead of normal agentic grep/glob/read exploration.

Steps:

- Implement baselines for raw grep/glob/read, explore-agent workflow, low-level BearWisdom tools, and `bw_research`.
- Run each benchmark task through each system with the same subject model where possible.
- Capture elapsed time, tool calls, returned tokens, final answer tokens, fallback rate, evidence recall, precision, and judge score.
- Emit a comparison report with per-task and aggregate summaries.
- Track regressions over time.
- Use the report to prioritize retrieval/router/evidence improvements.

Metrics sketch:

```text
task_id
system
success
evidence_recall
evidence_precision
returned_tokens
final_answer_tokens
tool_calls
elapsed_ms
fallback_grep_used
judge_score
```

Target claim:

> BearWisdom answers codebase research tasks with equal or better evidence, 5x fewer returned tokens, and 2x faster wall-clock time than grep/read exploration.

Acceptance criteria:

- The benchmark can compare `bw_research` against grep/explore baselines.
- Reports make token/time wins or losses obvious.
- The suite becomes the gate for future retrieval changes.

## Sequencing

Phase 0: Baseline

- Add the eval task schema.
- Seed the first 10-15 tasks.
- Implement deterministic scoring and JSON/markdown reports.
- Measure current grep/explore and low-level BearWisdom behavior.

Phase 1: Thin `bw_research`

- Implement the MCP/harness tool contract.
- Return evidence packets using existing search/reference/graph primitives.
- Add token budgets and path filters.
- Run the eval suite and identify failure classes.

Phase 2: Router And Fusion

- Add deterministic query-shape routing.
- Add multi-strategy candidate fusion.
- Add ranked evidence item generation with `why` and match sources.
- Benchmark against Phase 1.

Phase 3: Semantic Reliability

- Fix status/diagnostics for chunks/vectors/model state.
- Make embedding setup explicit.
- Add semantic regression tasks.
- Ensure hybrid reports vector participation.

Phase 4: Compression And Continuations

- Add skeleton/signature/context packing.
- Add result expansion cursors.
- Add omission accounting.
- Tune for 5-10x fewer returned tokens.

Phase 5: Proof And Optional Vector Backend Work

- Run full benchmark against grep/explore agents.
- Publish/track aggregate results.
- Evaluate TurboVec only if vector storage/search is now a measured bottleneck.

## Open Questions

- What should the first 30 benchmark tasks be?
- Should `bw_research` synthesize a summary itself, or only return evidence for the frontier model to summarize?
- What should be the default token budget for harness use?
- Which filters should be on by default: tests, docs, generated, vendor, snippets?
- How should continuation handles work for deeper exploration?
- Should BearWisdom include an optional reranker model, or keep reranking heuristic-first?
- How much of the existing command surface should remain visible to the harness?
