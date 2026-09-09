# BearWisdom engine and product review — 5 September 2026

## Verdict

BearWisdom contains substantial, useful compiler-inspired engineering. It is far beyond a parser that connects matching names: there are declaration-bound types, generic substitution, member walking, external dependency discovery, lazy materialization, flow extraction, language profiles, framework connectors, and multiple query surfaces.

The production system does not yet have compiler-grade semantic correctness. Several foundations are incomplete or bypassed: lexical binding identity, module identity, overload selection, control-flow typing, and semantic invalidation. These are not obscure language corner cases. Small valid programs reproduce wrong targets, merged declarations, and stale results. Those failures matter more than the unresolved percentage because they are presented as successful resolution.

My assessment is: a serious code-intelligence prototype with valuable infrastructure, but not yet a trustworthy replacement for language servers. It has a plausible advantage over Graphify's inspected resolver, no demonstrated categorical advantage over current GitNexus, and a substantial correctness gap relative to mature language-specific semantic engines. A public, independent evaluation could establish a much stronger position than additional feature breadth.

## Scope and evidence

The review began with a clean worktree at `32da1ea3`, on `feat/resolution-engine`. I inspected the production full/incremental indexing paths, resolution engine, type representation, flow interface, identity/persistence, navigation queries, AI context selection, benchmark generation/scoring, baseline and residue documents, and current primary competitor sources.

Concurrent changes appeared in flow extraction/configuration and tests while the review was underway. They were preserved. The test results below describe the code as executed during this review, not a guarantee about subsequent edits. The primary engine/query files behind the reproduced findings had not changed when their source locations were checked. The review did not modify production code or rebuild the project's own index.

Verification:

- `cargo test -p bearwisdom --lib indexer::resolve::engine:: -- --test-threads=4`: **686 passed**.
- Existing `resolution_corpus`, `resolution_corpus_js`, and `resolution_corpus_dart` integration tests: **passed**.
- Existing `resolution_gate`: **4 passed**.
- Existing `resolution_corpus_rust`: **failed**, with **30/32 asserted patterns passing**. Failures: nested self-crate import member resolution and the constructor portion of a renamed re-export member chase. It also prints three known-red candidate probes that are not part of those 32 assertions.
- Six new adversarial integration probes: **all six failed their intended-behavior assertions**. These were deliberately chosen to challenge suspected defects; this is not an estimate of overall error frequency.
- The library build reported **348 warnings**, including unused code/imports. Warnings alone do not establish poor runtime behavior, but substantial inactive machinery makes architecture descriptions unreliable without tracing actual callers.

[probes.rs](probes.rs) preserves the exact six diagnostic tests outside Cargo's automatic test discovery. To reproduce, copy it to `tests/tests/audit_engine_probe.rs`, run the command in its header, then remove the temporary copy. The probes use temporary source projects and in-memory databases; ordinary external discovery may still consult installed toolchains. No paid model benchmark or full 262-project recapture was run.

## 1. What the ~70% number means

Summing the checked-in `baseline.json` gives:

| Measurement | Value |
|---|---:|
| Projects | 262 |
| Internal resolved edges | 9,282,707 |
| Counted unresolved rows | 3,982,446 |
| Current formula | 69.9781% |
| Additional resolutions needed for 99%, holding this denominator fixed | approximately 3,849,795 |
| Fraction of present residue that must disappear | 96.67% |

The September 4 database census reports **3,988,062** residue rows, 5,616 more than the JSON baseline. Its sampling/schema exclusions and the baseline need a single reproducible provenance record. Treat its cause distribution as historical diagnostic evidence, not an exact current decomposition of this checkout.

The formula in `query/stats.rs:346` counts graph edges divided by graph edges plus filtered unresolved rows. Calling this `precision` (`stats.rs:235,280`) is mathematically incorrect. Precision requires knowing which reported targets are correct. These tables do not supply independent truth.

There are further measurement problems:

