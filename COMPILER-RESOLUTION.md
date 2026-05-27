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

### INFER-1 — Control-flow-graph-based narrowing  ·  [generic]  ·  ❌
**Compiler feature:** narrow across the whole flow graph — if/else merge, loop back-edges, reassignment *kills* a narrowing, `&&`/`||` short-circuit, exhaustiveness.
**Gap:** no CFG exists. Narrowing is lexical-block + early-return only (`type_checker/`, `indexer/flow`).
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

### EXT-1 — Scope-directed external routing (D2)  ·  [generic]  ·  ❌
**Compiler feature:** decide external-ness *first* by scope/import, not as the residual.
**Gap:** external is tier-1.5 residual (`classify_external_ns`) after internal resolution.
**Fix (target form, not the deleted reroute):** thread a scoped resolution request into the engine so an import-determined-external ref short-circuits the internal strategies and records demand. No env gate.
**Deps:** QUAL-1 (no grep to pre-empt)

### EXT-2 — Emit external return/field types (D4 / S5)  ·  [generic]+[profile]  ·  ❌
**Compiler feature:** `repo.find().email` walks past the first hop because the external method's return type is known.
**Gap (re-scoped after tracing the pipeline — most of the infrastructure already exists):**
- TypeRef-derived return/field types: `build.rs:223-391` (full) + `augment.rs:121-209` (incremental/hydrated). The demand loop (`full.rs:743-790`) **does** augment the cached index with hydrated files via `augment.rs` before re-resolving, so hydrated externals *do* flow through the map-build. The earlier "demand-hydrated files don't enter the maps" claim was wrong.
- Signature-based return-type fallback already runs in **both** paths for *any* method lacking a TypeRef return type (`build.rs:362-374`, `augment.rs:185-200`) — not just .NET, despite the comments.
- **The real remaining hole is `parse_return_type_from_signature` (`chain_walker.rs:591`): it only recognizes the `): T` colon form** (TS methods, Kotlin `fun`). It does **not** handle Python/Rust `-> T` or TS-arrow `=> T`, so externals whose signature uses those forms get no return type. Secondary: external symbols must actually *carry* a signature for the fallback to fire, and the `r.module.is_some()` TypeRef filter (`build.rs:238`, `augment.rs:128`) drops module-tagged external TypeRefs (signature fallback sidesteps this).
**Fix (bounded first slice):** extend `parse_return_type_from_signature` to the `-> T` and `=> T` return forms (careful: load-bearing across all languages, gate behind thorough sibling tests + regression cases for the existing colon/generic forms). Then verify external method symbols carry signatures. Failing test first: `PrismaClient.findUnique().email` resolves the second hop; a Python `def f() -> User` external second hop.
**Deps:** EXT-3
**Corpus proof:** `PrismaClient.findUnique/findMany`, R `mutate/ggplot`, C `curl_easy_*`.

### EXT-3 — Lazy member hydration, origin-blind (D3/S4)  ·  [generic]  ·  ⚠️
**State:** `qualified_member_lookup` already resolves external members origin-blind via the global index (the `members.rs:68` `ext:` skip is not the blocker). Remaining value: generics carried through external chains, supertype walk over external bases. Reachability-bounded (resource-monopoly constraint), never whole-dep-tree.

### EXT-4 — Delete the eager seed (D5/S6)  ·  [generic]  ·  ❌
**Compiler feature:** load deps at the ref, lazily.
**Gap:** `seed_demand_from_user_refs` (`full.rs`) **still runs** — the seed-skip was gated and the gate is gone. ~4× reindex cost on dep-heavy projects.
**Fix:** once EXT-1/EXT-2 land, the at-the-ref demand-load covers it; delete the seed.
**Deps:** EXT-1, EXT-2

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

