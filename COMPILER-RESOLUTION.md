# Compiler-grade resolution — source of truth

**This document is the single source of truth for the resolution/type-inference
engine's path to compiler behavior.** It supersedes the status sections of
`RESOLUTION-EXTERNAL-ROUTING.md` (its §9/§10 describe the reroute apparatus that
was **deleted** in `a3815b4e`) and folds in the remaining items of
`TYPE-INFERENCE-ROADMAP.md`. When those disagree with this file, this file wins.

Established 2026-05-26; **restructured 2026-06-02** around the per-language 99%
gate and the one-engine architecture gate (below). Keep the **Done** table
current; move tasks into it as they land. Tag every task `[generic]` (one engine
algorithm) · `[profile]` (per-language data) · `[hook]` (per-language code) and
status `❌ not started` · `⚠️ partial` · `✅ done` · `🔬 decision needed`.

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

**Definition of done (the gate this roadmap is measured against):**

1. **Every supported language reaches ≥99% internal resolution precision** — per
   QUAL-5's three-state metric (`external_known_unhydrated` excluded from the
   denominator). 99% is the bar for *every* language, not the corpus average; a
   language below 99% is an open task, not a rounding error.
2. **Reached through the one generic engine.** A language hits 99% by routing onto
   the shared algorithm with its specifics expressed as `LanguageProfile` /
   `FlowConfig` / `ChainConfig` **data** — not by growing a bespoke per-language
   resolver. A % gain bought with a new per-language algorithm is a *regression*
   against this bar even when the number goes up.

**Architecture gate (hard rule — applies to every task below):**

- **No new per-language resolution algorithm.** New language behavior lands as
  (a) profile/config **data**, (b) a generic-engine capability all languages
  share, or (c) a *minimal* `LanguageEngineHooks` delta — in that order. A new
  `walk_<lang>_chain` is forbidden.
- **Net resolution code trends down, not up.** The bespoke chain walkers
  (~3,800 LOC, QUAL-2) are debt to retire onto `resolve_via_chain`, not a pattern
  to extend. Every per-language task states whether it *adds* engine code or
  *moves* behavior into shared data.
- **Generic-first decomposition.** Before any `LANG-*` task, reduce it to the
  smallest generic-engine change that closes it for all languages at once; only
  the irreducible remainder is a hook.

This reframes the remaining work. The corpus gap does **not** close by "writing
the missing per-language rules" — it closes by finishing the generic capabilities
(§B), retiring the per-language forks onto them (§F QUAL-2), and feeding each
language its data. Incremental per-language % that drifts from this gate is
explicitly rejected.

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
| QUAL-2b-ts | Retire `walk_typescript_chain` (480 LOC); TS on `resolve_via_chain(&TS_CHAIN_CONFIG)`; irreducible deltas as a `ChainExtensions` data-carrier (4 bools + 1 fn-pointer). −106 prod LOC | `58ad2b24` |
| QUAL-2b-csharp | Retire `walk_csharp_chain`; extension-method probe → `extension_method_fallback` bool + shared helper; 0 new fn-pointers. −175 | `329c5a51` |
| QUAL-2b-go | Retire `walk_go_chain`; embedded promotion via shared `walk_inheritance`; 0 new fn-pointers, `chain.rs` untouched. −190 | `c406434e` |
| QUAL-2b-java | Retire `walk_java_chain` + local `find_enclosing_class`; `namespace_lookup=WildcardOnly`; 0 new fn-pointers. −146 | `22c4adc7` |
| QUAL-2b (py/ruby/php/c) | Retire 4 walkers; PHP `root_type_access` bool; C typedefs via generic `expand_aliases` (no fn-pointer); `expand_current_type` rewrites without an env for non-generic aliases. −546 | `73a578b3` |
| QUAL-2b-rust | Retire `walk_rust_lang_chain` — the **9th** walker, initially missed by the batch; `RUST_CHAIN_CONFIG` reuses the existing `normalize_type` (`::`→`.`) + `walk_inheritance`; 0 new fn-pointers. −131 | `b2c20fa3` |
| INFER-5 | Structural assignability over the member table — shape-based, with matched-member param/return types compared and missing-type-info → `Unknown`. Unblocks LANG-GO-1 for project-internal interfaces. | `d7626b76` |

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
- **2c** Re-export / barrel chains (TS/JS) — ⚠️ multi-hop **works** (TS relative barrels recurse to depth 5, `typescript/aliases.rs:293`; generic external-npm BFS to depth 4 with cycle guard, `engine/index/classify.rs:110`). One gap left in TS itself: the **monorepo seam** — `project file → workspace package → external npm` falls between both walkers (workspace packages are never sources in `pkg_reexports`, `build.rs:660`; `follow_reexports` bails on bare specifiers, `aliases.rs:315`). The *cross-language* generalization is split out as **2e**.
- **2d** Import-prefix → module binding (the Dart `i0.X` case, see LANG-DART-1b) — ❌.
- **2e** **Generic re-export following across languages** — ❌ [generic]+[profile]. The 2c walker is **TS/JS-only**: `follow_reexports` (`typescript/aliases.rs:293`) has no non-TS caller; every other language's `reexports_from` impl is a test stub returning `&[]`. The *data* is already built language-agnostically — `build.rs:640` populates the reexport map from any `EdgeKind::Imports` ref carrying a module, so Rust `pub use foo::Bar` and Python `__init__` re-exports already populate it — but no consumer reads it for them, so a name imported through a re-export hop (`pub use`, `from .x import *`, Java static import, C++ using-decl) never follows to its declaring symbol. **Fix:** lift `follow_reexports` into a generic engine walk over `reexports_from`, gated by a `LanguageProfile` flag marking which `Imports` refs are re-exports (needs extractor tagging of Rust `pub use` vs `use`). High leverage for Rust facade crates / Python package APIs.