1. **Extraction blind spot.** A reference that is never extracted is absent from both sides. The metric cannot measure total reference recall.
2. **Different counting units.** `edges` has `UNIQUE(source_id,target_id,kind,source_line)`; two calls to the same target on one line collapse. Unresolved rows are not constrained in the same way. The new `ref_resolutions` table already preserves separate sites, but the headline rate still counts edges.
3. **Asymmetric exclusions.** The unresolved side excludes snippets, drained refs, and certain Markdown import refs; the edge query does not apply corresponding site-level exclusions. Generated-file exclusions are symmetric, which is good.
4. **Added edge kinds.** The numerator has no restriction to the same reference-site universe: separately synthesized relationships or imported `scip_ref` edges can affect it.
5. **No calibrated certainty.** `contract/types.rs:86` fixes resolved confidence at `1.0`. The low-confidence report cannot detect heuristic decisions made by this engine. A strategy name is useful provenance, but is not a correctness proof.
6. **Errors can resemble empty data.** Several count queries use `unwrap_or(0)`. A failed measurement must be an unavailable/error result, not an apparently clean count.

Use four separate metrics: extraction recall; correct binding precision; correct binding recall; and resolution coverage on an explicitly fixed site set. Also report ambiguity, unsupported dynamic behavior, unavailable dependencies, parse failures, and excluded/generated code. Show results by language, construct, project, and dependency state, including both project-macro averages and site-micro averages.

An appropriate 99% claim would be: “99% correct reference binding recall, with independently measured precision, for this supported language/construct/configuration matrix and these supplied dependencies.” An unconditional 99% promise for arbitrary multilingual code and runtime-generated behavior is not credible.

## 2. Reproduced foundational failures

### A. Local variable types leak between functions — critical

```typescript
class Alpha { save(): void {} }
class Beta  { save(): void {} }
function first(value: Alpha)  { value.save(); }
function second(value: Beta)  { value.save(); }
```

Observed: **both calls resolve to `Beta.save`**. The first should resolve to `Alpha.save`.

`engine/file_lookup.rs:25,43` implements a flat per-file map from textual name to type. `pipeline.rs:295` seeds every declared binding before visiting references, and later writes overwrite earlier ones. Sorting seeds makes this deterministic; it does not make it semantically correct. The chain root reads these caches before consulting the declaration attached to the occurrence.

Fix: assign each lexical binding an identity; resolve name occurrences to that identity before type inference. Store facts by binding plus flow location, not by spelling. Parameters, nested blocks, closures, shadowed locals, and reassignments require explicit scope/lifetime handling. Add noninterference tests: changing an unrelated sibling function must not change existing bindings.

### B. Extracted control-flow facts are not consumed — critical

```typescript
class Alpha { onlyAlpha(): void {} }
class Beta { onlyBeta(): void {} }
function run(value: Alpha | Beta) {
  if (value instanceof Alpha) { value.onlyAlpha(); }
}
```

Observed: **no call edge to `Alpha.onlyAlpha`**.

`indexer/flow.rs` builds CFG and narrowing metadata, but production `FileLookup::set_cursor` and `install_local_cache` are explicit no-ops (`file_lookup.rs:364,367`). The production ref loop does not install and advance that analysis. Having a CFG module and tests is not equivalent to using flow facts in semantic queries.

Fix: connect a scope-correct flow environment to expression typing. Handle joins, loops, early returns, assignment kills, and discriminant guards. A minimal restoration of byte-range guards can improve coverage, but cannot replace correct flow joins and lexical identity.

### C. Unrelated TypeScript modules merge declarations — critical

```typescript
// a.ts
export interface Config { onlyA(): void; }
// b.ts
export interface Config { onlyB(): void; }
```

Observed: the database has **one `Config` symbol**, owned by `a.ts`. These are two separate module exports.

`symbol_key.rs:32,56` marks TypeScript interfaces mergeable and omits file identity for mergeable declarations. Their extractor qnames are both `Config`; the key contains no defining module/package identity. Legitimate declaration merging is only legitimate within the appropriate semantic container.

Fix: separate declaration identity from display names. Include language, package instance/version, module/assembly/crate instance, lexical container, and overload identity as appropriate. Merge declarations only after binding their semantic containers. Preserve multiple declaration locations for real merged symbols. The same design must distinguish two installed versions of a dependency and identical namespace names in unrelated projects/languages.

### D. Overloads can select the wrong return type — critical

```typescript
class Alpha { save(): void {} }
class Beta  { save(): void {} }
declare function make(x: string): Alpha;
declare function make(x: number): Beta;
function run() { make(123).save(); }
```

Observed: **`Alpha.save`**, when the call selects the number overload returning `Beta`.