### QUAL-1 — Bar the whole grep family (D1/D9)  ·  [generic]+[hook]  ·  ⚠️
**Invariant:** never bind a bare name to a coincidental same-name symbol.
- **1a** `ranked_candidates` + `unique_internal_name` removed from default ladder — ✅ (`019fc8a7`).
- **1b** Remaining family still fires (~10.6k+ edges on ts-immich). **Full inventory in the Appendix** (strategy-classification pass over every `strategy:` literal across 45 language hooks + the generic resolver): **~38 grep + ~20 borderline** strategies across ~30 languages — the old short list (`*_by_name`, `default_same_file`, `engine_bare_same_file`, `rust_global_name_fallback`) was ~6× incomplete. Remove/justify each; a genuine same-file *module-scope* hit is legitimate, a whole-program by_name is not. **Several fire BEFORE the scope/import strategy** (elixir/erlang/scala/c synthetic-or-ext fallbacks + the generic `default_same_file`/`engine_bare_same_file` = the BIND-1 shadowing bug) — those actively mis-resolve, not merely over-resolve, so they are the priority within 1b.
**Deps:** BIND-1 (so removing same-file grep doesn't drop legitimate scope hits)

### QUAL-2 — Single engine, no per-language grep (D9)  ·  [generic]  ·  ⚠️
- **2a** reroute apparatus deleted — ✅ (`a3815b4e`).
- **2b** per-language hooks still run their own Step-5 grep fallbacks; consolidate resolution into one engine algorithm + data. Largest structural refactor; do after QUAL-1b.

### QUAL-3 — `*_synthetic_global` is import-blind external grep  ·  [generic]+[profile]  ·  ⚠️
**Resolved (was 🔬).** Audited the 17 `*_synthetic_global` strategies: they bind to **real** symbols parsed from real SDKs by real walkers (`dart_sdk.rs`, `flutter_sdk.rs` ~700 files, `ext:cpython-stdlib:`, `kotlin_stdlib`/`jdk_src`/`android_sdk`/maven sources jars). **Not** fabricated stubs — `ecosystem files are locators only` is honored; the "synthetic" name is a misnomer. The stub-crutch framing is dead.
**Real defect:** the binding is whole-program bare-name `by_name` with first-`ext:`-match-wins and **no import/scope check** (`<lang>/hooks.rs` `resolve_ref`; `by_name` is a flat map, `lookup_impl.rs:18`). That is Invariant #2's grep, filtered to `ext:` rows — real target, unjustified claim. `dart_synthetic_global` = 176k edges (55% of ts-immich) is mostly import-gated Flutter widgets (`ListTile`/`Widget`/`BuildContext`) bound by coincidence.
**Decision (Option A — honest floor):** **delete** the whole `*_synthetic_global` ext-bare-name family outright (the import-blind `by_name`-into-`ext:` grep). A bare external name now resolves only via a scope/import strategy, a curated prelude check, or stays unresolved — never by coincidental name match. This drops the import-gated coincidences (the ~176k Dart Flutter binds and equivalents) AND the true-globals (`print`/`len`) until per-language **prelude strategies** (curated, scope-directed — like `rust_prelude`) are rebuilt to recover the latter legitimately. The baseline will fall to the honest scope-directed floor first, then climb back on real binds. Kept: `rust_prelude`, `scala_implicit_import`, `php_global_function`, `ada_modular_primitive` (these are curated/qualified-name, not by_name grep).
**Deps:** QUAL-1b, BIND-2; QUAL-5 for the honest restatement.

### QUAL-4 — Collapse confidence to {resolved, unresolved} (D8)  ·  ⚠️
As grep goes, drop 0.x-confidence edges; a front-end answers resolved or error. Falls out of QUAL-1/QUAL-2.

### QUAL-5 — Three-state external model  ·  [generic]  ·  ❌
Distinguish `resolved_edge` / `external_known_unhydrated` (import names dep X, X's metadata absent) / `unresolved_unknown`. Makes precision measurable instead of hidden in a coverage rate. Pairs with EXT-1.

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

**Single best next move:** EXT-2 (external return types) — it is simultaneously the routing doc's D4/S5, the corpus #1 leverage, and the prerequisite for INFER-6 and most external chain resolution.

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