### BIND-3 — Visibility / access-control awareness  ·  [generic]  ·  ✅ (over-resolve)
**Compiler feature:** won't bind `obj.privateMember` from outside its class.
**State (corrected — NOT a uniform hint):** visibility is INCONSISTENT today. It is a **hard gate** in C#/Go/Java/Kotlin/PHP — `is_visible` filters out a `private` target whose file differs from the ref's (`csharp/hooks.rs:199`, `go/hooks.rs:197`, `java/hooks.rs:208`, `kotlin/hooks.rs:232`, `php/hooks.rs:222`). It is a **ranking hint** (`+50` public / `-200` private) only on the ranked-candidates path (`default_resolver.rs:901`). It is **ignored** in the chain walker (`chain.rs` reads visibility nowhere) and for TS/JS/Rust. Net: `obj.privateMember` from outside resolves in TS/JS/Rust and through any chain, but fails in the 5 gated languages.
**Done — over-resolve (deliberate, non-compiler divergence).** BearWisdom is a navigation tool; go-to-definition must reach private members. The audit undercounted — gating existed in **7** languages (C#/Go/Java/Kotlin/PHP **+ Rust + Scala**), all now `is_visible → true`, so visibility never blocks a bind and every language is consistently visibility-blind. This is intentional — do not re-add a visibility gate later thinking the inconsistency is a bug. Two resolution tests that asserted private-cross-file non-resolution were inverted; `java`/`php` analogues still pass because the bare private name isn't import-reachable (legitimate scope behavior, not a gate). The `+50/-200` ranking skew (`default_resolver.rs:901`) is left as-is — it can't block resolution, only mis-order disambiguation.
**Deps:** —

### BIND-4 — Overload disambiguation (arity + parameter type)  ·  [generic]  ·  ⚠️
**Compiler feature:** when N symbols share a name in scope, bind the call to the **one** whose signature matches the call's arity and argument types — not "a symbol of that name."
- **Member / method chains — ✅ generic, already landed (was untracked).** `members.rs:315 find_on_chain` selects `type_hit → arity_hit → first`; `dispatch.rs:93 select_multi_arg` + `most_specific_index` + `select_return_type` pick the most-specific overload by resolved arg types; wired through `chain.rs:388-440`. This is the correct generic shape — reuse it.
- **Bare-name function / static-method calls — ❌.** `foo(a,b)` with several same-name `foo` binds the **first** name+kind match at confidence 1.0 (`default_resolver.rs:463`, `resolve_via_scope_visible:480`); the `resolve_all` ladder has no arity/param strategy (only Erlang has an ad-hoc `*_arity` hook). **This violates Invariant #2 in spirit — it binds to *a* declaring symbol, often the wrong overload.**
**Fix [generic]:** when `by_name(target)`+kind yields ≥2 candidates, prune by `canonical_form::signature_arity == call_args.len()`, then `subtype::args_assignable` (both helpers already exist) — the bare-name analogue of `find_on_chain`. Folds in the per-language `erlang_*_arity` hooks. **LANG-CPP-1's "overload" clause depends on this; it does not own it.**
**Deps:** INFER-4 (arg types for the param-type tiebreak; arity-only pruning needs nothing).

---

## B. Type-checker breadth (generic inference — the spine)

These are what make the engine a *checker*, not a binder-with-generics. All
`[generic]` — each lifts every language at once. **This is the block that closes
the corpus gap without per-language code; finish it before any `LANG-*` work.**

### INFER-1 — Control-flow-graph-based narrowing  ·  [generic]  ·  ✅
**Compiler feature:** narrow across the whole flow graph — if/else merge, loop back-edges, reassignment *kills* a narrowing, `&&`/`||` short-circuit, exhaustiveness. Decision: **CFG-native** — consumers query the CFG directly via `fact_at(name, ref_byte)`; real union types at joins; the chain walker dispatches across `Union` members for member lookup.

**Landed (commits `afedb777` · `d45e21dd` · `402756cc` · `16940b8b` · `e405c157` · `c78db287` · `6bda447b` · `ab0d12fb`):**
- **Foundation** — `indexer/flow_cfg.rs` carries the per-function CFG: `BasicBlock { byte_range, defs }` + `Edge { from, to, guard: FactMap }` + `Cfg { blocks, edges, entry, in_facts }` + `FileCfg { functions }`. `Fact = Single(String) | Union(Vec<String>) | Never` with canonical-sorted unions; `FactMap::join` drops names absent on either side (a narrowing must hold on every reaching path) and unions disagreeing types. `run_dataflow` is a worklist forward pass with a **visited-bit** in `compute_in_fact` so loop back-edges don't pessimistically drop untouched names on the first pop. `fact_at` / `fact_string_at` give consumers the in-fact at a program point, with block-local defs applied up to the cursor.
- **Consumer wiring** — `FlowMeta.cfg: FileCfg`; `run_flow_queries` builds it (dispatch via `cfg_node_kinds_for` keyed by `strategy_prefix`); `LocalTypeCache.cfg` stores it; `LocalTypeCache::lookup` consults `fact_string_at` first, falling back to the interval `narrowings` vec when the CFG is absent or returns a `Union`/`Never` the consumer cannot yet represent. `SymbolLookup::install_local_cache` plumbed across the trait + impl + the `loop_body.rs:340` call site.
- **Four narrowing cases**:
  - **if/else join merge** — `build_if` emits the diamond with peeled `else_clause`; the join's in-fact is the pointwise merge of predecessors. TS-specific `condition_to_true_guard` recognizes `typeof` / `instanceof` shapes; **`&&`** composes both sides on the true edge, **`||`** unions them (producing real `Fact::Union` when the operands disagree on a name's type).
  - **loop back-edge fixed point** — `build_loop` emits `pred → header → body → header` + `header → exit`. A def in the body propagates back through the worklist; a use BEFORE the def textually but inside the loop is correctly NOT narrowed (the interval model misses this because it's byte-ordered). A body that doesn't touch the name preserves the narrowing through the fixed point.
  - **switch** — `build_switch` emits `pred → scrutinee → { case, ..., default } → exit`. Cases are disjoint blocks (no fall-through modeled). Post-switch join correctly drops a narrowing if any case reassigns the name.
  - **def-resets-fact** — generalizes the slice-1 interval truncation: a def inside a block kills any in-fact for that name from the def byte forward. Block-local in `fact_at`, and propagated through the worklist's per-block kill set.
- **Per-language `CfgNodeKinds` tables** — fourteen languages: TS · JS · Java · Python · C# · Rust · Go · C · PHP · Lua · Groovy · Scala · Kotlin · Ruby · R. Narrowings inherit per-language via `guards_for_range(meta.narrowings, body_range)` — every language whose `type_guard_query` lands narrowings on guarded scopes gets CFG-native narrowing through the shared pipeline, no per-language condition parsing.
- **Generic widenings to the kind table** that unblocked the harder grammars: `switch_kind`/`switch_case_kind`/`switch_default_kind` → slices and `switch_body_field` → `Option` (Go's `expression_switch_statement` + `type_switch_statement` differ structurally and have no body wrapper); `block_kind` → `block_kinds` slice (Ruby uses three: `body_statement` for the method body, `then` for if, `do` for while/until; R uses `braced_expression`); `transparent_kinds` slice for pass-through wrappers (Go's `statement_list`, Lua's `variable_declaration`, Kotlin's `function_body`); `find_function_body` descends one level through `transparent_kinds` so Kotlin's `function_declaration > function_body > block` is reached without a per-language hook.
- **Chain walker Union dispatch** (`ab0d12fb`) — the final loop close. `Cfg::fact_union_at` and `FileCfg::fact_union_at` project `Single → [s]`, `Union → branches`, `Never → None`. `LocalTypeCache::lookup_union` mirrors `lookup()` returning `Vec<String>`; `SymbolLookup::local_type_union` is the new trait method (default impl wraps `local_type` for backwards compat); the `LOCAL_TYPE_CACHE` thread-local overrides it. `DefaultRootResolver`'s Identifier branch now calls `local_type_union` and intern's a `Type::Union(member ids)` for the multi-branch case, returning that as the chain root. The existing Union arm in `core/members.rs:245` handles member lookup — every branch must carry the member (the safe semantic: a runtime value could land on any branch). The architect's "real union types at joins" decision is now observable end-to-end at the chain walker.
- **Tests**: 35 new tests (foundation Fact math, if/else, `&&`/`||`, loop back-edge, switch, def-kill, 12 per-language smoke tests — three narrowing assertions for Groovy/PHP/Ruby `instanceof`/`is_a?`, structural-only for Rust/Kotlin/Scala/C/Lua/R/Go/Java/C#/Python — plus two end-to-end Union dispatch tests in `core/chain_tests.rs`: `cfg_union_narrowing_resolves_member_present_on_every_branch` and `cfg_union_narrowing_drops_when_member_missing_on_a_branch`). **6022 bearwisdom lib tests, zero regressions**.

**Architectural decisions (resolved):**
1. **CFG data structure** — hand-rolled `Vec<BasicBlock>` + edge list (no petgraph dependency).
2. **Build location** — extract-time, serialized into `FlowMeta`.
3. **Fixed-point strategy** — worklist with `iter_cap=8` per block; visited-bit skip on unvisited preds; widen-by-drop on non-convergence.
4. **Coexist vs. replace** — ✅ replace; consumers query the CFG directly. Interval `Narrowing` is the fallback for languages without `CfgNodeKinds` and for `Union`/`Never` facts the consumer can't yet consume.
5. **Fact representation at joins** — real `Fact::Union` in the CFG (inline enum, not a type-arena variant — avoids touching the arena until the chain walker is ready to dispatch across unions).

**Closed-out items (no further INFER-1 work):**
- **Interval `Narrowing` vec retirement** — investigated 2026-05-28 and kept. `meta.narrowings` feeds both the CFG (as edge guards) and the interval fallback in `LocalTypeCache`; the interval path covers top-level narrowings outside any function body (Python `if isinstance(x, T):` at module scope is the canonical case) and non-CFG-native languages. Documented as a fallback rather than retired.

**Adjacent items that belong elsewhere (NOT INFER-1):**
- **Rust if-let narrowing** — needs ADT/payload lookup (`Option<T>` → `T`, `MyEnum::Variant(P)` → `P`). Now unblocked by the chain-walker Union dispatch consumer surface, but the actual feature lives in a future LANG-RUST-1 / type-system slice.
- **Go runtime `flow_config()` is `None`** (OOM workaround in `languages/go/mod.rs`) — `GO_CFG_KINDS` is wired and the type-switch path is structurally sound (proved with a synthesized-narrowing smoke test), but until the assignment_query OOM is fixed Go inherits no actual narrowings at indexing time. Belongs to a Go-investigation slice.
- **Go `discriminant_guard_query`** — empty in v1; expression-switch case discriminants would narrow the scrutinee per case. Belongs to LANG-GO-1.
- **Remaining ~10 languages without `CfgNodeKinds`** — Dart, Swift, the smaller-volume tree (Bash, Perl, Haskell, Elixir, Erlang, OCaml, Hare, …). Per-language tables, ~30–60 lines + smoke test each. Lower yield since most have no `type_guard_query`. Belongs to a long-tail expansion slice.

**Deps:** — (foundational; INFER-2/3/4 depend on this)

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

### INFER-4 — Expression-level type inference  ·  [generic]  ·  ⚠️ (blocked on a `CallArg` prerequisite)
**Compiler feature:** type an arbitrary RHS expression.
**Gap:** flow infers only call / literal / ident. Ternary, array/object-literal element types, `await`-unwrap, spread, index-access, binary ops all fall to `Type::Unknown` — and **cannot be typed today** because `CallArg` (`types.rs:435`) is a FLAT enum: extractors discard these sub-expressions to `CallArg::Other` before the type checker runs. The engine already returns `Unknown` for all of them (the sound state).
**Prerequisite (architect decision pending):** add recursive `CallArg` variants (`Ternary`/`ArrayLiteral`/`Await`/`Spread`/`IndexAccess`/`Binary` with `Box<CallArg>` children) + per-language extraction across ~10 `calls.rs`. Generic engine + legitimate per-language *extraction* (not resolution drift), but a real scope expansion. Object-literal-as-synthetic-`Class` was rejected as **unsound** (a memberless `Class` is more permissive than `Unknown` in `args_assignable`). Once the prerequisite lands, `resolve_arg_type` grows conservative recursive arms (Unknown over a guess).
**Deps:** the `CallArg`-recursive-variant prerequisite (also gates INFER-8). Verified 2026-06-02 (workflow `wmtkasf8r`).

### INFER-5 — Structural assignability / shape matching  ·  [generic]  ·  ✅ (`d7626b76`)
**Done.** `is_assignable_to` (`subtype.rs`) gains a shape-based positive arm over the member table (`MembersIndex` + `SymbolTypeMap`): a Class is structurally assignable to another when it carries every target member with a compatible kind AND assignable param/return types. Gated to (Class, Class), runs after the nominal arms, emits only Yes/Unknown (never No). **Soundness:** a missing member, an incompatible member type, or absent type data → Unknown, never a false Yes (a false "assignable" would mis-select overloads and mis-resolve Conditional aliases across all languages). Member-type recursion bounded at depth 4. **Coverage ceiling:** `MembersIndex` skips `ext:` files, so external interfaces (e.g. Go stdlib `io.Reader`) yield Unknown — this satisfies project-internal interface targets only; external structural satisfaction awaits hydration. Unblocks LANG-GO-1 (implicit interface satisfaction) for internal interfaces.
**Deps:** — (was the prerequisite for LANG-GO-1)

### INFER-6 — Built-in container generic semantics  ·  [generic]+externals  ·  ❌
**Compiler feature:** `Array<T>.pop():T`, `Map<K,V>.get():V`, `await Promise<T>:T`.
**Gap:** needs stdlib generic *signatures* — ties to EXT-2.
**Deps:** EXT-2

### INFER-7 — Turbofish + cast/assertion adoption  ·  [generic]  ·  ✅ (re-scoped)
**Done, for what it actually covers.** Turbofish consumed (`chain.rs:449`); cast/type-assertion adopted as root and mid-chain type (`chain.rs:322,363`; extractor emits it, `typescript/calls.rs:926`); the LHS annotation seeds `local_type` forward so any later chain rooted on the annotated var gets the declared type (`loop_body.rs:347`). The only unused slot is `expected_return` for overload selection (`DispatchQuery`, always `None` from `loop_body.rs:456`).
**Scope correction (was titled "expected-type / bidirectional" and marked closed):** that title overstated. Turbofish + cast adoption is the *explicit-annotation* expected-type path. The **dominant** bidirectional flow — an expected callback-parameter type seeding an un-annotated lambda parameter (`arr.map(x => x.foo)`) — is **not** built here; it is **INFER-9**. Read INFER-7 as closed only for the explicit cases, not as "bidirectional inference is done."

### INFER-8 — Argument-driven generic inference  ·  [generic]  ·  ❌
**Compiler feature:** infer a callee's own type parameter from an argument's static type — `identity(x)` where `fn identity<T>(x: T) -> T` binds `T` from `x`; `arr.map(f)` flows the element type through `f`'s signature.
**Gap:** the only three generic-binding sites (`chain.rs:454/483/828`) bind from inheritance args, turbofish, and receiver `Apply` args. **None unify a callee's declared `param_types` against resolved `arg_types`.** `dispatch.rs:230 resolve_arg_types` already computes the arg types but uses them only for overload *selection*, never `env.bind_positional`.
**Fix:** in `chain.rs`, alongside the turbofish branch, unify declared `param_types ⇄ resolved arg_types` and bind the resulting generic args before `yield_type_of`. Do **not** fold into INFER-7 (that is turbofish/cast only).
**Deps:** INFER-4 (arg expression types).  **Corpus leverage: high** — generic helpers, factory functions, `.map`/`.reduce` chains in every language with generics.

### INFER-9 — Contextual callback-parameter typing  ·  [generic]+[profile]  ·  ❌
**Compiler feature:** type an **un-annotated** lambda parameter from the expected callback-parameter type of the method it is passed to — `arr.map(x => x.foo)` types `x` as the array element. The single highest-frequency inference case in JS/TS/Kotlin/Swift/C#.
**Gap:** un-annotated lambda params resolve to `Type::Unknown` (`dispatch.rs:260`); lambda params get a type **only** from explicit annotations at extract time. No seeding from the expected signature; no profile field (`grep callback_param/expected_callback → 0`).
**Fix:** seed the lambda parameter's `local_type` from the resolved callback-parameter type of the enclosing call, gated by a `LanguageProfile` field declaring which stdlib higher-order methods carry element-typed callbacks (or, once INFER-6 lands, derive it from the container generic). This is the bidirectional half INFER-7's old title implied but never built.
**Deps:** INFER-8 (generic-arg binding, to know the element type), INFER-6 (container element types).  **Corpus leverage: high.**

### INFER-10 — Generic type-alias / typedef expansion in the shared walker  ·  [generic]  ·  ❌
**Compiler feature:** when a chain root or hop is a type alias, expand it to its target before member/return lookup, so `aliasVal.member` binds. Cross-language: Rust `type X = Y`, Go `type X = Y` / `type X Y`, C/C++ `typedef` / `using X = Y`, Scala type members, Kotlin `typealias`.
**Gap:** `expand_alias` is **consumed only by TS** (`typescript/chain_walker.rs:47`). C/C++ has a separate one-hop `dereference_typedef` hook (`c_lang/hooks.rs:445`). The **shared** `resolve_via_chain` (Go/Java/C#/Kotlin/Scala/Dart/Swift) never expands aliases — it looks up members on the alias *name*, which has none, so the walk stalls at the first `.member` after an alias-typed value. The *data* is already generic: `build.rs:605-634` synthesizes `AliasTarget::Application` for any `TypeAlias` with a `field_type`, populated for ~15 languages.
**Fix:** call `expand_alias` (or the `alias_target`/`field_type` deref) at root + each hop inside `resolve_via_chain`, unifying with the TS path and retiring the C hook — pure consumer-side generic change. **Prerequisite (small extractor slice):** Go emits no target `TypeRef` for `type Foo Bar` (`go/types.rs:217`) — mirror `rust_lang/extract.rs:355` (~10 lines + test) so the alias becomes visible to `expand_alias`.
**Deps:** — (rides on the consolidated walker, QUAL-2b).  **Corpus leverage: medium-high** — Rust/Go/Scala alias chains.

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

## D. Codegen / synthetic-symbol synthesis  ·  the third symbol source

The symbol table has three sources of declarations: parsed project source, decoded
dependencies (§C), and **symbols a generator/macro/preprocessor emits that never
appear in parsed text.** A compiler resolves against post-expansion symbols; today
this is N hardcoded one-offs with **no generic surface.**

### CODEGEN-1 — Generic post-extract symbol synthesis  ·  [hook]  ·  ❌
**Compiler feature:** a per-language recognizer emits synthetic declared symbols (and their member/return types) into the same table, so refs to generated members bind like any other symbol.
**Gap:** there is **no synthesis hook** on `LanguagePlugin` (only `extract` + `embedded_regions`). Every codegen path is hardcoded inside a language's `extract()` or an on-disk walker: Python `@dataclass → synthesize_dataclass_init` (`python/symbols.rs:684`), Svelte `$store` desugar (`svelte/hooks.rs:40`), C/C++ `#define` expansion (`c_lang/macro_catalog.rs` + `salvage_macro_expanded_decls`). These prove the shape; nothing generalizes it.
**Reference implementation (present, untracked):** C/C++ `#define` expansion is the most mature — `macro_catalog.rs` reads the project's own macro bodies from sibling/parent headers, honours token-paste `##` and stringify `#`, and `salvage_macro_expanded_decls` re-runs the declaration scanners over the substituted body (`c_lang/extract.rs:158`). The recognizer + re-scan shape is exactly what generalizes.
**Fix:** a `LanguageEngineHooks` method `synthesize_symbols(&self, file) -> Vec<ExtractedSymbol>` invoked post-extract, fed by per-language recognizers. Case space (all currently unresolved, all deterministically synthesizable **without running the real generator**):
- **Java/Kotlin Lombok** — `@Data`/`@Getter`/`@Builder` → `getX()`/`setX()`/`builder()` from field declarations. High frequency in real Java corpora; **no Java codegen path exists today.**
- **Rust derive / proc-macro** — `#[derive(...)]` impls, `bitflags!`/`prost`/`sqlx` accessors. Two on-disk walkers exist (`cargo_build_scripts.rs` indexes `OUT_DIR` only *after* `cargo build`; `cargo_expand_runtime.rs` shells `cargo expand`) but in-memory derive output is unreachable. Common derives are mechanically synthesizable without invoking the compiler — that belongs here.
- **C# source generators** — the `❌` half of LANG-CSHARP-1 is this same capability, not a C# quirk.
- **Generalizes** the existing Python/Svelte/C one-offs onto the shared hook.
**Cleanup (no-env-gate rule):** `cargo_expand_runtime.rs:43` gates expansion behind `BEARWISDOM_CARGO_EXPAND` — an env toggle, which the project rule forbids. Decide it in code (use always, or drop it) when CODEGEN-1 lands.
**Deps:** —  **Corpus leverage: high** — Lombok getters, derive impls, source-gen members are a large unresolved slice in Java/Rust/C#.

---

## E. Language-specific resolution rules

Per-language name/method-resolution a compiler hard-codes — expressed as **data on
the shared engine**, per the architecture gate. **A `LANG-*` task that adds a new
per-language walker is a gate violation; it must reduce to engine data + the
irreducible hook.** Ranked by corpus volume.

### LANG-RUST-1 — Trait method resolution  ·  [hook]  ·  ❌
Autoref/autoderef + trait-in-scope + which `impl Trait for T` provides `.foo()`; associated types; blanket impls. Confirmed absent in `rust_lang` — `.method()` is name-only. The defining Rust compiler feature.

### LANG-CPP-1 — ADL (+ overload via BIND-4; template-param binding ✅)  ·  [profile]+[hook]  ·  ⚠️
**Template-parameter binding — ✅ (was mis-marked ❌).** `templates.rs:84-93` folds template param names into the signature (`<T,charT>`) feeding the generic-param strategy; `default_resolver.rs:526` resolves self-declared params as `engine_generic_param` (passing test `default_resolver_tests.rs:1062` resolves `OutputIt`). `T1`/`T2`/`OutputIt`/`iterator_t`/`charT` are no longer the gap.
**Remaining — ⚠️:** **overload resolution** (genuinely absent — `c_lang/hooks.rs:380` on ambiguity just picks the first @0.95) → **owned by the generic BIND-4, not here**; **ADL** (argument-dependent lookup) → the only true C++-specific `[hook]` remainder. The C/C++ `#define` leg of CODEGEN-1 also lands under this language.

### LANG-GO-1 — Implicit interface satisfaction + embedded promotion  ·  [hook]  ·  ⚠️
- **Embedded promotion** — ✅ chained `t.BaseMethod()` now resolves. Two fixes were needed (the earlier "inherits_map populated" claim was wrong for full builds): both `inherits_map` child-kind filters excluded `Struct` (only Class/Interface/Trait), so Go struct embedding never populated the map — `build.rs` and `augment.rs` now include `Struct`; and `walk_go_chain` Phase 3 now climbs via `find_member_via_inheritance` (strategy `go_chain_inheritance`, conf 0.90). Bare-name promoted refs from inside the struct already resolved via `resolve_via_enclosing_member`.
- **Implicit interface satisfaction** — ❌ structural ("has the methods"), blocked on INFER-5.
**Deps:** INFER-5 (interface-satisfaction half only; embedded-promotion fix is independent)

### LANG-KOTLIN-1 — Extension functions + smart-casts  ·  [profile]+[hook]  ·  ❌
`x.ext()` resolved by receiver type to a top-level/member extension fn; smart-casts (roadmap L2 deferred — grammar-version-dependent); companion objects.
**Scoped (why it's not a C#-style quick slice):** for `fun String.shout()` the extractor emits the receiver `String` as a **TypeRef** (`kotlin/coverage_tests.rs:464`), and the symbol qname is just `shout` — so Phase 3's `current_type.last` lookup misses. Unlike C# (where `signature_is_extension_on` reads `this <recv>` from the signature inside the dedicated `walk_csharp_chain`), Kotlin (a) routes through the **shared** `chain::resolve_via_chain` and (b) has no signature marker — the receiver is a position-dependent TypeRef indistinguishable from a first-param type. Clean fix needs extractor-level receiver tagging (a dedicated field / `extension_receiver` map) + a config-gated Phase-3 fallback in the shared walker (reusable for Scala). Multi-part.

### LANG-CSHARP-1 — Extension-method search across usings  ·  [hook]  ·  ⚠️
**Extension methods** — ✅ `walk_csharp_chain` Phase 3 now, after instance-member lookups miss, searches `by_name(method)` for a static method whose signature's first parameter is `this <current_type>` (`signature_is_extension_on`, strategy `csharp_extension_method`, conf 0.85). The `this`-param is read straight from the stored signature text — no extract-time tag needed. Visibility-blind (no `using`-scope gate), consistent with the over-resolve model (BIND-3). **Partial classes** — ✅ work incidentally: `members_by_parent` accumulates members under the shared parent qname across files (`build.rs:153`). **Source-generated members** — ❌ → this is **CODEGEN-1**, not a C#-specific fix; do not build a bespoke C# walker for it.

### LANG-PY-1 — MRO  ·  [hook]  ·  ❌
Method resolution order for multiple inheritance. (Dynamic `__getattr__`/duck typing out of scope.)

### LANG-SWIFT-1 — Protocol extensions  ·  [hook]  ·  ❌
Protocol extensions, retroactive conformance, `some`/`any`.

### LANG-TS-1 — Remaining type-level operations  ·  [profile]  ·  ⚠️
Structural matching (INFER-5); conditional-beyond-decidable; mapped-beyond-transparent (roadmap TS2 — rare); template-literal types; `satisfies`; `infer`; declaration merging / module augmentation. Mostly low corpus value — keep deprioritized.

### LANG-DART-1 — Dart binding  ·  [hook]  ·  ⚠️
- **1a** bare prefix-ref drop — ✅ (`4be1cb83`).
- **1b** prefix → library binding — ❌ `i0.Value` should resolve `Value` in `i0`'s module (drift external) or the local file (`i2.X`). `collect_dart_import_aliases` (`dart/extract.rs:84`) captures only the alias and *drops the URI* — needs a prefix→module map (BIND-2d). The lone remaining Dart gap.
- **1c** Flutter SDK externals — ✅ (was mis-marked ❌). `FlutterSdkEcosystem` (`ecosystem/flutter_sdk.rs`) walks `packages/flutter/lib/src`; `demand_pre_pull` surfaces `ListTile`/`IconData`/`EdgeInsets` (`flutter_sdk.rs:77`); registered + tested (`flutter_sdk_tests.rs`).

### Deferred per-language (recorded, low value)
- Kotlin smart-cast / Rust `if let`/`match` discriminant (roadmap L2/L3) — grammar-uncertain.
- Haskell generic-param bounds (roadmap L1) — params outside any bracket clause, near-zero payoff.

---

## F. Correctness, confidence — and the one-engine consolidation

### QUAL-1 — Bar the whole grep family (D1/D9)  ·  [generic]+[hook]  ·  ⚠️ (H.1+H.2 ✅, H.3 remains)
**Invariant:** never bind a bare name to a coincidental same-name symbol.
- **1a** `ranked_candidates` + `unique_internal_name` removed from default ladder — ✅ (`019fc8a7`).
- **1b** ✅ for the two grep buckets: the ordering bugs are fixed (Appendix), **H.2** (internal whole-program `by_name` — `{c,kotlin,swift,dart,elixir}_by_name`, `rust_global_name_*`, …) deleted in `c4252e8b`, and **H.1** (the `*_synthetic_global` ext-bare-name family) deleted in `bf46353b` (= QUAL-3). Verified: 0 occurrences of any of those strategy literals remain. **Remaining = H.3 only** (~20 borderline: same-file/module fallbacks whose risk is ladder order, or `by_name` scoped by a coarse path/prefix rather than a qualified-name lookup). Each needs reorder-after-imports or a tighter filter — none is a whole-program bare grep, so this is precision-tuning, not invariant-breaking.
**Deps:** BIND-1 ✅ (so removing same-file grep doesn't drop legitimate scope hits)

### QUAL-2 — One generic chain walker, all languages on it (D9)  ·  [generic]  ·  ✅ — **the central architectural deliverable, done**
**This task decided whether BearWisdom is "one engine + data" or "94 resolvers."** Done: all **9** bespoke `MemberChain` walkers are retired onto `resolve_via_chain(&<LANG>_CHAIN_CONFIG)`.
- **2a** reroute apparatus deleted — ✅ (`a3815b4e`).
- **2b — retire the bespoke chain walkers — ✅.** The 9 forks (`walk_typescript_chain` 480L, `walk_csharp_chain`, `walk_go_chain`, `walk_java_chain`, `walk_python_chain`, `walk_ruby_chain`, `walk_php_chain`, `walk_c_lang_chain`, `walk_rust_lang_chain`) are deleted; each language is now `<LANG>_CHAIN_CONFIG` **data** on the shared walker. The per-language deltas reduced to `ChainExtensions` bools (`expand_aliases` / `walk_inheritance` / `promote_external_qname` / `root_construction` / `extension_method_fallback` / `root_type_access`) — **one `ChainExtensions` fn-pointer total** (`root_fallback`, the npm-globals ambient probe; `normalize_type` is a separate pre-existing `ChainConfig` fn-pointer reused by C and Rust for `::`→`.`). Net **−1,294 production LOC**; **6,100** lib tests, zero regressions across all 9 slices. Commits `58ad2b24` · `329c5a51` · `c406434e` · `22c4adc7` · `73a578b3` · `b2c20fa3`. The 5 languages already on the generic walker (Dart/Kotlin/Scala/Swift/Starlark) stay on `ChainExtensions::NONE`. **14 languages now share one chain algorithm.** Migrating onto the shared ladder is widening-only for the weaker forks (deeper inheritance climb, `by_name`-prefix-unique 1.0 hop, `members_of`-final) — fires after base lookups miss, kind-gated; corpus-delta deferred to the DOC-4 recapture.
  **Honesty note:** the py/ruby/php/c batch under-counted — it migrated those 4 and reported "4 remaining" when Rust was still a fork, so QUAL-2 was briefly mis-marked complete at 8. `walk_rust_lang_chain` was then caught (a misused `bw_grep` without `regex=true` first gave a false "gone") and migrated as the 9th. The 9-count is now verified.
- **Ada is NOT a chain-walker fork.** Beyond the 9, the only remaining `walk_*_chain` is Ada's `walk_field_chain` — verified NOT a 10th chain walker: Ada's extractor emits **no `MemberChain`s** (`chain: None` on every ref; no `ada/` file among `chain: Some(...)` emitters), so `resolve_via_chain` (requires ≥2 segments) resolves nothing for Ada. It is a field-resolution helper *inside* Ada's flat-dotted-name resolver (`AdaResolver::resolve`). Forcing it on resolves nothing AND would need ~7 Ada-only flags (a gate violation). **Out of QUAL-2b scope.** Whether Ada's name resolver (and other languages' name-resolution tails) consolidate onto a shared NAME-resolution path is a separate future question.
- **2c — Step-5 by-name fallback tail** — largely folded into the migrations; any residue is QUAL-1 H.3 reorder precision-tuning.
**Outcome:** every `LANG-*` rule (Rust traits, Go interface satisfaction, Kotlin extensions, INFER-10 alias expansion) now lands as a `ChainConfig` / `ChainExtensions` delta on the *one* walker — the gate is enforceable going forward.

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

## G. Corpus-driven gaps (volume the above doesn't directly cover)

### CORPUS-1 — Component-in-template binding  ·  [profile]  ·  ⚠️ (HTML only)
Svelte (LANG-SVELTE-1 + BIND-2a), **Vue** (incl. vuetify `VCol`/`VBtn` via `vue/global_registry.rs:111`), **Astro** (`astro/extract.rs:116`), **Angular** (`angular/extract.rs:120`), **MDX** (`mdx/extract.rs:5`) — all ✅ (were mis-listed as pending). **Remaining: plain HTML custom-element tags** — `html/extract.rs:18` resolves script-block calls only, no PascalCase / custom-element component binding.

### CORPUS-2 — DSL / markup resolvers below the 99% gate  ·  [profile]+[hook]  ·  ❌
Languages whose resolvers are weak or absent and so fail the per-language 99% bar: **gsp** (16% — `gsp/` has only `profile.rs`+`mod.rs`, **no resolver at all**), **bicep** (44%), **vbnet** (46%), **matlab** (62%), **html** (66%), **prolog** (74%), **r** (76%). (`astro` dropped — its 0% was stale; `astro/hooks.rs:26`'s `build_file_context` fix already resolved it.) Each must reach 99% through the shared engine + its profile data, not a bespoke walker. Numbers are pre-recapture (DOC-4) and indicative.

---

## H. Hygiene / consolidation

### DOC-1 — Retire `RESOLUTION-EXTERNAL-ROUTING.md`  ·  ✅
Deleted 2026-05-26; its north star (D1–D9) is folded into §A/§C/§F here.

### DOC-2 — Retire `TYPE-INFERENCE-ROADMAP.md`  ·  ✅
Deleted 2026-05-26 (along with `RESOLUTION-GOAL.md`, `RESOLUTION-TASKS.md`,
`GENERIC-PARAM-UNIFICATION.md`, `baseline-gaps.md`, `baseline-by-so2025.md`).
Remaining roadmap items folded into LANG-* / INFER-* here; R1 is moot (the gate
and arrow-return path were deleted in `a3815b4e`).

### DOC-3 — Close residual long-tail  ·  ❌
13 `$t` in plain `.ts` (outside the SFC splice path); small `$lib` edge cases (`keyboard`/`imageLoader`/`useLogger`).

### DOC-4 — Full-corpus recapture → the per-language 99% scoreboard  ·  ❌
`baseline-all.json` (`captured_at` 2026-05-27) predates this restructure and lacks the QUAL-5 three-state fields. One recapture **at initiative closeout** (per the one-recapture rule — *not* mid-refactor), reporting QUAL-5's three states separately and **per language against the 99% gate**. This recapture is what turns "Definition of done" from a target into a checked scoreboard; until then per-language % is honest-floor noise.

---

## Sequence — architecture-first, toward 99% per language through one engine

The ordering rule changed. The corpus gap does **not** close by writing per-language
rules; it closes by finishing the generic capabilities and **moving every language
onto them**. Per-language % is a *consequence* of that, measured once at closeout (DOC-4).

1. **QUAL-2b — consolidate the chain walkers — ✅ DONE.** All **9** `MemberChain` walkers retired onto `resolve_via_chain` (Ada is a name resolver, not a chain fork — out of scope). **14 languages** on one algorithm, **−1,294** production LOC, 1 fn-pointer, 0 regressions. The gate is enforced; everything below now lands as data/capability on the survivor.
2. **§B generic inference.** **INFER-5 ✅** (structural assignability, `d7626b76` — unblocks LANG-GO-1 for project-internal interfaces). **INFER-4 / 8 / 3 / 2 (the expression-typing track) are blocked on a prerequisite:** `CallArg` is a flat enum, so ternary / array / spread / index / binary / await sub-expressions are discarded at extraction — typing them needs recursive `CallArg` variants + per-language extraction (~10 `calls.rs`). Legitimate extraction work, but a scope expansion → **architect decision pending.** **INFER-10** (alias expansion) rides the consolidated walker and is CallArg-independent — do it next. (INFER-9 callback typing partly depends on the same prerequisite.)
3. **CODEGEN-1 — the third symbol source.** Lombok / derive / source-gen are a large unresolved slice; one generic hook + per-language recognizers.
4. **EXT-2 finish + EXT-5 + INFER-6** — external return types, framework ambient globals, container generics (each gated on hydration).
5. **BIND-2e (generic re-export) + BIND-4 (bare-name overload)** — generic binder completeness across languages.
6. **`LANG-*` as data on the survivor walker** — Rust traits, Go interface satisfaction, Kotlin/Scala extensions, Py MRO, Swift protocols (+ Swift's missing `swift_import` strategy), CORPUS-1 HTML, CORPUS-2 DSLs. By now each is a `ChainConfig`/hook/profile delta, not a fork.
7. **DOC-3 long-tail; DOC-4 closeout recapture** — the per-language 99% scoreboard.

**State (2026-06-02):** foundations done — externals (EXT-1 / EXT-2 slice-1 / EXT-3 / EXT-4), three-state metric (QUAL-5), CFG narrowing with end-to-end `Union` dispatch across 14 languages (INFER-1), binder shadowing (BIND-1), visibility-blind over-resolve (BIND-3). **QUAL-2 ✅** — all **9** chain-walker forks retired onto `resolve_via_chain`: **14 languages on one algorithm, −1,294 production LOC, 1 fn-pointer, 6,100 lib tests green, 0 regressions** (commits `58ad2b24` · `329c5a51` · `c406434e` · `22c4adc7` · `73a578b3` · `b2c20fa3`). The "one engine" invariant is now **true** for chain resolution. **INFER-5 ✅** (`d7626b76`). **Decision pending:** INFER-4/8/3/2 (expression typing) need a recursive-`CallArg` + per-language-extraction prerequisite — expression sub-structure is discarded at extract time. Newly tracked after the 2026-06-02 audit: **INFER-8/9/10, BIND-2e, BIND-4, CODEGEN-1**; re-scoped **INFER-7, LANG-CPP-1**; corrected **LANG-DART-1c ✅, CORPUS-1 (HTML only), CORPUS-2 (astro dropped)**. Remaining: INFER-10 (next), the CallArg prerequisite + INFER-4/8/3/2, INFER-9, CODEGEN-1, BIND-2e/4, `LANG-*` as data, DOC-4 closeout recapture.

**Single best next move:** **INFER-4 — expression-level type inference** (ternary, array/object-literal element types, `await`-unwrap, spread, index-access, binary ops; `dispatch.rs:241` currently maps these to `Type::Unknown`). It is the root of the inference spine: INFER-8 (arg-driven generics), INFER-9 (callback-param typing) and INFER-3 (body-return inference) all consume typed expressions, and INFER-2 (interprocedural) carries the results across files. All `[generic]` — they lift every one of the 13 consolidated languages at once with zero per-language code, which is exactly the leverage the consolidation unlocked. (QUAL-2b, the prior single-best-move, is ✅ done.)

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