Member lookup commonly returns the first compatible member. `overload_alts.rs` retries up to four alternative returns if the next member is missing. That is downstream member-shape evidence, not overload applicability. If both returns have the member, the wrong initial selection survives. Argument-driven generic substitution occurs after a callee has been chosen; it does not supply full overload selection.

Fix: collect the scoped candidate set, check arguments/arity/defaults/variadics, infer generic constraints, apply language-specific conversion and specificity rules, then compute the selected signature's return. Represent unresolved ambiguity explicitly. Do not promote “this return type contains the requested next member” into an exact declaration binding.

### E. Incremental indexing keeps stale semantic answers — critical for AlphaVision

The probe initially defines `make(): Alpha` in `api.ts`; an unchanged consumer calls `make().save()`. It then changes the API to `make(): Beta` and runs incremental indexing.

Observed:

```text
before:      consumer.run -> Alpha.save
incremental: consumer.run -> Alpha.save
fresh build: consumer.run -> Beta.save
files_reresolved: 0
```

`symbol_key.rs` omits return types, field types, bases, and visibility. That is not inherently wrong for stable identity. The mistake is using identity survival as evidence that dependent semantics did not change: `incremental.rs:356` explicitly invalidates vanished keys and newly satisfiable names, while surviving keys trigger neither.

Fix: retain stable `SymbolId`, but separately track semantic interface fingerprints and query dependencies. A return-type change must invalidate consumers of that return; adding a better overload must invalidate overload lookup even when the previous target remains; changing a re-export, configuration, or negative name lookup must invalidate dependent queries.

A further source-level concern: full resolution persists type metadata and the arena (`pipeline.rs:156`), while the inspected incremental path ends after flushing reference output. It does not call `persist_type_info`. Successive edits and process restarts need dedicated parity tests for that state, beyond the reproduced invalidation bug.

### F. Find All References loses occurrences and its cache loses results — high

```python
def helper():
    return 1
def caller():
    helper(); helper()
    helper()
    helper()
```

Observed:

```text
ref_resolutions:                       4 call sites
find_references, uncached/unlimited:   3 results
find_references, cached limit=1:       1 result
subsequent cached unlimited query:     1 result
```

There are two independent defects. The edge uniqueness key collapses same-line occurrences. The reference cache is keyed only by target name, although the cached result was already truncated by `limit` (`query/references.rs:32,111,117`).

Fix: implement references over occurrence identity and full source ranges. Cache an untruncated canonical result and slice it, or include every result-changing argument and snapshot version in the key. Apply the same audit to other query caches.

## 3. Why the unresolved residue is large

The September 4 census provides a useful direction, with important qualifications:

| Historical bucket | Approximate volume | Interpretation |
|---|---:|---|
| Bare-root family as originally recorded | 2.324M | Includes substantial misclassified receiver failures |
| Import unlinked / declared dependency unsupplied | 587K | Module binding or dependency supply failures |
| Member-walk failures | 756K | Missing member surface, incomplete typing, silent declines, alias gaps |
| Captured symbol but missing binding/field/return type | 277K | Missing extraction facts or inference |
| Enclosing member dispatch failures | 43K | Ownership, inheritance, or implicit receiver problems |

These are the old labels, not independent causal estimates. The census estimates approximately 663K originally root-classified rows were actually receiver-chain deaths. That estimate comes from sampling; it is not an exact number of failures fixed by any one change.

Current HEAD already improves root-cause recording, preserves a chain cause through failed bare-ladder fallback, and connects external-name classification. Do not count these as unresolved implementation tasks or predict a rate gain from relabeling.

The file-scope namespace guard remains problematic: `semantic_model.rs:214` asks whether any same-named module/namespace exists anywhere in the lookup. That can reroute a failed value chain through bare-name lookup. Bind the root in the current lexical/import environment and carry its category; a global name search is not a namespace classification.

The largest plausible levers are shared mechanisms:

- Complete local/parameter/field type extraction and contextual lambda typing, through scope-correct bindings. `lambda_seed.rs` already implements contextual typing for supported call shapes; expanding extractor coverage and wiring is required, not inventing it from scratch.
- Correct module identity, aliases, re-exports, package exports, ambient modules, and installed dependency linking.
- Rich, demand-reachable external declarations, with their transitive type/member surfaces.
- Return inference and generic constraints across dependencies, rather than only fixed-order initializer special cases.
- Correct ownership of implementation bodies and implicit receivers.

