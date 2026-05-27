# Compiler-grade resolution — source of truth

**This document is the single source of truth for the resolution/type-inference
engine's path to compiler behavior.** It supersedes the status sections of
`RESOLUTION-EXTERNAL-ROUTING.md` (its §9/§10 describe the reroute apparatus that
was **deleted** in `a3815b4e`) and folds in the remaining items of
`TYPE-INFERENCE-ROADMAP.md`. When those disagree with this file, this file wins.

Established 2026-05-26. Keep the **Done** table current; move tasks into it as
they land. Tag every task `[generic]` (one engine algorithm) · `[profile]`
(per-language data) · `[hook]` (per-language code) and status `❌ not started` ·
`⚠️ partial` · `✅ done` · `🔬 decision needed`.

---

## Thesis — what "compiler behavior" means here

The resolver is a generic compiler **front-end**: *scope-directed*, *origin-blind*,
over **one uniform symbol/type table** with dependencies **lazily loaded** into it,
and **all language specifics are data** fed to one algorithm.

**Invariants (the bar every task is measured against):**

1. **Origin-blind.** internal / workspace / stdlib / third-party are rows in one
   table. The resolver never branches on origin; "external" is where an import
   points, not a control-flow branch.
2. **Scope-directed, never grep.** A name binds via scope/import rules or it is
   **unresolved**. No bare-name match across the whole program binding to a
   coincidental hit.
