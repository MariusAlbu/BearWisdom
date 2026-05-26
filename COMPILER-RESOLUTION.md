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

### BIND-1 — Lexical shadowing precedence  ·  [generic]  ·  ❌
**Compiler feature:** local shadows param shadows enclosing-member shadows import shadows global.
**Gap:** `default_resolver.rs:686` ladder runs `same_file`(#2) before `file_import`(#9) — a same-file symbol can beat an import that names the same thing.
**Fix:** import-binding must precede coincidental same-file; or gate `same_file` on "no import binds this name."
**Deps:** —

### BIND-2 — Complete import/module resolution  ·  [profile]+[generic]  ·  ⚠️
**Compiler feature:** resolve every import specifier to a module (alias, re-export, prefix, package entry, wildcard), then bind the imported name.
- **2a** SvelteKit `$lib`/`$app` — ✅ (`7fb058e7`).
- **2b** tsconfig `extends` chains — ❌ `parse_tsconfig_paths` does not follow `extends` (`npm.rs:147`); monorepo path-alias projects miss.
- **2c** Re-export / barrel chains in general — ⚠️ `reexport_chain` exists; verify depth + cross-package.
- **2d** Import-prefix → module binding (the Dart `i0.X` case, see LANG-DART-1b) — ❌.

### BIND-3 — Visibility / access-control awareness  ·  [generic]  ·  🔬
**Compiler feature:** won't bind `obj.privateMember` from outside its class.
**State:** visibility is a *ranking hint*, not a gate (`default_resolver.rs`).
**Decision needed:** a code-intelligence tool may *want* to over-resolve private members for navigation. Decide enforce-vs-ignore before building — this is a deliberate divergence, possibly a no-op.
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

### INFER-7 — Expected-type / bidirectional (roadmap D7/G1)  ·  [generic]  ·  ⚠️
**State:** turbofish + cast adoption landed (roadmap G1); LHS-annotation direction "subsumed for resolution, zero observable gain." Likely closeable as-is — confirm and mark done.

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
**Gap:** the resolve loop filters `ext:` refs (CLAUDE.md externals constraint); external method `return_type`/`field_type` maps aren't populated. `build.rs` derives the colon form for parsed externals, but demand-hydrated files don't reliably enter the type maps; the arrow form (`-> T`) was deleted with the gate.
**Fix:** extract + canonicalize external signatures across languages (colon **and** arrow), and ensure demand-hydrated external files land in the shared return/field maps.
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

### LANG-GO-1 — Implicit interface satisfaction + embedded promotion  ·  [hook]  ·  ❌
A type satisfies an interface by *having the methods* (structural — blocked on INFER-5); embedded-struct field/method promotion (verify whether `inherits_map` covers it).
**Deps:** INFER-5

### LANG-KOTLIN-1 — Extension functions + smart-casts  ·  [profile]+[hook]  ·  ❌
`x.ext()` resolved by receiver type to a top-level/member extension fn; smart-casts (roadmap L2 deferred — grammar-version-dependent); companion objects.

### LANG-CSHARP-1 — Extension-method search across usings  ·  [hook]  ·  ⚠️
Search extension methods across `using` directives (roadmap notes partial); partial classes; source-generated members.

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
- **1b** Remaining family still fires (~10.6k+ edges on ts-immich): `*_by_name` (`dart_by_name` 7.6k, kotlin/swift/c), `default_same_file`, `engine_bare_same_file`, `rust_global_name_fallback` (`rust_lang/hooks.rs:435`). Remove/justify each; a genuine same-file *module-scope* hit is legitimate, a whole-program by_name is not — draw that line per strategy.
**Deps:** BIND-1 (so removing same-file grep doesn't drop legitimate scope hits)

### QUAL-2 — Single engine, no per-language grep (D9)  ·  [generic]  ·  ⚠️
- **2a** reroute apparatus deleted — ✅ (`a3815b4e`).
- **2b** per-language hooks still run their own Step-5 grep fallbacks; consolidate resolution into one engine algorithm + data. Largest structural refactor; do after QUAL-1b.

### QUAL-3 — Audit `*_synthetic_global`  ·  🔬
**The single largest unexamined mass:** `dart_synthetic_global` = 176k edges (55% of ts-immich); `swift/kotlin/python_synthetic_global` also large. These prop up "engine %" for several languages.
**Decision needed:** are these legitimate SDK-global bindings or synthetic-stub crutches that violate `ecosystem files are locators only`? If crutches, they must be replaced by real indexed packages (EXT-2/EXT-5) and the headline numbers re-stated honestly.

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
2. **QUAL-3** — audit `dart_synthetic_global` (176k). Decides whether our headline numbers are honest; may redirect everything.
3. **QUAL-1b** — finish barring the grep family (with BIND-1 so legitimate scope hits survive). Completes the "never grep" invariant.
4. **INFER-1 (CFG)** — the foundational checker feature; unblocks the real reach of INFER-2/3/4.
5. **EXT-1 + EXT-4 + QUAL-5** — scope-directed routing, delete the seed, three-state metric (the clean redo of the deleted routing spine).
6. **LANG-RUST-1 / LANG-CPP-1 / LANG-GO-1** — the high-volume per-language method-resolution rules.
7. **EXT-5, CORPUS-1/2, INFER-2/3/4/5, remaining LANG-*** — breadth.
8. **DOC-1..4** — in passing / at closeout.

**Single best next move:** EXT-2 (external return types) — it is simultaneously the routing doc's D4/S5, the corpus #1 leverage, and the prerequisite for INFER-6 and most external chain resolution.