The existing roadmap documents concrete language supply problems: Java/Kotlin missing source jars, F# not receiving the .NET surface available to C#, Zig/Swift/R toolchain gaps, Dart path/workspace packages, and pnpm layout issues. It also documents reachability failures where data is already supplied: Node `node:` normalization, Dart wildcard re-exports, PHP namespace imports, and C include closure. These are different failure classes and need different fixes.

Making all bare names globally visible is not a fix. It increases accidental matches. Nor does excluding builtins satisfy the product requirement of navigating to library definitions; dependency supply should expose real declarations where available.

No present evidence justifies promising that the current M2–M6 list reaches 90%, much less 99%. Root-cause grouping should record dependent obligations so a fix's actual downstream gain can be measured, including retargeted wrong answers and newly discovered reference sites.

## 4. How close is the architecture to a compiler?

The good parts are worth preserving: shared extraction contracts, a structured `TypeArena`, declaration-bound `Type::Decl`, separate language profiles, per-file parse artifacts, lazy externals, named binder rules, reference outcomes, framework relationships, and an embedded database/API.

**User-confirmed architectural requirement (September 5):** follow the Roslyn-inspired direction of removing strings and string comparisons from semantic resolution. The existing ID migration is intentional and must be completed consistently. Fixes proposed in this review must preserve this direction. Strings belong at source ingestion, external-format/persistence boundaries, diagnostics, and display; semantic lookup, binding, substitution, dispatch, and dependency tracking should operate on typed IDs and structural data.

Intern source names once into `NameId`; bind `(ScopeId, NameId)` through explicit visibility/import relationships to `SymbolId` or an explicit candidate set. Track locals with `BindingId`, semantic types with `TypeId`, and modules/packages/files with their own identity types. A name ID identifies a spelling, not a declaration: two unrelated `Config` declarations can share `NameId` while retaining distinct `SymbolId`s and owners. Overload signatures and generic parameters likewise require structural/owner-aware identity. Do not replace qname strings with IDs interned from the same unscoped qnames and assume the identity problem is solved. Do not format a bound type or symbol back into text to resolve it again.

The limiting design is the semantic contract. A `SemanticModel` that runs an ordered ladder once per extracted reference is not yet a model in which every expression can be queried for its binding and type. Naming modules after Roslyn concepts does not supply Roslyn's invariants.

Specific gaps in the inspected production path:

- **Incomplete identity migration.** ID-keyed types/members coexist with qname-keyed aliases, inferred returns, and fallback lookup. The reproduced module collision and file-local collision are symptoms of ownership being lost before lookup.
- **Partial inference.** `inference_prelude.rs` runs ordered passes for wrappers and initializers. `infer_call_wrapper_returns` gathers then applies candidates once; it does not implement a general dependency-driven expression/type solver. Recursive and multi-hop inferred returns need worklist/SCC handling and cycle outcomes.
- **Generic unification is incomplete.** `generics.rs` uses first concrete bindings and structural matching; several mismatches are silent no-ops. Full language-specific constraints, contextual typing, variance, overload applicability, and conversions cannot all be reduced to a shared separator/profile table.
- **Inheritance semantics are approximated.** Member lookup performs a bounded breadth-first parent traversal. Python C3 MRO, C++ ambiguity, Rust trait obligations, and Java/C# overload/override rules differ. A shared traversal must be parameterized by actual semantics, not assumed equivalent.
- **Fallbacks remain heuristic.** Parent resolution can choose a member-bearing type from a global name search. Extension lookup reduces receiver names to simple names without a file import context in its API. Composite alias lookup retries name candidates. These need precise visibility/candidate evidence or explicit approximate status.
- **Silent resource bounds.** Supertype traversal is capped at 8, mapped-member recursion at 6, wrapper peeling at 4, and overload alternatives at 4. Budgets are reasonable, but exhaustion should be distinguishable from a semantic miss.

The target architecture should have language front ends produce scoped declarations, occurrences, expressions, constraints, and flow facts; a shared query/dependency engine derives bindings and types; and graph, IDE, and AI APIs project that same semantic state. Keep graph traversal as a consumer. It should not define what a symbol or expression means.