3. **Lazy metadata load.** Dependencies decode into the *same* table on demand
   (the compiler's decode-`.rmeta`/`.d.ts` step), not as a sidecar.
4. **Language specifics are data**, not algorithm — `LanguageProfile` /
   `FlowConfig` / extractor output first; `LanguageEngineHooks` only when data
   can't express it.

**Scope boundary (NOT building):** the back end — no borrow-check, trait
coherence, monomorphization, codegen. Only front-end resolution + *enough* type
inference to bind every reference to its declaring symbol.

---

## 0. Done (committed)

| id | what | commit |
|----|------|--------|
| — | Lift `BW_TS_FLOW` gate — TS flow active (phantom hang) | `502955d4` |
| QUAL-1a | Remove `ranked_candidates` + `unique_internal_name` from the default binder ladder | `019fc8a7` |
| — | Fix blast-radius tests that asserted bare-name resolution (now structural) | `5b73a311` |
| BIND-2a | SvelteKit `$lib`/`$app` alias resolution via `svelte.config` (`ts_tsconfig_alias` 31→5,680 edges) | `7fb058e7` |
| LANG-SVELTE-1 | Svelte `$store` auto-subscribe desugar at embedded splice (`$t` 370→13) | `2b4a08cf` |
| LANG-DART-1a | Drop bare Dart library-prefix refs (`i0`/`i4`) across all usage kinds (Dart unresolved 4,291→3,216) | `4be1cb83` |
| QUAL-2a | Delete the dormant `BEARWISDOM_COMPILER_RESOLVE` internal/external reroute apparatus | `a3815b4e` |
| BIND-2b | tsconfig `extends` chains for path aliases | `1471eab7` |
| EXT-2 (slice 1) | `parse_return_type_from_signature` handles `-> T` / `=> T` (Python/Rust/TS-arrow) in addition to `): T`; arrow externals populate return/field maps via the build.rs/augment.rs signature fallback. Unit-tested (`chain_walker_tests.rs`) + end-to-end through `build_with_context` (`mod_tests.rs::signature_derived_return_type_arrow_form`) | `1f03bec9` |
| QUAL-1b (H.2) | Remove the internal whole-program `by_name` family (`{c,kotlin,swift,dart,elixir}_by_name`, `rust_global_name_*`, …) | `c4252e8b` |
| QUAL-3 (H.1) | Delete the import-blind `*_synthetic_global` ext-bare-name grep family (Option A — honest floor) | `bf46353b` |
| EXT-4 | Delete the eager `seed_demand_from_user_refs` — externals load lazily via the expand loop | `aad62601` |
| — | Large-stack parse pool (`build_parse_pool`, 128MB) + iterative `scope_tree`/`recurse_for_object_types` so deep generated `.d.ts` unions don't overflow; full-corpus recapture (honest floor) | `c25ff7d9` |
| EXT-1 | Scope-directed external routing: module-scoped demand for import-qualified externals (`ChainMiss.module` → `SymbolLocationIndex::locate`), recovers the externals EXT-4 dropped, no cross-package grep | `29a055a4` |
| EXT-3 | Supertype walk over external bases: `build_explicit` includes pulled `ext:` files, so `walk_up_with_args` climbs external hierarchies and composes generics across them (reachability-bounded) | _(this change)_ |
| QUAL-5 | Three-state external model: `ResolutionBreakdown` surfaces `external_known_unhydrated` (internal `external_refs`) + a `precision` field apart from `resolved`/`unresolved_unknown`; precision excludes the unhydrated bucket. No schema change — the three states already lived in `edges`/`external_refs`/`unresolved_refs`; the gap was metric visibility | _(this change)_ |

ts-immich: grep-baseline 6,179 → **4,627** unresolved, with honest structural edges.

> **Reality check on the deleted apparatus.** `RESOLUTION-EXTERNAL-ROUTING.md`
> §9/§10 logged a routing spine + externals veto + `external_unique_symbol` +
> arrow-return hydration as "landed." All were gated behind
> `BEARWISDOM_COMPILER_RESOLVE` = **off** — validated in A/B but **never shipped**.
> `a3815b4e` removed them. Their *goals* (D2/D4/D5/S5b below) remain valid and
> are re-anchored here on the clean binder-first path, not the reroute.

---

## A. Binder discipline (name resolution)

### BIND-1 — Lexical shadowing precedence  ·  [generic]  ·  ✅
**Compiler feature:** local shadows param shadows enclosing-member shadows import shadows global.
**Done:** `resolve_via_same_file` now yields (returns `None`) when a non-wildcard import binds the target name — so an explicit import wins over a coincidental file-level sibling. True lexical locals still win because `resolve_via_scope_visible` runs first (#1). Gating, not reordering, keeps the change surgical.
**Deps:** —

### BIND-2 — Complete import/module resolution  ·  [profile]+[generic]  ·  ⚠️
**Compiler feature:** resolve every import specifier to a module (alias, re-export, prefix, package entry, wildcard), then bind the imported name.
- **2a** SvelteKit `$lib`/`$app` — ✅ (`7fb058e7`).
- **2b** tsconfig `extends` chains — ✅ `parse_tsconfig_paths_with_extends` (`ecosystem/manifest/npm.rs`) follows `extends` (relative + `node_modules` package presets, array form, child-wins, cycle-guarded); the npm manifest reader now uses it for `path_aliases`. Inline test module split to `npm_tests.rs` per the sibling-test rule.
- **2c** Re-export / barrel chains — ⚠️ multi-hop **works** (TS relative barrels recurse to depth 5, `typescript/aliases.rs:293`; generic external-npm BFS to depth 4 with cycle guard, `engine/index/classify.rs:110`). Two gaps: (i) the **monorepo seam** — `project file → workspace package → external npm` falls between both walkers (workspace packages are never sources in `pkg_reexports`, `build.rs:660`; `follow_reexports` bails on bare specifiers, `aliases.rs:315`); (ii) re-export following is **TS/JS-only** — no other language has it.
- **2d** Import-prefix → module binding (the Dart `i0.X` case, see LANG-DART-1b) — ❌.

### BIND-3 — Visibility / access-control awareness  ·  [generic]  ·  ✅ (over-resolve)
**Compiler feature:** won't bind `obj.privateMember` from outside its class.
**State (corrected — NOT a uniform hint):** visibility is INCONSISTENT today. It is a **hard gate** in C#/Go/Java/Kotlin/PHP — `is_visible` filters out a `private` target whose file differs from the ref's (`csharp/hooks.rs:199`, `go/hooks.rs:197`, `java/hooks.rs:208`, `kotlin/hooks.rs:232`, `php/hooks.rs:222`). It is a **ranking hint** (`+50` public / `-200` private) only on the ranked-candidates path (`default_resolver.rs:901`). It is **ignored** in the chain walker (`chain.rs` reads visibility nowhere) and for TS/JS/Rust. Net: `obj.privateMember` from outside resolves in TS/JS/Rust and through any chain, but fails in the 5 gated languages.
**Done — over-resolve (deliberate, non-compiler divergence).** BearWisdom is a navigation tool; go-to-definition must reach private members. The audit undercounted — gating existed in **7** languages (C#/Go/Java/Kotlin/PHP **+ Rust + Scala**), all now `is_visible → true`, so visibility never blocks a bind and every language is consistently visibility-blind. This is intentional — do not re-add a visibility gate later thinking the inconsistency is a bug. Two resolution tests that asserted private-cross-file non-resolution were inverted; `java`/`php` analogues still pass because the bare private name isn't import-reachable (legitimate scope behavior, not a gate). The `+50/-200` ranking skew (`default_resolver.rs:901`) is left as-is — it can't block resolution, only mis-order disambiguation.
**Deps:** —

---

## B. Type-checker breadth (generic inference — the spine)

These four are what make the engine a *checker*, not a binder-with-generics. All
`[generic]` — each lifts every language at once. Highest-leverage block.

### INFER-1 — Control-flow-graph-based narrowing  ·  [generic]  ·  ⚠️ (slice 1 ✅)
**Compiler feature:** narrow across the whole flow graph — if/else merge, loop back-edges, reassignment *kills* a narrowing, `&&`/`||` short-circuit, exhaustiveness.

**Today's model (the gap).** A narrowing is a byte interval `Narrowing { name, narrowed_type, byte_start, byte_end }` (`types.rs:786`), emitted by the tree-sitter `type_guard_query` over the guard body's range (`flow.rs:run_type_guard_query`). `LocalTypeCache::lookup` (`index/mod.rs:262`) is a point query — "which interval contains the cursor (= ref byte offset)." `forward` is a flat `name → type` map mutated in source order by `record_local_type` (`lookup_impl.rs:366`); it has no position, so it can't express a fact that holds only over part of a scope. There is no graph: a reassignment never truncates an interval, an if/else join is never unioned, loops/`&&`/exhaustiveness are invisible.

**Slice 1 done — reassignment kills a narrowing (the def-resets-fact seed).** `collect_reassignment_sites` (`flow.rs`) reads the `assignment_query` `@lhs` capture into `(name, byte_offset)`; `kill_narrowings_at_reassignments` truncates any `Narrowing`/`DiscriminantNarrowing` whose `[start, end)` straddles a same-name reassignment at `start < B < end` down to `[start, B)`, earliest-B-wins. The strict-interior test naturally excludes the declaration that establishes the variable (it sits before any guard range), so no node-kind discrimination is needed — one generic post-pass over the runner's output, all languages at once, zero per-language code. This models the CFG's "a def block resets the fact on its out-edge" as interval truncation. Failing→green test: `flow_tests.rs::flow_reassignment_kills_narrowing_for_rest_of_scope` (`x.bar()` after `x = reset()` inside an `instanceof` block no longer sees `Derived`). 29 flow + 317 type_checker + 233 resolve tests green.

**Why this first.** It proves the "facts have kill points" machinery against the existing interval model without a CFG-wide fixed point — the smallest honest increment before the graph build-out.

**Design proposal (the full build-out, for review).** Replace the byte-interval narrowing model with a per-function CFG that the narrowing pass walks. Generic-engine-first; `LanguageProfile`/`FlowConfig` stays data, `LanguageEngineHooks` only if data can't express a language's branch shapes.
- **Representation:** `BasicBlock { stmts, byte_range }` + typed edges `Edge { to, fact: Option<Narrowing> }`; a per-function `Cfg { blocks, edges, entry }`. Facts (the narrowed `(name → type)` set) ride edges; a block's in-fact is the **merge** of its predecessors' out-facts.
- **Build location:** a new `build_cfg(fn_node, cfg: &FlowConfig)` in `indexer/flow` (alongside `run_flow_queries`), per function, from the tree-sitter tree — same input the query runner uses, so it stays `[generic]` + data-driven. Branch/loop/switch node kinds come from `FlowConfig` (today's queries already name `if_statement`/`switch_statement`/`statement_block` per language).
- **Narrowing walk:** forward dataflow. Edge fact = the guard condition's implied narrowing (true-edge gets `x: T`, false-edge gets the negation). **Join** block = `∪` of predecessor facts (a branch that narrowed `x` out contributes `never`). **Reassignment** = a def in a block clears `x` from the block's out-fact (slice 1 generalized). **Loop** back-edges iterate to a **fixed point** (facts only widen; monotone, bounded by the type lattice). `&&`/`||` lower to extra edges so the guard fact sits on the true edge. Exhaustive `switch` leaves the default block unreachable → `never`.
- **Consumer integration (RESOLVED — CFG-native):** the narrowing consumers query the CFG directly at a program point rather than reading a compiled-down interval table. `LocalTypeCache`/`loop_body.rs:419`/the chain walker resolve `fact_at(name, ref_byte)` (the block containing the ref → its in-fact), so they see the true path-sensitive fact, not an interval approximation. The byte-interval `Narrowing` model is retired as the source of truth; slice 1's truncation becomes the CFG's def-resets-fact edge. The larger blast radius — every narrowing consumer is rewired — is accepted for full path sensitivity and no interval ceiling.

**Architectural decisions (1–3 default as noted, pending override; 4 resolved by the architect, 5 follows from it):**
1. **CFG data structure & ownership** — petgraph vs. a hand-rolled `Vec<BasicBlock>` + edge list. Hand-rolled is lighter and avoids a dep; petgraph gives traversal/SCC utilities for the loop fixed-point. Lean hand-rolled unless the loop machinery wants SCCs.
2. **Build location & granularity** — per-function in `indexer/flow` at extract time (CFG serialized into `FlowMeta`, survives to resolve), vs. built lazily at resolve time per file. Extract-time keeps the resolver thin but bloats `FlowMeta`; resolve-time keeps `FlowMeta` slim but re-parses. (Slice 1 is extract-time post-pass — consistent with the former.)
3. **Fixed-point strategy** — worklist vs. naive iterate-to-stable; widening operator and iteration cap for loops (TS caps at a small N). Pick the cap.
4. **Coexist vs. replace the interval model** — ✅ **RESOLVED: replace (CFG-native).** The resolver queries the CFG directly; the larger blast radius on `loop_body.rs` / chain walker / `LocalTypeCache` is accepted for full path sensitivity and no interval ceiling. (The compile-down-to-intervals alternative is rejected.)
5. **Fact representation at joins** — follows from #4: CFG-native ⇒ a join produces a **real union type** (`string ∪ null`) in the type arena. (The "drop to declared type on disagreement" shortcut only made sense under the now-rejected compile-down option.)

**Remaining slices (next, scaffolded — not faked):**
- **if/else join merge** — union predecessor facts at the merge block; needs the CFG join node + a fact-merge op (decision 5).
- **loop back-edge fixed point** — iterate facts over the back-edge to stability (decisions 1, 3).
- **`&&`/`||` short-circuit** — lower to extra edges so the guard fact lands on the true edge.
- **switch exhaustiveness** — unreachable default block ⇒ `never` (ties to INFER-5 for the "all branches covered" check).
- **reassignment-kills** is generalized into the CFG's def-resets-fact once the graph lands; slice 1 stands alone until then.

**Fix:** new machinery — a per-function flow graph the narrowing pass walks. Largest item; caps how far INFER-2..4 reach.
**Deps:** — (foundational)

### INFER-2 — Interprocedural type propagation  ·  [generic]  ·  ❌
**Compiler feature:** an *inferred* (un-annotated) return/local type crosses call + file boundaries.
**Gap:** `LocalTypeCache` is per-file, per-thread (`index/mod.rs:210`). Only declared/signature types cross files; inferred types die at the file edge.
**Fix:** persist inferred return/field types into the shared type maps so other files' chains read them.
**Deps:** INFER-3 (for the inferred returns to exist)

### INFER-3 — Body-based return-type inference  ·  [generic]  ·  ❌
**Compiler feature:** infer a function's return type from its `return` statements when unannotated.
**Gap:** `return_type` is sourced from signature/declaration only.
**Fix:** when no annotation, infer from the function body's return expressions (reuses INFER-4).
**Deps:** INFER-4

### INFER-4 — Expression-level type inference  ·  [generic]  ·  ⚠️
**Compiler feature:** type an arbitrary RHS expression.
**Gap:** flow infers only call / literal / ident (`resolve_arg_types`, flow runner). Missing: ternary, array/object-literal element types, `await` unwrap, spread, index-access, binary ops.
**Deps:** —

### INFER-5 — Structural assignability / shape matching  ·  [generic]  ·  ⚠️
**Compiler feature:** a value matches an interface *by shape* (TS structural typing; Go implicit interface satisfaction).
**Gap:** "structural-supertype discovery" exists (`members.rs:164`), but `is_assignable_to` (`subtype.rs`) is conservative *nominal*.
**Fix:** shape-based assignability over the member table.
**Deps:** — (prerequisite for LANG-GO-1)

### INFER-6 — Built-in container generic semantics  ·  [generic]+externals  ·  ❌
**Compiler feature:** `Array<T>.pop():T`, `Map<K,V>.get():V`, `await Promise<T>:T`.
**Gap:** needs stdlib generic *signatures* — ties to EXT-2.
**Deps:** EXT-2

### INFER-7 — Expected-type / bidirectional (roadmap D7/G1)  ·  [generic]  ·  ✅
**Closed.** Confirmed: turbofish consumed (`chain.rs:449`); cast/type-assertion adopted as root and mid-chain type (`chain.rs:322,363`; extractor emits it, `typescript/calls.rs:926`). The LHS annotation is fully exploited — it seeds `local_type` forward so any later chain rooted on the annotated var gets the declared type (`loop_body.rs:347`). The only unused slot is `expected_return` for overload selection (`DispatchQuery`, always `None` from `loop_body.rs:456`), which affects only ReturnType-dispatch languages (Haskell / Rust trait specialization) and resolves nothing new on the TS/JS/C#/Java corpus. Nothing to build.

---

## C. Externals as first-class (the biggest *type-aware* unlock)

Re-anchors the routing doc's D2/D4/D5/S5b on the binder-first path. **This is the
#1 corpus-leverage block** — chain-walking through Prisma/ORM/tidyverse/Flutter
return types is gated here, and it is what makes external types ordinary table rows.

### EXT-1 — Scope-directed external routing (D2)  ·  [generic]  ·  ✅
**Compiler feature:** decide external-ness *first* by scope/import, not as the residual.
**Done.** `ChainMiss` gained an optional `module`; at the Tier-1.5 external classification (`loop_body.rs`), an `ext:<module>`-classified ref now records a **module-scoped demand** (`ChainMiss { current_type:"", target_name: <leaf>, module: Some(<module>) }`). `expand::locate_via_symbol_index` resolves a module-scoped miss via `SymbolLocationIndex::locate(module, name)` — the file defining the name *inside that package*, with **no `find_by_name` cross-package fallback** (a miss under the module is a genuine gap, not licence to grep). A re-resolve upgrades the opaque `external_ref` into a real edge. The pull is reachability-bounded for free: `locate` only answers for `(module, name)` pairs the demand-driven index already carries (npm/cargo/go_mod/etc. `build_symbol_index`), so builtin/primitive namespaces and unscanned modules locate to nothing and stay `external_ref`, exactly as before. No env gate. Recovers the import-qualified externals EXT-4 dropped. Verified: ts-rallly +29 binds (no regression); go-fiber rate flat.
**Scope note (intentional):** only the *records-demand* half landed. The *short-circuit-internal-strategies* half is intentionally NOT done — post-QUAL-1 the internal strategies are scope-directed and cannot coincidentally bind an import name, so running them first then classifying external as residual is functionally equivalent AND avoids the lexical-shadowing risk a blind short-circuit would carry (a true inner-scope binding must still win over the import per BIND-1).
**Deps:** QUAL-1 (no grep to pre-empt) ✅

### EXT-2 — Emit external return/field types (D4 / S5)  ·  [generic]+[profile]  ·  ⚠️ (slice 1 ✅)
**Slice 1 done (`1f03bec9`):** `parse_return_type_from_signature` (`chain_walker.rs:591`) now recognizes `-> T` (Python/Rust) and `=> T` (TS-arrow) on top of `): T`, with angle-depth guard, Rust `where`/block stripping, and trailing-`:` handling. External methods using those forms populate the return/field maps through the signature fallback build.rs/augment.rs already run. Covered by `chain_walker_tests.rs` (colon/arrow/generic/where/block) + end-to-end `mod_tests.rs::signature_derived_return_type_arrow_form` (Python `-> User` external → `return_type_name` via `build_with_context`).
**Remaining:** (a) verify real external method symbols actually carry signatures across ecosystems (extractor coverage — measurable only at closeout recapture, not now); (b) the `r.module.is_some()` TypeRef filter (`build.rs:238`, `augment.rs:128`) still drops module-tagged external TypeRefs — the signature fallback sidesteps it but a direct fix would widen coverage.
**Compiler feature:** `repo.find().email` walks past the first hop because the external method's return type is known.
**Gap (re-scoped after tracing the pipeline — most of the infrastructure already exists):**
- TypeRef-derived return/field types: `build.rs:223-391` (full) + `augment.rs:121-209` (incremental/hydrated). The demand loop (`full.rs:743-790`) **does** augment the cached index with hydrated files via `augment.rs` before re-resolving, so hydrated externals *do* flow through the map-build. The earlier "demand-hydrated files don't enter the maps" claim was wrong.
- Signature-based return-type fallback already runs in **both** paths for *any* method lacking a TypeRef return type (`build.rs:362-374`, `augment.rs:185-200`) — not just .NET, despite the comments.
- **The real remaining hole is `parse_return_type_from_signature` (`chain_walker.rs:591`): it only recognizes the `): T` colon form** (TS methods, Kotlin `fun`). It does **not** handle Python/Rust `-> T` or TS-arrow `=> T`, so externals whose signature uses those forms get no return type. Secondary: external symbols must actually *carry* a signature for the fallback to fire, and the `r.module.is_some()` TypeRef filter (`build.rs:238`, `augment.rs:128`) drops module-tagged external TypeRefs (signature fallback sidesteps this).
**Fix (bounded first slice):** extend `parse_return_type_from_signature` to the `-> T` and `=> T` return forms (careful: load-bearing across all languages, gate behind thorough sibling tests + regression cases for the existing colon/generic forms). Then verify external method symbols carry signatures. Failing test first: `PrismaClient.findUnique().email` resolves the second hop; a Python `def f() -> User` external second hop.
**Deps:** EXT-3
**Corpus proof:** `PrismaClient.findUnique/findMany`, R `mutate/ggplot`, C `curl_easy_*`.

### EXT-3 — Lazy member hydration, origin-blind (D3/S4)  ·  [generic]  ·  ✅
**Done.** `build_explicit` (supertype.rs) no longer skips `ext:` files, so a pulled external base's OWN `Inherits`/`Implements` edges enter the `SupertypeGraph`. `walk_up_with_args` then climbs the external hierarchy and composes generic args across those hops; the existing `members.lookup_with_binding` → `env.bind_positional(owner_args)` → `yield_type_of` path (chain.rs:432-475) substitutes the bound args into an inherited member's return type — so a method on a deep external base resolves with the concrete type, not an unbound `T`. The generic-substitution machinery already existed; the only blocker was the missing external edges. Reachability-bounded for free: the graph is built from `parsed`, which holds only the externals the demand loop pulled (post-EXT-4), never the whole dep tree. **Verified:** new `build_explicit_includes_pulled_external_base_edges` test (failing→green) + 317 type_checker tests green; ts-rallly / java-spring-petclinic / ts-nestjs-realworld regression-free. Corpus gain is shape-dependent (OO inheritance from pulled external bases) — measurable at closeout, not on the functional/ceiling/sources-absent samples here.
**Earlier state (for reference):** `qualified_member_lookup` already resolved external *members* origin-blind (the `members.rs:68` `ext:` skip was never the blocker); the supertype-edge gap above was.

### EXT-4 — Delete the eager seed (D5/S6)  ·  [generic]  ·  ✅
**Done.** Deleted `seed_demand_from_user_refs` + `_inner` + `enqueue_named_target` + `follow_inheritance_closure` (`stage_link.rs`, ~370 lines) and its `full.rs` call. Externals now load lazily through the Stage-2 chain-miss → `expand` → re-resolve loop only — no eager pre-pull of every import-qualified external up front (that was the dominant reindex cost on dep-heavy projects). **Tradeoff (honest floor):** import-qualified externals on demand-driven ecosystems (npm/TS) that don't surface a chain miss may stay unresolved until **EXT-1** records that demand on the scoped path; chained externals are already covered by the expand loop.
**Deps:** EXT-1 (to recover the bare import-qualified external coverage)

### EXT-5 — Ambient framework globals via real packages  ·  [profile]  ·  ❌
**Compiler feature:** `expect`/`it`/`describe`/`assertTrue`/`hasSize` resolve to the test framework's declared types.
**Gap:** highest-frequency unresolved category corpus-wide (jest/vitest/junit/mockk/scalatest across TS/JS/Java/Kotlin/PHP/Scala/Lua). Must come from indexing the framework's **actual type package** (reachability externals + tsconfig `types`), **never synthetic stubs** (`ecosystem files are locators only`).
**Deps:** EXT-2/EXT-3 (the package must hydrate)

---

## D. Language-specific resolution rules

Per-language name/method-resolution a compiler hard-codes. Ranked by corpus volume.

### LANG-RUST-1 — Trait method resolution  ·  [hook]  ·  ❌
Autoref/autoderef + trait-in-scope + which `impl Trait for T` provides `.foo()`; associated types; blanket impls. Confirmed absent in `rust_lang` — `.method()` is name-only. The defining Rust compiler feature.

### LANG-CPP-1 — Template-parameter binding + ADL  ·  [profile]+[hook]  ·  ❌
`T1`/`T2`/`OutputIt`/`iterator_t`/`charT` are top C++ unresolved — template params aren't treated as generic params. Plus argument-dependent lookup and overload. High corpus volume.

### LANG-GO-1 — Implicit interface satisfaction + embedded promotion  ·  [hook]  ·  ⚠️
- **Embedded promotion** — ✅ chained `t.BaseMethod()` now resolves. Two fixes were needed (the earlier "inherits_map populated" claim was wrong for full builds): both `inherits_map` child-kind filters excluded `Struct` (only Class/Interface/Trait), so Go struct embedding never populated the map — `build.rs` and `augment.rs` now include `Struct`; and `walk_go_chain` Phase 3 now climbs via `find_member_via_inheritance` (strategy `go_chain_inheritance`, conf 0.90). Bare-name promoted refs from inside the struct already resolved via `resolve_via_enclosing_member`.
- **Implicit interface satisfaction** — ❌ structural ("has the methods"), blocked on INFER-5.
**Deps:** INFER-5 (interface-satisfaction half only; embedded-promotion fix is independent)

### LANG-KOTLIN-1 — Extension functions + smart-casts  ·  [profile]+[hook]  ·  ❌
`x.ext()` resolved by receiver type to a top-level/member extension fn; smart-casts (roadmap L2 deferred — grammar-version-dependent); companion objects.
**Scoped (why it's not a C#-style quick slice):** for `fun String.shout()` the extractor emits the receiver `String` as a **TypeRef** (`kotlin/coverage_tests.rs:464`), and the symbol qname is just `shout` — so Phase 3's `current_type.last` lookup misses. Unlike C# (where `signature_is_extension_on` reads `this <recv>` from the signature inside the dedicated `walk_csharp_chain`), Kotlin (a) routes through the **shared** `chain::resolve_via_chain` and (b) has no signature marker — the receiver is a position-dependent TypeRef indistinguishable from a first-param type. Clean fix needs extractor-level receiver tagging (a dedicated field / `extension_receiver` map) + a config-gated Phase-3 fallback in the shared walker (reusable for Scala). Multi-part.

### LANG-CSHARP-1 — Extension-method search across usings  ·  [hook]  ·  ⚠️
**Extension methods** — ✅ `walk_csharp_chain` Phase 3 now, after instance-member lookups miss, searches `by_name(method)` for a static method whose signature's first parameter is `this <current_type>` (`signature_is_extension_on`, strategy `csharp_extension_method`, conf 0.85). The `this`-param is read straight from the stored signature text — no extract-time tag needed. Visibility-blind (no `using`-scope gate), consistent with the over-resolve model (BIND-3). **Partial classes** — ✅ work incidentally: `members_by_parent` accumulates members under the shared parent qname across files (`build.rs:153`). **Source-generated members** — ❌ not handled (the remaining ⚠️).

### LANG-PY-1 — MRO  ·  [hook]  ·  ❌
Method resolution order for multiple inheritance. (Dynamic `__getattr__`/duck typing out of scope.)

### LANG-SWIFT-1 — Protocol extensions  ·  [hook]  ·  ❌
Protocol extensions, retroactive conformance, `some`/`any`.

### LANG-TS-1 — Remaining type-level operations  ·  [profile]  ·  ⚠️
Structural matching (INFER-5); conditional-beyond-decidable; mapped-beyond-transparent (roadmap TS2 — rare); template-literal types; `satisfies`; `infer`; declaration merging / module augmentation. Mostly low corpus value — keep deprioritized.

### LANG-DART-1 — Dart binding  ·  [hook]  ·  ⚠️
- **1a** bare prefix-ref drop — ✅ (`4be1cb83`).
- **1b** prefix → library binding — ❌ `i0.Value` should resolve `Value` in `i0`'s module (drift external) or the local file (`i2.X`). `collect_dart_import_aliases` currently *drops* the prefix; needs a prefix→module map (BIND-2d). Dart still 3,216 unresolved.
- **1c** Flutter SDK externals — ❌ `ListTile` etc. (EXT-2/EXT-3).

### Deferred per-language (recorded, low value)
- Kotlin smart-cast / Rust `if let`/`match` discriminant (roadmap L2/L3) — grammar-uncertain.
- Haskell generic-param bounds (roadmap L1) — params outside any bracket clause, near-zero payoff.

---

## E. Correctness — grep family, synthetics, confidence

### QUAL-1 — Bar the whole grep family (D1/D9)  ·  [generic]+[hook]  ·  ⚠️ (H.1+H.2 ✅, H.3 remains)
**Invariant:** never bind a bare name to a coincidental same-name symbol.
- **1a** `ranked_candidates` + `unique_internal_name` removed from default ladder — ✅ (`019fc8a7`).
- **1b** ✅ for the two grep buckets: the ordering bugs are fixed (Appendix), **H.2** (internal whole-program `by_name` — `{c,kotlin,swift,dart,elixir}_by_name`, `rust_global_name_*`, …) deleted in `c4252e8b`, and **H.1** (the `*_synthetic_global` ext-bare-name family) deleted in `bf46353b` (= QUAL-3). Verified: 0 occurrences of any of those strategy literals remain. **Remaining = H.3 only** (~20 borderline: same-file/module fallbacks whose risk is ladder order, or `by_name` scoped by a coarse path/prefix rather than a qualified-name lookup). Each needs reorder-after-imports or a tighter filter — none is a whole-program bare grep, so this is precision-tuning, not invariant-breaking.
**Deps:** BIND-1 ✅ (so removing same-file grep doesn't drop legitimate scope hits)

### QUAL-2 — Single engine, no per-language grep (D9)  ·  [generic]  ·  ⚠️
- **2a** reroute apparatus deleted — ✅ (`a3815b4e`).
- **2b** per-language hooks still run their own Step-5 grep fallbacks; consolidate resolution into one engine algorithm + data. Largest structural refactor; do after QUAL-1b.

### QUAL-3 — `*_synthetic_global` is import-blind external grep  ·  [generic]+[profile]  ·  ✅
**Done (`bf46353b`).** Option A executed: the whole `*_synthetic_global` ext-bare-name family is deleted (0 strategy literals remain). A bare external name now resolves only via scope/import or a curated prelude, or stays unresolved. This is the honest-floor crater behind the corpus drop (kept: `rust_prelude`, `scala_implicit_import`, `php_global_function`, `ada_modular_primitive`). True-globals like `print`/`len` will be recovered later via curated per-language **prelude strategies** (EXT-5-adjacent), not by name grep. Original audit/decision below.

**Resolved (was 🔬).** Audited the 17 `*_synthetic_global` strategies: they bind to **real** symbols parsed from real SDKs by real walkers (`dart_sdk.rs`, `flutter_sdk.rs` ~700 files, `ext:cpython-stdlib:`, `kotlin_stdlib`/`jdk_src`/`android_sdk`/maven sources jars). **Not** fabricated stubs — `ecosystem files are locators only` is honored; the "synthetic" name is a misnomer. The stub-crutch framing is dead.
**Real defect:** the binding is whole-program bare-name `by_name` with first-`ext:`-match-wins and **no import/scope check** (`<lang>/hooks.rs` `resolve_ref`; `by_name` is a flat map, `lookup_impl.rs:18`). That is Invariant #2's grep, filtered to `ext:` rows — real target, unjustified claim. `dart_synthetic_global` = 176k edges (55% of ts-immich) is mostly import-gated Flutter widgets (`ListTile`/`Widget`/`BuildContext`) bound by coincidence.
**Decision (Option A — honest floor):** **delete** the whole `*_synthetic_global` ext-bare-name family outright (the import-blind `by_name`-into-`ext:` grep). A bare external name now resolves only via a scope/import strategy, a curated prelude check, or stays unresolved — never by coincidental name match. This drops the import-gated coincidences (the ~176k Dart Flutter binds and equivalents) AND the true-globals (`print`/`len`) until per-language **prelude strategies** (curated, scope-directed — like `rust_prelude`) are rebuilt to recover the latter legitimately. The baseline will fall to the honest scope-directed floor first, then climb back on real binds. Kept: `rust_prelude`, `scala_implicit_import`, `php_global_function`, `ada_modular_primitive` (these are curated/qualified-name, not by_name grep).
**Deps:** QUAL-1b, BIND-2; QUAL-5 for the honest restatement.

### QUAL-4 — Collapse confidence to {resolved, unresolved} (D8)  ·  ⚠️
As grep goes, drop 0.x-confidence edges; a front-end answers resolved or error. Falls out of QUAL-1/QUAL-2.

### QUAL-5 — Three-state external model  ·  [generic]  ·  ✅
**Compiler feature:** a miss is not one bucket — separate "dep source absent" from "genuine unknown" so precision is measurable instead of hidden in a coverage rate.
**Done.** No schema change: the three states were already persisted in three disjoint tables — `edges` (`resolved`), `external_refs` (`external_known_unhydrated` — the EXT-1 outcome: `classify_external_ns` routed the ref to a known `ext:<module>` namespace but `locate` never hydrated the source, `loop_body.rs:608`), `unresolved_refs` (`unresolved_unknown` — `classify_external_ns → None`, `loop_body.rs:747`). The gap was the metric: `ResolutionBreakdown` (`stats.rs`) computed `internal_edges / (internal_edges + internal_unresolved)` — already precision, since `internal_unresolved` joins only `unresolved_refs` — but never surfaced the third bucket, so the dependency-availability gap was invisible and the precision contract was undocumented/untested. Added `external_known_unhydrated` (internal-origin `external_refs` count) and a `precision` field (== `internal_resolution_rate`, naming the contract) to `ResolutionBreakdown`, and surfaced both in the MCP/CLI compact header (`compact.rs`). Precision explicitly excludes the unhydrated bucket from the denominator. Failing-first locked by `resolution_breakdown_distinguishes_three_states` (resolved + dep-absent + typo → 1/1/1, precision 50%) and `external_known_unhydrated_excluded_from_precision_denominator` (all-unhydrated → precision 100%). Consumers unchanged: `cmd_resolution_gate` serializes the breakdown via serde (new fields flow through); the quality-check baseline reads only `resolution_rate`/`internal_edges` (preserved). `resolution_gate` integration suite green (rate still matches dead-code health).
**Deps:** EXT-1 ✅ (produces the `external_known_unhydrated` signal at the `locate` miss).

---

## F. Corpus-driven gaps (volume the above doesn't directly cover)

### CORPUS-1 — Component-in-template binding  ·  [profile]  ·  ⚠️
Svelte done (LANG-SVELTE-1 + BIND-2a). Generalize the import→component link to **Vue** (`VCol`/`VBtn`), **Astro**, **MDX**, **Angular templates**, **HTML**.

### CORPUS-2 — DSL / markup resolvers  ·  [profile]+[hook]  ·  ❌
Weak or absent resolvers dragging the corpus: **astro** (0%), **gsp** (16%), **bicep** (44%), **vbnet** (46%), **matlab** (62%), **html** (66%), **prolog** (74%), **r** (76%). Lower per-language volume; breadth play.

---

## G. Hygiene / consolidation

### DOC-1 — Retire `RESOLUTION-EXTERNAL-ROUTING.md`  ·  ✅
Deleted 2026-05-26; its north star (D1–D9) is folded into §A/§C/§E here.

### DOC-2 — Retire `TYPE-INFERENCE-ROADMAP.md`  ·  ✅
Deleted 2026-05-26 (along with `RESOLUTION-GOAL.md`, `RESOLUTION-TASKS.md`,
`GENERIC-PARAM-UNIFICATION.md`, `baseline-gaps.md`, `baseline-by-so2025.md`).
Remaining roadmap items folded into LANG-* / INFER-* here; R1 is moot (the gate
and arrow-return path were deleted in `a3815b4e`).

### DOC-3 — Close residual long-tail  ·  ❌
13 `$t` in plain `.ts` (outside the SFC splice path); small `$lib` edge cases (`keyboard`/`imageLoader`/`useLogger`).

### DOC-4 — Full-corpus recapture  ·  ❌
The baseline (`baseline-all.json`, 2026-05-24) predates this session — svelte numbers especially are stale, grep-removal lowered some honestly. One recapture at initiative closeout (per the one-recapture rule), reporting QUAL-5's three states separately.

---

## Leverage-ranked sequence

1. **EXT-2 + EXT-3** — emit/hydrate external return types. Unblocks ORM/stdlib/Flutter chains (the largest *type-aware* corpus block) and INFER-6; the chain-walker is already waiting on it.
2. **QUAL-1b + BIND-1 (incl. QUAL-3 split)** — finish barring the grep family (with BIND-1 so legitimate scope hits survive), and split `*_synthetic_global` into kept true-globals vs import-gated SDK symbols gated on an import. Completes the "never grep" invariant and restates the inflated Dart/Swift/Kotlin engine %.
3. **INFER-1 (CFG)** — the foundational checker feature; unblocks the real reach of INFER-2/3/4.
4. **EXT-1 + EXT-4 + QUAL-5** — scope-directed routing, delete the seed, three-state metric (the clean redo of the deleted routing spine).
5. **LANG-RUST-1 / LANG-CPP-1 / LANG-GO-1** — the high-volume per-language method-resolution rules.
6. **EXT-5, CORPUS-1/2, INFER-2/3/4/5, remaining LANG-*** — breadth.
7. **DOC-1..4** — in passing / at closeout.

**State (2026-05-27):** the entire #1 externals block is done — EXT-2 slice-1 ✅ (`1f03bec9`), EXT-1 ✅ (`29a055a4`), EXT-3 ✅ (`fa0a1e43`), EXT-4 ✅ (`aad62601`); #2's QUAL-1b H.1+H.2 ✅ (`bf46353b`/`c4252e8b`) and BIND-1 ✅. **QUAL-5 ✅ (`de0d9820`)** — the three-state external model now reports `resolved` / `external_known_unhydrated` / `unresolved_unknown` separately, so precision excludes dep-source gaps. **INFER-1 ⚠️** — slice 1 (reassignment-kills-narrowing) landed (`afedb777`); the full per-function CFG is designed and the integration fork is resolved **CFG-native** (consumers query the CFG directly, real union types at joins). Remaining top of the stack: the INFER-1 CFG build-out (gates INFER-2/3/4).

**Single best next move:** INFER-1 CFG build-out — implement the per-function CFG (hand-rolled `Vec<BasicBlock>` + edges, extract-time into `FlowMeta`, worklist fixed-point) and rewire the narrowing consumers to `fact_at(name, ref_byte)`. First slice: **if/else join merge** (forces the first real join node + the union fact-merge op that the loop / `&&`·`||` / exhaustiveness slices all reuse). Then loop back-edge fixed point, `&&`/`||` edge-lowering, switch exhaustiveness.

---

## Appendix — Grep-family inventory (QUAL-1b work-list)

Classification of every resolution `strategy` (every `strategy:` literal across 45 language hooks + the generic `default_resolver.rs`/`bare.rs`/`chain.rs`), bucketed by the QUAL-1b test: *does the strategy justify the bind by program structure, or match a bare name against the whole-program index?* Of ~200+ strategies the large majority are SCOPE-DIRECTED (untracked here). Three buckets need action; ~6 legitimate bare-name strategies are kept.

**Ordering bugs — ✅ FIXED.** These greps ran *before* the scope/import strategy and shadowed the correct bind; each now runs after scope/import:
- `default_same_file` / `engine_bare_same_file` gated to yield when a non-wildcard import binds the name (the generic BIND-1 fix) — true locals still win via `scope_visible` (#1).
- `elixir_synthetic_global` moved after scope/module/alias/qualified, before `elixir_by_name`.
- `erlang_otp_arity` moved after `erlang_import_arity` + `resolve_all` + same-file, before `erlang_cross_file_arity`.
- `scala_synthetic_global` early pre-scope call removed; now a final fallback after scope/import/implicit (single-segment DSL stays covered by the post-chain-failure call).
- `c_synthetic_global` moved after scope/qualified/same-file, before `c_by_name`.

### H.1 — External bare-name (the QUAL-3 synthetic family) · ~20
`by_name` filtered only to `ext:` rows → binds "any external symbol of this name." **Fix = QUAL-3 split:** keep the true-global subset, gate import-requiring symbols on an actual import.
`{c,java,kotlin,swift,python,ruby,php,perl,lua,dart,r,scala,fsharp,haskell,elixir}_synthetic_global` (each `<lang>/hooks.rs` `resolve_ref`) · `erlang_otp_arity` · `puppet_synthetic_global` · `gdscript_synthetic_global` · `heex_ext_component` · `prolog_runtime_fallback` (ext: + path heuristic) · `robot_global_name`/`robot_global_normalized` (ext: branch).

### H.2 — Internal whole-program bare-name · ~18
`by_name` over project rows with no import/scope justification ("any symbol of this name wins"). **Fix = make scope-directed or delete** (needs BIND-1 so legitimate scope hits survive).
`c_by_name` · `kotlin_by_name` · `swift_by_name` · `dart_by_name` · `elixir_by_name` · `erlang_cross_file_arity` · `rust_global_name_fallback` · `rust_global_name_scoped` · `rust_global_typeref_fallback` · `default_ranked_candidate` (dormant in `resolve_all`, still callable) · `hcl_provider_alias_cross_file` · `hcl_cross_file_bare` · `nix_attr_path_last_seg` · `puppet_internal_global` · `puppet_unqualified_fallback` · `prolog_project_by_name` · `gdscript_internal_global` · `heex_internal_component`.

### H.3 — Borderline (legitimate mechanism, weak guard) · ~20
Either a same-file/same-module fallback whose only risk is ladder ORDER, or a `by_name` scoped by a coarse path/prefix filter rather than a qualified-name lookup. **Fix = reorder after imports, or tighten the filter.**
`ts_same_file` · `default_unique_internal_name` (whole-program `by_name`, saved only by a single-candidate gate) · `default_ambient_package` · `default_ambient_namespace_path` · `ts_lib_globals` · `ts_npm_globals` · `python_ref_module_path` · `python_ref_module_via_import` · `python_module_qualified_by_name` · `python_from_import_prefix` · `ruby_external_gem` · `rust_same_file_name_fallback` · `rust_same_module_by_name` · `nim_stdlib_any` (any stdlib import admits the whole stdlib) · `odin_same_package` · `bicep_runtime_grammar` · `bash_shell_source` · `robot_dynamic_library_fallback` (first class in the imported file, name-free).

### Kept — legitimate bare-name (TRUE-GLOBAL) · ~6
No import needed; binds a real language/runtime global. **No action.**
`engine_generic_param` (declared type param) · `rust_prelude` (PRELUDE_NAMES + stdlib path) · `scala_implicit_import` (`java.lang`/`scala`/`Predef`) · `ada_modular_primitive` (RM 13.7) · `php_global_function` (`by_qualified_name` on a bare name — PHP globals live at global scope).

**Structural surprise:** Swift has **no** import strategy at all — `swift_by_name` does double duty as both "resolve via imported module" and "match any global symbol," so the `import` entries in `file_ctx.imports` (populated, used for external classification) never scope a bind. Fixing Swift means adding a real `swift_import` strategy, not just removing the grep.