Roslyn's documentation describes semantic analysis in a compilation context. Rust-analyzer provides a particularly relevant Rust implementation model: project configuration inputs, lazy derived queries, expression/body representations, transactional changes, and immutable analysis snapshots. These are architectural references, not an instruction to rewrite BearWisdom using another project's internal APIs. [Roslyn semantic model](https://learn.microsoft.com/en-us/dotnet/csharp/roslyn-sdk/work-with-semantics), [rust-analyzer architecture](https://rust-analyzer.github.io/book/contributing/architecture.html).

## 5. What AlphaVision requires

LSP is a protocol. BearWisdom can replace language-server implementations for supported capabilities while still exposing LSP alongside its native API. Rejecting the protocol would not improve semantic depth.

The current public definitions query takes a name and performs SQL qname/simple-name search (`query/definitions.rs:24`). That is useful symbol search, but does not resolve the exact identifier at a cursor. References similarly start from a name, combine homonyms, and filter target declarations to internal origin. `complete_at` ignores its column parameter and imports candidates by name equality; it is not receiver-sensitive completion over an unsaved expression.

Build these foundations before declaring navigation complete:

1. `symbol_at(snapshot, document, position)` with explicit position encoding and full occurrence ranges.
2. Definition, references, implementation, and type-definition queries by identity; preserve overloads and merged declaration locations.
3. Unsaved document overlays, versioned immutable snapshots, cancellation, syntax-error tolerance, and atomic publication of completed semantic updates.
4. Receiver-aware completion and signature help driven by expression types and candidate sets.
5. Rename with exact editable spans, import aliases, conflict checks, and version-checked edits.
6. Explicit result completeness: exact/ambiguous/partial/unavailable plus the analysis revision and relevant gaps.

The existing `ref_resolutions` table is a valuable starting point, but it is currently an outcome log with starting positions, not a complete editor occurrence model. Keep aggregate edges for graph queries and retain every editable source occurrence separately. The LSP specification and SCIP schema are useful contracts for positions, occurrences, identities, and roles. [LSP 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/), [SCIP schema](https://github.com/sourcegraph/scip/blob/main/scip.proto).

## 6. Performance and token claims

Rust, SQLite covering indexes, parallel parsing, lazy external materialization, and result compaction are sensible choices. None independently prove that the product beats grep in total workflow cost.

The checked-in historical baseline includes approximately 62 minutes for Pascal Castle, 33 minutes for Next.js, and 26 minutes for .NET Fluent UI Blazor. End working sets include about 11.8 GB for the latter and 17.9 GB for the SQL Bitwarden corpus entry. These are recorded baseline values, not new measurements, not necessarily peak memory, and not controlled competitor comparisons. The subsequent package-coverage optimization is already recorded as dramatically improving `ts-ever-demand`; its older 43-minute baseline should not be advertised as current performance.

There is also a structural save-latency concern: incremental resolution reconstructs a compilation and ingests the unchanged database symbol population (`pipeline.rs:217`). Finalization scans edges to rematerialize incoming counts. Measure scaling with total repository size, not just changed-file size. A persistent dependency-aware semantic host should avoid rebuilding all semantic lookup state on every save.

Measure cold index, warm restart, first useful result, p50/p95/p99 query latency, edit-to-correct-result latency, dependency/configuration changes, peak RSS, index size, and concurrent readers during indexing. Pin corpus revisions, toolchain/dependency versions, hardware, and cache state.

For AI economics, measure successful task cost: model input including cached-input accounting, output, tool schema/context overhead, tool result bytes/tokens, reasoning rounds where reported, indexing amortization, and elapsed time. Use real tokenization on returned evidence. `query/context.rs:89` currently assigns fixed costs by symbol kind, while returning ranked symbol metadata; that is an estimate, not proof of delivered-context savings.

Grep remains excellent for exact strings, configuration values, newly edited unindexed files, and unsupported languages. The stronger product objective is to minimize total effort for correct code tasks. Use text search as an efficient primitive and semantic analysis to avoid ambiguous follow-up searches.

## 7. The existing AI benchmark cannot prove output quality

`benchmarks/src/sampler.rs:94` derives impact-analysis ground truth from BearWisdom's own blast-radius query. Other categories similarly use its graph/query output. A tool repeating a wrong or incomplete graph answer can score well; an agent finding additional real dependencies is not rewarded correctly.

`scorer.rs:74` accepts substring presence of a simple name, including mentions in negations or irrelevant contexts. At lines 89 and 95, precision and recall use the same numerator and denominator. False-positive claims are not counted. Expected file evidence is not checked by this scorer. Consequently the composite score is unsuitable for claims about semantic correctness or factual answer quality.

Retain the runner and timing/result infrastructure, but replace task selection and adjudication:

- Pin repositories and create held-out tasks independently of BearWisdom's successful edges.
- Use compiler/indexer truth for scoped static binding tasks, with semantic mismatches manually classified. Include unavailable/ambiguous cases rather than forcing every task to have one answer.
- Use real bug fixes, historical changes, checked patches/tests, and human-reviewed questions for AI task utility.
- Require target identity and source evidence; count false positives and false negatives separately.
- Compare the same model and task budgets with grep/read/glob, BearWisdom plus that floor, GitNexus plus that floor, and Serena or a language-server-backed setup plus that floor.
- Randomize/repeat runs and report confidence intervals, failures, abstentions, token cost, time, and task success. Publish raw tasks, outputs, and environment manifests.
- Add ablations: text only, symbols, scoped bindings, typed chains, external supply, framework flows, and context packing. This identifies what actually earns the gain.

The current SCIP importer is a useful integration seed but is not yet an independent oracle: it maps declarations by file/line, falls back to qname matching, emits aggregate `scip_ref` edges, and upgrades confidence. A correctness oracle must preserve external symbol identity and exact occurrences in an independent store, then compare, not merely reinforce existing graph pairs.

## 8. Competitive assessment

This is a source/architecture comparison, not a new same-machine benchmark. Competitor code was checked on September 5; GitNexus's default branch is `main`, Graphify's is `v8`. Their documentation is a statement of implemented/intended behavior, not independent performance evidence.

| System | Relevant strength | Implication for BearWisdom |
|---|---|---|
| Graphify | Broad knowledge ingestion, graph navigation, assistant integration and explicit provenance | BearWisdom's type/member machinery is substantially deeper than the inspected cross-file resolver; learn from usable evidence and uncertainty presentation |
| GitNexus | A current scope-resolution pipeline, semantic model, callable-value flow, MRO handling, process/context/impact tools, optional program-dependence analysis | A serious technical competitor; “we have a type system and connectors” does not establish a unique advantage |
| Serena | AI-facing semantic navigation/editing using language servers | A relevant user-utility baseline even though it does not pursue the same owned multilingual engine architecture |
| Roslyn / rust-analyzer / other mature language engines | Language-specific semantics, project configuration, and editor analysis | The correctness bar for replacement; also useful reference implementations/oracles |

Graphify's inspected `symbol_resolution.py` explicitly skips member calls in these cross-file helper paths and relies on import/label evidence for supported calls. That supports a narrow semantic-depth advantage, not a claim that every Graphify feature is inferior. Its current repository also contains a published cross-tool evaluation document, so the older local report's blanket “no benchmark” assessment is stale. [Graphify resolver source](https://github.com/Graphify-Labs/graphify/blob/v8/graphify/symbol_resolution.py), [Graphify benchmarks](https://github.com/Graphify-Labs/graphify/blob/v8/BENCHMARKS.md).

GitNexus's current architecture describes an authoritative semantic store, scope-indexed reference resolution and callable-value inclusion propagation. Its source also contains optional reaching-definition/control-dependence passes and explicit budget/truncation accounting. These are meaningful mechanisms. They do not prove compiler correctness, and some older type-environment docs describe a legacy path; use the current scope pipeline as the comparison point. [GitNexus architecture](https://github.com/abhigyanpatwari/GitNexus/blob/main/ARCHITECTURE.md), [pipeline source](https://github.com/abhigyanpatwari/GitNexus/blob/main/gitnexus/src/core/ingestion/scope-resolution/pipeline/run.ts), [Serena](https://github.com/oraios/serena).

I would retire the June report as a basis for positioning. In particular, the claim that engine output at confidence 1.0 constitutes measured accuracy is contradicted by the current metric and the reproduced wrong bindings. A valid competitive claim must specify the task and measured outcome.

## 9. A credible path toward 99%

### Phase 1: Establish correctness and observability

Freeze a representative site-level evaluation corpus and publish metric definitions. Add adversarial cross-scope/module/version/overload cases and independent binding truth. Keep the existing residue worklist, but measure both missed and wrong bindings. Correct the reference cache/occurrence query and expose approximate/ambiguous outcomes.

Exit criteria: each review probe passes; different limits and snapshots produce correct query results; wrong bindings are counted, and exclusions are explicit. Resolution percentage may temporarily fall when incorrect guesses become honest ambiguity.

### Phase 2: Fix identity and incremental semantics

Introduce proper package/module/container identity, lexical binding IDs, and semantic fingerprints separate from identity. Preserve all occurrence and declaration locations. Make incremental vs full parity a first-class invariant over edit sequences, including return changes, field/base changes, adding overloads, imports/re-exports, dependency/config changes, and undo/restart.

Complete the ID-based resolution contract described in section 4: remove semantic string keys, qname-splitting decisions, signature-string matching, and format-then-reparse type bridges from the production resolver. Compare structured signatures and bound identities; compute semantic fingerprints from structural facts. Keep persistent identity/remapping explicit so arena-local integers are never assumed durable across unrelated snapshots.

Exit criteria: shadowing and module noninterference; repeated incremental edits produce the same observable binding/type results as a fresh build on the same inputs; persisted type state survives restart.

### Phase 3: Connect and deepen type inference

Wire CFG/narrowing into production. Use dependency-indexed worklists for type/return obligations, with SCC handling and explicit budget exhaustion. Add overload applicability and language-specific dispatch. Expand contextual callbacks, destructuring, generic propagation, and external member surfaces through the same typed-expression machinery.

Exit criteria: each newly supported construct passes both positive and decoy/negative tests, and improves held-out correct binding recall without unacceptable precision regression.

### Phase 4: Close build-context and supply gaps

Pin target frameworks, compiler options, conditional flags, lockfiles, dependency versions and generated-source configuration. Make library surfaces lazy, reusable, and content-addressed. Prioritize the language-specific reachability/supply cases already documented in the roadmap. Distinguish missing installation from an installed declaration that the resolver failed to bind.

Exit criteria: reproducible supplied-environment results, explicit degraded-mode behavior, and verified transitive type reachability.

### Phase 5: Deliver one excellent IDE and AI experience

Start with one primary language family and one contrasting language. TypeScript/JavaScript is a practical first family because the engine and adversarial evidence are rich there; choose C# if Roslyn parity is the central evaluation target, or Rust if dogfooding is the priority. Keep broad structural indexing for other languages, but publish a capability matrix rather than implying equal semantic support.

Build versioned editor snapshots and a compact AI evidence API over the same model. Suggested concepts are `locate`, `inspect`, `references`, `impact`, and task-context retrieval with explicit token budgets. Every answer should carry stable handles, source ranges/revision, why the evidence is relevant, known gaps, and a way to expand only what the agent needs.

Exit criteria: independent task success, useful token/time savings at equal quality, and measured editor responsiveness on representative projects. Set latency and memory targets against real hardware and repo sizes; they should be acceptance criteria, not retrospective descriptions of whatever the implementation achieves.

## 10. The strongest possible differentiator

An incomparable product is unlikely to come from having the most grammars or graph edge kinds. A defensible advantage would be a shared, continuously current semantic model that gives both the editor and an AI **small, source-backed answers whose correctness and completeness are measurable**.

For an AI, the valuable unit is often an evidence bundle: the relevant declaration, its contract, the exact callers/uses affected, boundary connections, and the uncertainties that could change the answer. For an editor, it is the exact occurrence in the current unsaved snapshot. The same identity/type/dependency substrate can serve both.

Compiler integration is a strategic choice, not a requirement. Using existing compiler/indexer output as an independent oracle should happen early. Optional semantic backends could accelerate high-assurance language support. If fully owned analysis is essential, the project must implement and test the corresponding language semantics itself; generic infrastructure still saves considerable work, but profiles cannot erase language complexity.

The next major investment should therefore be semantic invariants and independent evidence. The existing code contains much of the infrastructure needed to pursue that direction. Increasing the resolution rate without fixing identity, scope, uncertainty, and freshness would make the product appear stronger while making it harder to trust.
