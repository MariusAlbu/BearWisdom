# Gap-closure master plan — 2026-06-12

Synthesis of six parallel planning tracks over the category analysis (`UNRESOLVED-CATEGORIES-2026-06-12.md`). Population: 1,280,723 app-class unresolved refs, baseline 86.94% (`CORPUS-2026-06-11.md`). Target: ≥99% per language through the one generic engine; documented-borderline is the principled escape hatch. Every root cause below was verified by the planning agents against live post-recapture DBs and the engine source — not inferred from the sweep alone.

Tags: `[generic]` one engine change · `[profile data]` LanguageProfile data · `[hook]` LanguageEngineHooks last resort · `[extractor]` per-language extraction fix · `[locator]` ecosystem locator (locator-only contract).

---

## 0. Discoveries that re-rank the work

The planning pass falsified four assumptions from the category analysis. These re-rank everything:

1. **SIMD (75k refs) is an extraction kind bug, not an include-graph problem.** `typedef __attribute__((neon_vector_type(4))) int32_t int32x4_t;` parses as a plain `declaration` (the attribute pushes it off the `type_definition` path), so `int32x4_t` is indexed as `kind=variable`. The C profile's existing dead-last `resolve_via_namespaceless_global` rung (NamespaceScope::Global) would bind all 74,860 zig refs — the kind table excludes `variable` from `TypeRef` targets. A real include-edge graph was evaluated and **rejected**: `arm_mve.h` (4,770 refs) doesn't transitively include `arm_vector_types.h` — strict include tracking would FAIL where global lookup succeeds.
2. **The TS monorepo leak (≈50–100k refs) is import-extraction starvation, not resolution.** Multi-line `import type { ... } from "@calcom/types/Calendar"` blocks emit **zero** import-binding refs; 5,686 of ~8,000 calcom internal TS files captured 0 imports. `resolve_via_workspace_package` exists, runs 2nd in the ladder, and is correct — it just never sees the binding. Compounding: `/../`-bearing duplicate file paths (1,562 calcom files) with NULL package_id.
3. **External type-info maps are already hydrated.** `SymbolIndex::build_with_context_and_arena` runs all type-metadata passes (field_type/return_type/alias_target/generic_params/inherits) with NO `ext:` filter; the filter at `loop_body.rs:368` only stops external files emitting edges. The CLAUDE.md framing ("return-type maps not hydrated") is stale. The rate-bearing gaps are four narrow binding mechanisms (§E below). Also: `external_known_unhydrated` (8.36M) is already rate-neutral — converting it to edges moves nothing.
4. **Lua stdlib is already in the index** (`floor → math.floor` at `ext:lua-stdlib:`, the locator fired). The gap is generic ambient-namespace admission + `local floor = math.floor` alias binding. MATLAB's locator is correct but there is no install on the index machine — install-conditional borderline, NOT missing code. The historical `matlab_stdlib.rs` synthetic list is confirmed deleted; nothing reopens it.

Plus two latent wins found in the engine: `resolve_via_ranked_candidates` (default_resolver.rs:2632) is **fully built but unwired** — it is exactly the overload-set ranking Pascal needs; and Go anonymous-struct fields are **already indexed** under enclosing-function qnames (`TestFindOTPById.expectError`) — only range-element flow typing is missing.

Two rule violations found in passing: `BEARWISDOM_QT_DIR` (qt_runtime.rs) and `BEARWISDOM_MATLAB_ROOT` (matlab_runtime.rs) env probes — decision 10 below.

---

## 1. ARCHITECT DECISION MENU (blocking — one review session)

| # | Decision | Recommendation | Blocks |
|---|---|---|---|
| 1 | **Bicep ARM spec as vendored profile data** — pin `{System,Az}NamespaceType.cs` (or distilled JSON) at a tagged Bicep release; existing parser (`extract_function_names`) reused; in-tree clone still takes precedence | **Approve.** Finite (~115 names), versioned, spec-defined; names extracted from a versioned upstream artifact, not authored | F-B |
| 2 | **zig-compiler vendored-subtree reclassification** — treat `lib/include/` + `lib/libtsan/` (clang headers, compiler-rt) as vendored toolchain via `vendored_self_declared`-style recognition | **Do BOTH:** ship D-M1 (fixes a real bug benefiting all C projects) AND reclassify (attribution honesty). ts-nextjs precedent | D-track accounting |
| 3 | **MATLAB on the recapture machine** — install MATLAB base (or point the locator at a `toolbox/` tree) vs document platemo borderline | Install if cheap (~0.55pt conditional); else borderline. Zero code either way | F-M |
| 4 | **Dynamic-receiver depth (Lua/JS)** — generic engine binds only flow-typed receivers (`local x = M.new(); x:method()`); untyped multi-candidate `self:emit_signal()` and Bookshelf-through-dynamic-require stay borderline | **Approve the line as drawn.** Binding would be guessing; corrupts dead-code/diagnostics | A-M6 |
| 5 | **Qt boundary reconciliation** — Qt *types/functions* (`QString`, `tr`) = externals track via `qt_runtime` locator walking real Qt headers; Qt *attribute-position macro noise* (`Q_OBJECT` as type_ref) = native-C structural suppression; Qt *function-like macro calls* (`QCOMPARE`/`QVERIFY`/`SIGNAL`/`SLOT`) = bindable only if `qt_runtime` admits the `#define` symbols, else borderline | Approve the three-way split; E-M5 and D-M2 implement their halves | E-M5, D-M2 |
| 6 | **Vendor policy generalization** — extend self-declaring-manifest vendor recognition (rebar `deps/`, vendored go/composer) corpus-wide; never a path-segment name list | Approve V1 (rebar) now; V2 generalization as follow-up policy | V1/V2 |
| 7 | **`multi_candidate_ranking` opt-in scope** — profile-gated, Pascal first; default `false` everywhere (byte-identical for the 14 ≥99 languages) | Approve; revisit per-language only after Pascal recapture proves the margin logic | A-M2 |
| 8 | **Java `LOG`-as-type_ref fix locus** — extractor emits correct edge kind vs widening the TypeRef kind table | **Extractor fix** (removes the artifact; table-widen masks it) | B-M5 |
| 9 | **Template borderline calls** — Liquibase/Grails builder DSL (`column`/`changeSet`, runtime methodMissing), nix module-system arg reads, blade/ejs/handlebars framework helpers, ember `svg-jar` | Document all as borderline; GSP standard `g:` taglibs as profile data (Robot precedent); scss mixins + html script-link + MDX/astro components get connectors (targets are in-index) | T-track |
| 10 | **Env probes in locators** (`BEARWISDOM_QT_DIR`, `BEARWISDOM_MATLAB_ROOT`) | Decide: discovery *hints* (tolerable, they locate — don't gate behavior) vs remove per no-env-var-gates. Recommend: replace with manifest/standard-path discovery where possible; never let one gate resolution behavior | hygiene |
| 11 | **Externals hydration depth** — reachability extends only along project-touched import/re-export edges, capped by existing MAX_TRANSITIVE_PASSES/MAX_ALIAS_HOPS; framework entry packages get one bounded extra re-export hop (to reach `test_api`/`matcher` leaves) | Approve, measured on serverpod first | E-M3 |
| 12 | **Pascal include-group boundary** — `ModuleScope::SameDir` (cheap; castle's `.inc` siblings are co-directory) vs true `{$I}` include-graph edge | Try SameDir first; escalate only if recapture shows over-binding | A-M1 |

---

## 2. Track plans (condensed; full agent plans preserved below in source control history)

### Track A — Call-link gaps with in-index candidate (386,665 refs · ceiling ~3.9pt · realistic ~1.5–2.0pt · ~9–11.5 sessions)

Engine reality check: the live walker is `type_checker/core/chain.rs` (TypeId-based); `type_checker/chain.rs` is legacy — do not target it. The ladder is `core/default_resolver.rs::run_ladder` (~25 strategies).

| MS | What | Where | Tag | Impact |
|---|---|---|---|---|
| Prep | Add inert profile axes (`multi_candidate_ranking`, `scope_functions`, ModuleScope variant) to DEFAULT_PROFILE in one commit | `profile/language_profile.rs` | [profile data] | enables A-M1/2/3 |
| A-M1 | Pascal `.inc` include-scope: same-unit module scope (SameDir first, per decision 12). `TCastleColor` (1 candidate, still failing) is the acceptance case | `resolve_via_module_scope` + pascal include-edge emission | [profile data]+[generic] | ~0.1–0.15pt |
| A-M2 | **Wire `resolve_via_ranked_candidates` into `run_ladder`** (dead-last, profile-gated), pre-filtered by `dispatch::arg_assignable_candidates` for arity. Unblocks `Vector3` (19 candidates), `WritelnWarning` (8), `TVector3` type-as-call | `default_resolver.rs:2440` region | [generic], gated | ~0.6–0.8pt — largest single lever |
| A-M3 | Kotlin generic-builder Apply-carry: re-bind `Apply` args from returned type mid-chain; scope functions (`apply/also`→receiver, `let/run`→lambda yield) as profile data | `core/chain.rs::yield_type_of`/`bind_apply_args` | [generic]+[profile data] | ~0.35pt |
| A-M4 | Rust SelfRef→enclosing-impl typing + impl-member keying with empty scope_path (`find_position_of`, 1 candidate, fails today); `clone` only where receiver narrows — rest borderline | `core/chain.rs` DefaultRootResolver + MembersIndex | [generic] | ~0.2–0.3pt |
| A-M5 | Haskell where-bound locals + operator defs — extractor scope_path emission; resolution already works via `resolve_via_scope_visible` | `haskell/extract.rs` | [generic, extractor] | ~0.2–0.25pt |
| A-M6 | Lua/JS flow-typed receivers only (decision 4); untyped dynamic dispatch documented borderline | `core/chain.rs` flow | [generic]+[hook]/borderline | ~0.1–0.2pt |

Order: Prep → A-M1 → A-M2 (critical path, ~4 sessions, banks the Pascal lever). A-M3/M4/M6 share `core/chain.rs` — serialize ownership (M4 → M3 → M6). A-M5 parallel.

### Track B — Type-link + workspace/barrel binding (196,262 + 18,188 component tags · ~4.5–5.5 sessions)

| MS | What | Where | Tag | Impact |
|---|---|---|---|---|
| B-M1 | **Import-binding survival**: multi-line `import type {…}` emits zero refs. Diagnose locus with a unit test (extraction vs consume/strip), fix emission so `build_file_context_inner` sees the bindings. The ladder strategies are already correct and waiting | `ecosystem/ecmascript_imports.rs` (`emit_clause_refs`/`push_import_refs`), `typescript/hooks.rs:502` | [generic] | ~0.5–1.0pt — biggest B lever |
| B-M2 | Path normalization: fold `..`/`.` at extraction so `facade/../service/X.ts` never creates duplicate NULL-package files (lexical only, never symlink) | indexer import→path writer | [generic] | ~0.05–0.1pt + compounds B-M1 |
| B-M3 | Dart cross-package `package:` attribution (appflowy PB types, serverpod) — same starvation shape as B-M1 | `dart/hooks.rs` + dart import layer | [generic] | ~0.2–0.3pt |
| B-M4 | Svelte component tags: mostly FREE after B-M1 (svelte `<script>` uses the TS context builder; `resolve_via_component_import` + `follow_reexports` + `$lib` alias already exist). Verify alias table population | `js_config_aliases.rs` if alias missing | [generic]/[profile data] | ~0.12pt |
| B-M5 | Kind-compat surgical: Java `LOG` (extractor fix per decision 8), Pascal inherits kind check | `java/extract.rs`, `pascal/predicates.rs` | [extractor]/[profile data] | ~0.02pt |
| B-M6 | Vue auto-import: populate registry from project `components.d.ts`/plugin config — config-driven, never a hand list; no config → borderline | `vue/global_registry.rs` | [profile data] | ~0.08pt |

Guardrail fixture: locally-defined `Foo` must still beat an imported `Foo` (workspace strategy runs before scope rungs).

### Track E — Externals binding gaps (138,877 + test-DSL externals · ~6.5–9 sessions)

Premise correction: hydration exists; the work is consumption-side. No change to the `ext:` edge filter. Hard perf rule: nothing enters the resolve hot loop that isn't an O(1) map probe (aspnetcore resolve ≈25min).

| MS | What | Where | Tag | Impact |
|---|---|---|---|---|
| E-M1 | `constructors_are_callable` profile flag: `Calls` accepts `enum_member`/constructor targets for Haskell/OCaml/F#/Elm (pandoc `Str`/`Div`/`Para`) | kind-compat + profile flag | [generic]+[profile data] | haskell ADT calls |
| E-M2 | Injected globals as profile data (AliasDecode precedent): `(scope, bare_name) → fqn` from parsed `:refer` vectors / on-disk declared globals; consulted before bare internal lookup | new profile-data struct | [profile data] | clojure ~6k + busted |
| E-M3 | Reachability leaf-walk: follow framework entry-package re-exports to the defining leaf (`package:test` → `test_api`/`matcher` → top-level `expect` FUNCTION). Bounded per decision 11 | `ecosystem/pub_pkg/reachability.rs` + hex/npm analogues | [generic, locator] | dart `expect` ~12k — biggest E lever |
| E-M4 | External-hop chain continuation: qname-keying alignment (user `PrismaClient` vs `.d.ts` `Prisma.PrismaClient.user`) + Apply/TypeEnvironment substitution surviving one external→external hop | `core/chain.rs`, `engine/chain_walker.rs` | [generic] | Prisma/Kysely/RxJS/Mongoose, all ecosystems |
| E-M5 | Qt/Flutter ctor-call + static-member binding (candidates already indexed external) — reuses E-M1 + A-M2 mechanisms; macros per decision 5 | shared kind-compat/member lookup | [generic] | keepassxc + appflowy |

Order: E-M1, E-M2 (cheap, isolated) → E-M3 (big lever) → E-M4 last (deepest, hot-walker, measure calcom wall-clock pre/post). E-M4 conflicts with Track A on `core/chain.rs` — see §3.

### Track D — Native C surface (128,725 + native no-candidate · ~3–4 sessions + 1 review)

| MS | What | Where | Tag | Impact |
|---|---|---|---|---|
| D-M1 | **Typedef-with-attribute kind fix**: route `typedef __attribute__((…)) T Name;` to typedef handling → `TypeAlias`. Existing Global rung binds the rest. Include-graph explicitly rejected (Option B fails on `arm_mve.h`) | `c_lang/declarations.rs` + `visitor.rs` dispatch | [generic, extractor] | ~75k ≈ 0.76pt — 1 session |
| D-M2 | Structural attribute/qualifier-macro suppression: leading-position rule (type_identifier followed by another type token = qualifier macro), tightened catalog `retain` (attribute/linkage-shaped bodies only), keyword guard (kills `else`) | `c_lang/type_refs.rs::sweep_typerefs`, `extract.rs` | [generic, extractor] | ~13–18k denominator-honest |
| D-M3 | Win SDK locator activation for aspnetcore ANCM (`.vcxproj` present; verify IIS `um/` headers reachable in pinned SDK root) | `ecosystem/msvc_sdk.rs` | [locator] | ~7.7k |
| D-M4 | POSIX-on-Windows: documented borderline (locator correct; `/usr/include` absent on host; NO synthetic stubs) | docs | — | honesty |
| D-M5 | (reserve) include-edge tie-break, only if D-M1 recapture shows false binds | — | [generic] | precision |

D-M1/M2/M3 are file-disjoint → parallelizable.

### Track F — Builtin surfaces (79,447 · ~2 sessions + decisions)

| Lang | Verdict | Action | Impact |
|---|---|---|---|
| Lua | Data already indexed; engine gap | F-L: ambient admission generalizing `is_ambient_global_lib_path` for `ext:lua-stdlib:` paths + `AliasTarget` emission for `local x = math.y` + ranking (receiver > alias > ambient; bare collisions stay borderline) | ~0.06pt, 1 session |
| Bicep | Spec finite/versioned; source unreachable for template-only projects | F-B (decision 1): vendored pinned spec asset as fallback in `discover_bicep_source`; extractor reused verbatim | ~0.14pt, 0.5 session |
| MATLAB | Locator correct, install-conditional | Decision 3; fix stale `matlab_stdlib` comment (matlab_runtime.rs:11) | 0 or ~0.55pt |
| Nix | `lib.types` combinators ride F-L's mechanism IF nixpkgs source on disk; module-arg reads eval-time | Decision 9: split — borderline for reads; locator question deferred per no-walkers-without-discussion | ~0–0.04pt |

### Track T/K/I/V — Long tail (artifacts, field reads, templates, hygiene · ~10–12 sessions)

| MS | What | Tag | Impact |
|---|---|---|---|
| K1 | Erlang quoted-atom strip: `#'queue.declare'{}` target `'queue.declare'` vs `-record` symbol `queue.declare` — strip matched surrounding quotes (verified: zero record defs carry quotes) | [extractor] | ~5k bindable, 0.5 session |
| V1 | Rebar `deps/` vendor recognition via self-declaring manifest (`.app.src`/`rebar.config` under subtree, NOT a workspace member; umbrella `apps/<x>/` protected) | [generic, ecosystem] | ~16k denominator, rabbitmq rate becomes meaningful |
| I1 | **Go anon-struct locals** (flagship): fields already indexed under `<func>.<field>`; synthesize anon-struct TypeId from the composite literal + NEW range-element flow (`for _, s := range tests`) + existing `infer_yield` fall-through (bb179b1b pattern) | [generic] | ~32k ≈ 0.33pt, 2–3 sessions |
| T1 | MDX/Astro component connector: targets are INTERNAL symbols; wire the MDX frontmatter ESM-import region through the TS ScriptBlock path into `FileContext.imports` (Vue global-registry pattern: synthetic imports, zero resolver special-casing) | [generic]+[hook] | ~5.7k |
| T5a | scss `@include`→`@mixin` binding (in-index targets) + html `<script>` host linking | [generic] | ~2–3k |
| T2 | GSP `g:` taglib surface as profile data (decision 9) | [profile data] | ~4k |
| K2 | Java FQN segment leak (scoped_identifier vs scoped_type_identifier; both .java and JSP paths) | [extractor] | ~1.7k denominator |
| K3 | Batch: cpp keyword guard (after V1 re-measure — the `else` files are vendored zig), groovy/gsp `$` GString markers, svg-jar borderline | [extractor] | ~3k |
| I2/T3 | Nix module args, Liquibase/Grails builder DSL → documented borderline | — | honesty |

Order: K1 → V1 → I1 → T1+T5a → K2/K3 → T2 → borderline docs.

---

## 3. Cross-track conflicts and shared seams

1. **`core/chain.rs` is touched by A-M3/M4/M6 and E-M4.** Single-owner rule: land A-M4 (SelfRef/impl keying) → A-M3 (Apply-carry) → E-M4 (external-hop continuation) → A-M6. Each is additive (a fallback hop after existing lookups miss); run the full `type_checker::core::chain` suite between each.
2. **Constructor-application kind-compat is ONE design** consumed three ways: E-M1 (haskell ADT), A-M2 (pascal type-as-call), E-M5 (Qt/Flutter ctor). Design the profile flag once (`constructors_are_callable` + kind-table wiring), then each track flips its language.
3. **B-M1 unblocks B-M4 (svelte) and changes calcom** — E-M4's calcom verification must run AFTER B-M1 lands or its before/after numbers are confounded. Sequence: B-M1 → recapture calcom → E-M4 → recapture calcom again.
4. **V1 (rebar vendor) before K3-cpp and before any rabbitmq/zig accounting** — vendored-path artifacts may vanish from the denominator first.
5. **D-M1 before decision-2 reclassification measurement** — fix the real bug first, then reattribute; both are wanted.
6. **One cargo at a time, corpus-wide rule:** implementation agents may edit in parallel (file-disjoint tracks), but `cargo test` invocations serialize globally. This is the real parallelism bottleneck — batch edits, test in sequence (per the batch-then-test rule).

---

## 4. Execution waves

```
Wave 0  (0.5–1 session)   Architect decision session: menu §1. Prep commit: inert profile axes.
Wave 1  (parallel, ~4-5 sessions wall-clock)  — independent, high-value, low-risk:
        D-M1 (SIMD kind fix)        ~0.76pt    B-M1 (import bindings)   ~0.5–1.0pt
        K1   (erlang atoms)         ~0.05pt    V1   (rebar vendor)      denominator
        E-M1 (ctor kind-compat)     haskell    F-L  (lua ambient)       ~0.06pt
Wave 2  (parallel, ~5-6 sessions)
        A-M1→A-M2 (pascal, critical path)      I1 (go anon-structs)
        E-M2 (injected globals)                B-M2 (path normalization) + B-M3 (dart)
        D-M2 (macro suppression)               D-M3 (win sdk locator)
Wave 3  (serialized on core/chain.rs, ~4-5 sessions)
        A-M4 → A-M3 → E-M4 → A-M6              E-M3 (reachability leaf-walk)
        T1 (mdx connector) + T5a (scss/html)   B-M4 (svelte verify)
Wave 4  (~3-4 sessions)
        A-M5 (haskell extractor)               E-M5 (Qt/Flutter)
        F-B (bicep, after decision 1)          B-M5/B-M6, K2/K3, T2
Closeout (1-1.5 sessions)
        Borderline documentation (decisions 4/9; D-M4; I2/T3; per-language caps)
        ONE full corpus recapture → new baseline → floors reset → report regen
```

Wall-clock estimate for the whole program, architect + AI pair with parallel implementation agents and serialized cargo/test: **~18–24 focused sessions**. The first ~5 sessions (Wave 0+1) bank an estimated **+1.5–2.0pt** alone.

## 5. Impact math vs the 99% target (honest)

Denominator 9,806,857 (1pt ≈ 98k refs). Summing realistic per-track captures:

| Track | Realistic capture |
|---|---|
| A (chain/receiver) | +1.5–2.0pt |
| B (workspace/barrels) | +0.8–1.4pt |
| E (externals binding) | +0.5–0.8pt |
| D (native C) | +0.9–1.0pt |
| F (builtins) | +0.2pt (+0.55 conditional on MATLAB install) |
| T/K/I/V (long tail) | +0.6–0.7pt |
| **Total** | **+4.5–6.2pt → ~91.5–93.5% corpus aggregate** (+MATLAB conditional) |

The corpus aggregate does NOT reach 99 from these plans alone — and per the resolution contract it doesn't have to: **the gate is per-language ≥99 through the generic engine, with documented borderline for what static resolution cannot honestly bind.** What the plans deliver per-language:

- Languages pushed to or near ≥99: pascal (castle/doublecmd mechanisms ≈90%+ of its gap), typescript (B-M1 + E-M4 close the monorepo+prisma mass), dart (B-M3 + E-M3), go (I1 ≈80% of its gap), svelte (B-M4), erlang (K1+V1), haskell (A-M5+E-M1), bicep (F-B), java (K2 + B-M5 + existing 91.65 base).
- Languages whose residual is then **borderline-documentable**: lua (dynamic self + bare-name collisions), nix (eval-time injection), matlab (install-conditional), groovy/gsp (runtime methodMissing DSL), c/cpp POSIX-on-Windows host limitation.
- The no-candidate residue (~10–14% of current unresolved) is the long-term frontier: macro expansion, build-step codegen, remote flake inputs — each either gets a locator when real source exists on disk, or a documented cap.

## 6. Program-wide guardrails

- **Single-project recaptures only between milestones** (`bw quality-check --recapture --project <name>`); **ONE full corpus recapture at closeout** — never mid-program.
- **One cargo invocation at a time, globally** — across all implementation agents. Targeted `cargo test -p bearwisdom --lib <module>`; never redirect-then-grep.
- Every new profile axis lands **inert in DEFAULT_PROFILE** (byte-identical behavior for non-opted languages); the 14 ≥99 languages are the regression floor.
- Every milestone starts with the failing fixture (sibling `_tests.rs`); precision guards are first-class (e.g., `widget:close()` must NOT bind stdlib; local `Foo` must beat imported `Foo`; two same-named anon-struct fields bind to their own functions).
- Suppression must be structural (recognized syntactic position) — never by name list. Binding must never guess between candidates — decline and document instead.
- Mid-reindex SQL lies; trust `bw reindex` stdout.

---

## 7. Execution status ledger

_Updated 2026-06-12 (bug-fix wave 1). "Done" = code landed AND full `cargo test -p bearwisdom --lib` green (7,084+ passing). Recaptures pending for all items (single-project, batched after the wave)._

| Item | Status | Notes |
|---|---|---|
| K1 erlang quoted-atom records | ✅ DONE | `strip_quoted_atom` in `record_expr` arm; record_update/access/index verified ref-free; def side already stripped |
| K2 java FQN segment leak | ✅ DONE | leak was the `scan_all_type_identifiers` backstop recursing into `scoped_type_identifier` prefixes; covers .java + embedded-JSP |
| B-M5a java `LOG` as type_ref | ✅ DONE | `emit_chain_type_ref` gated to ≥3-segment chains (Java-local); 2-segment receiver TypeRefs proven redundant — chain roots resolve via `types_by_name`. `Stripe.Event.create()` pin guards the gate |
| V1 rebar `deps/` vendor recognition | ✅ DONE | extended `vendored_self_declared.rs` (rebar family, own gate); umbrella `apps/`/`lib/` protected via `REBAR_APP_DIRS`; known gap: custom `project_app_dirs` not parsed |
| D-M1 typedef-attribute kind fix | ✅ DONE | parse-tree probe DISPROVED the single-declaration hypothesis: construct splits into stranded `type_definition` + sibling `declaration`; fix detects the artifact and emits the sibling's declarators as TypeAlias |
| D-M2/K3 cpp keyword guard (`else`) | ✅ DONE | TWO emission paths needed the guard: `type_refs.rs::sweep_typerefs` AND the visitor's declaration `type_identifier` arm (`visitor.rs` — error recovery re-lexes keywords as a declaration's type token). NOTE: `keywords::KEYWORDS` was the WRONG list (contains `string`/`vector`); `CPP_KEYWORD_BLOCKLIST` is correct |
| B-M2 path normalization | ✅ DONE | producer = `secondary_scan.rs::resolve_to_existing_files`; lexical `..`-folding + removal of the `canonicalize()` that created the verbatim/non-verbatim `strip_prefix` asymmetry (the 1,562-row production shape); `is_under_project_root` now a lexical prefix check on folded paths |
| K3 groovy/gsp `$` GString markers | ✅ DONE | production fix was right the first time; the failing tests used fixtures that don't produce the `$` parse shape (simple GStrings absorb `$` into `string_fragment`). Real `$` sources: Geb/jQuery-style `$()` `method_invocation` + GString closure-body parses — rewritten fixtures cover them; inner-expression refs preserved |
| B-M1 import-type extraction | ❗ REFRAMED | premise FALSIFIED: extraction works (proven vs live DB + compiled grammar); real bug = `by_package` first-wins bare-qname collision (`engine/index/build.rs:1011-1016`) starving `resolve_via_workspace_package`. Diagnostic tests landed as permanent guards. Secondary backlog item: persisted `imports` table only reads `EdgeKind::Imports` (starves completion/unresolved_classify readers; not rate-bearing) |
| B-M1′ by_package collision fix | ✅ DONE | `by_package` now populated from the raw per-symbol loop BEFORE qname collapse (was: from first-wins-collapsed `by_qname.values()`); full-build path now consistent with the already-correct incremental `augment.rs` path; re-export alias synthesis push preserved; construction strictly cheaper (one fewer pass). All 4 consumers preserved. Failing-first collision test: deep workspace import binds the correct package's symbol despite same-qname siblings + external twin |

_Full `cargo test -p bearwisdom --lib` after the wave + repairs + by_package fix: **7,089 passed, 0 failed** (2026-06-12)._

### Wave 2 (2026-06-13) — all landed, suite 7,156/0

| Item | Status | Notes |
|---|---|---|
| Prep + A-M1 + A-M2 pascal chain | ✅ DONE | `multi_candidate_ranking` axis (inert default, pascal-only on); `ModuleScope::SameDirUnique` (TCastleColor case); `resolve_via_ranked_candidates` wired gated at ladder end; NEW `resolve_via_arity_ranked` engine rung (args pick the overload via `arg_assignable_candidates`). Orchestrator repair: SameDirUnique dedup keyed by (qname, kind, **file_path**) — cross-file same-qname rows are overloads, not duplicates (the Swift SourcesTargetSubtree arm intentionally keeps qname-level collapse for extension merging) |
| I1 go anon-struct + named-struct range flow | ✅ DONE | no new type machinery: anon-struct identity = enclosing-fn qname (fields already members of `arena.class(fn_qname)`); range-var typed via byte-range Narrowing (collision-free by construction). Orchestrator extension: NAMED struct slice elements narrow too, gated on an in-file type-like declaration (primitives stay un-narrowed) |
| E-M1 constructor kind-compat | ✅ DONE | haskell already accepted Calls→EnumMember (locked by test); widened ocaml Calls→Struct (variant ctors extract as Struct — noted looser, EnumVariant kind is the precise future fix) + fsharp Calls→EnumMember; java negative pin |
| E-M2 clojure :refer | ✅ DONE | extractor already emitted per-name Imports refs; clojure `build_file_context` built ImportEntry BACKWARDS (imported_name=module, alias=name) — fixed to import-binding shape; `classify_external` arm aligned. Orchestrator repair: `.clj/.cljs/.cljc` added to `trim_source_extension` (the rung's path match failed on extension) |
| B-M3 dart attribution | ✅ DONE | dart pubspec `name:` now populates `declared_name` (missing manifest arm); [hook] `resolve_via_dart_package_import` scans whole-library `package:` imports → `symbols_in_package` (textbook hook case — no named bindings to express as data). CEILING CORRECTED: appflowy bindable ≈4,155 not 17,452; `.pb.dart` codegen NOT INDEXED AT ALL → NEW BACKLOG: dart codegen indexing coverage |
| D-M2 macro suppression | ✅ DONE | catalog missed perl5 because defines are indentation-nested (`#  define`, 1,495 in perl.h) — parser required adjacent `#define`; body-shape gate (`#define MyInt int` survives); position rule = ERROR-sibling fingerprint (probe-verified; looser two-token rule would over-suppress). Orchestrator repair: string literals tokenize whole (`extern "C"` case) |
| D-M3 win sdk locator | ✅ DONE | vcxproj scan depth cap 6→10 (IIS installer projects at depth 7); SDK probe/um-registration/demand-pull verified correct. Env probe noted: `BEARWISDOM_MSVC_INCLUDE` (decision 10). vba-rubberduck has no vcxproj → typelib surface, out of scope |
| F-L lua ambient | ✅ DONE | `is_ambient_global_lib_path` generalized to any `ext:*-stdlib:` ecosystem path; `AmbientGlobals::On` lua-only; `local floor = math.floor` alias capture in extract; ranking receiver > alias > ambient > decline |

_Wave-2 suite: **7,156 passed, 0 failed**. Both pascal-chain and lua-ambient agents died at a session limit mid-flight and stalled on resume; orchestrator completed verification + 5 compile/test repairs directly. Recaptures pending: castle, doublecmd, pocketbase, pandoc, babashka, appflowy, perl5, aspnetcore, koreader, svelte-shadcn._

### Wave 3 (2026-06-13) — all landed, suite 7,199/0 (first compile)

| Item | Status | Notes |
|---|---|---|
| A-M4 rust SelfRef | ✅ DONE | Premise corrected: impl-association already worked (2,482 same-scope edges in the evidence DB). Real gap = opaque interning of generic self-scopes — `arena.class("Parser<T>")` never decomposed; one-line `intern_type_str` fix carries Apply params (members keyed under both raw and base qnames, so safe). `find_position_of` RE-SCOPED: bare free-fn call via `use super::*` — default_resolver wildcard-visibility, wave-4 item. `clone`-on-unbounded-T pinned DECLINED |
| A-M3 kotlin Apply-carry + scope functions | ✅ DONE | mid-chain Apply re-bind already existed (now guarded by fixture); NEW `scope_functions` profile axis (`Receiver` for apply/also — threads receiver; `LambdaBody` for let/run/with — declines without chain-miss). Inert `&[]` in DEFAULT_PROFILE + all 92 language profiles; kotlin opts in |
| E-M3 reachability leaf-walk | ✅ DONE | Chain corrected: `expect` leaf is **matcher's secondary `expect.dart` library** (matcher's own entry never exports it) — only `package:test`'s cross-package export reaches it. `SiblingRoots` cross-package export-follow in `demand_pre_pull`, file-granular, depth 3→5, triple-bounded. ~~group/setUp = M-4 keying~~ CORRECTED by wave-4 DB evidence: `group` 3,068 edges vs `setUp` 0 edges in the SAME external file — bare-name binding gated on which roots got demand-pulled (M-3 family after all); wave-3 pre-pull may cover it, serverpod recapture arbitrates |

### Wave 4 (2026-06-13, in flight)

| Item | Status | Notes |
|---|---|---|
| E-M4 external-hop chain | ✅ DONE | hop-1 (receiver→first member) already worked via external-type-symbol promotion; the real break was hop-2+: bare delegate yield with no registered type symbol dropped the namespace context. Fix: `prev_member_ns` carried namespace + one additive re-qualification probe in `qualified_member_lookup` (widening-only, O(1), internal path pays one prefix check). NEW BACKLOG: prisma generated-client extracts as 103 type_aliases — NO delegate class/method symbols; calcom findMany residue blocked on delegate-structure EXTRACTION, not the walker |
| B-M6 vue auto-import | ✅ DONE | config-driven only: `auto_import_dts.rs` parses `components.d.ts`/`auto-imports.d.ts` (`typeof import('…')['…']` bindings, both PascalCase tags and camelCase composables), registry carries exact tag→module entries, hooks inject ImportEntry — zero resolver changes. Evidence-direct: hoppscotch's top unresolved tags ARE the d.ts entries. Vuetify-plugin components + `$t`/`$emit` globals = documented borderline (no config source / runtime contract) |
| Residuals (4 tasks) | ✅ DONE | (1) shadcn −2.2 VERDICT: honest dedup — 18,042 edge rows = 10,951 distinct logical refs (1.65× row multiplicity); no resolution lost; METRIC NOTE: rate counts edge ROWS, denominator carries row-multiplicity noise corpus-wide. (2) prolog quoted atoms: NO-OP — Scryer `'$…'` VM builtins implemented in Rust, both sides quoted, 0/172 would match under any normalization; correctly unresolved. (3) scala FQN: latent `stable_type_identifier` leak closed in both walkers (trailing simple name only). (4) rust `use super::*` glob-import: DIAGNOSIS ONLY — extraction stores module_path=NULL for wildcard uses AND no relative-module (`super`/`crate`) wildcard mapping exists; needs new machinery; `#[ignore]` fixture documents both gaps — WAVE-5 ITEM |
| T2 gsp taglibs | ✅ DONE | Framing inverted by DB: ALL unresolved "taglib" refs are embedded `${tag(...)}` EXPRESSION calls (effective_lang=groovy, .gsp host); markup `<g:tag>` emits no refs; dominant custom taglibs (warehouse:message ×4,182) not even indexed (GString error-recovery drops field_declarations). Standard `g:` contract as profile data per decision 9 (gsp/taglib.rs + groovy classify_external consult; triple-gated: .gsp host + no chain + Tier-1-first; branded `grails-taglib` external). Projected ~2.9k openboxes drain. OPEN (larger workstreams): `<ns:tag>` markup ref emission + groovy GString error-recovery for taglib closure indexing |

_Wave-4 suite: **7,232 passed, 0 failed** (41 ignored incl. the rust glob-import documentation fixture)._

**INCIDENT (caught at first verification recapture):** the wave-3 cross-package export-follow ran away on dart-serverpod — 18 GB working set, killed at ~75 min. TWO defects: (1) `extract_dart_cross_package_exports` reused `directive_spec`, which also matches `import` lines — cross-package IMPORTS hopped, expanding the walk into the import closure of the entire pub cache instead of the entry's export tree; (2) the `seen` set was per-root, so ~79 roots each re-walked the overlapping closure. Fixed: exports-only cross-package hops + caller-owned `seen` shared across the whole pre-pull (each file pulled once project-wide). Two regression pins added (`cross_package_import_does_not_hop`, `shared_seen_pulls_each_file_once_across_roots`). pub_pkg suite 16/0.
| A-M5 haskell where-locals | ✅ DONE | Real gap = missing `bind` node arm: nullary where-locals, nullary top-level values, paren-form operator defs, AND let-locals were silently dropped (with-args where-locals already worked). Operator defs normalized to bare surface form (infix-form name from `operator` field). Pandoc reframe: `mknode` mass is import/export-linkage, `$$` is an EXTERNAL operator — M5 gain moderate, residue is E-track |
| T1 mdx/astro connector | ✅ DONE (already wired) | Plan's root cause was STALE — the full pipeline (collect_imports_exports → embedded dispatch → TS file-context → component/file-import rungs) already existed. Deliverable = regression fixtures (mdx 4, astro 2 — astro frontmatter path was completely untested). Orchestrator added `.astro`/`.mdx` to `trim_source_extension` (extensionless-import latent bug, same class as `.clj`) |
| T5a scss + html connectors | ✅ DONE | scss: kind-compat hypothesis wrong (PERMISSIVE table); real gaps = file-level `@use` can't express member binding (→ `resolve_bare_post` hook binding unique reachable Function, builtins decline naturally) + `SCSS_CSS_FN_HINT` module_skip pre-ladder decline for @function calls. html: `build_file_context` returned None for ALL html files → new `HtmlHooks` (script-src import entries + host-hook fallback for embedded-JS calls, HEEx precedent). WATCH-FLAG: host_file_ctx None→Some also activates the previously-dead custom-element selector path for all HTML — eyeball html-heavy projects in recapture |

**Recapture results (tracked baseline, single-project):**

| Project | Pre | Post | Delta | Notes |
|---|---:|---:|---:|---|
| velocity-apache-struts | 90.51 | 91.88 | +1.37 | K2 + B-M5a verified: java.type_ref 3,997 → 830, unresolved −3,216. Edges also −4.9k: the ≥3-segment gate removes previously-RESOLVED 2-segment receiver TypeRefs (`Collections` in `Collections.sort()`) — call-chain edge remains, but `find_references` on receiver classes loses those hits. Rate-correct; graph-richness tradeoff to revisit if reference queries regress |
| gsp-openboxes | 42.72 | 43.58 | +0.86 | K3 verified with an honest swap: gsp `$` markers 2,654 → 248, but each suppressed marker now yields its inner-expression refs (taglib calls the artifact was hiding), so bucket totals look flat while composition changed; edges +890 (some inner refs resolve). Remaining `$` is jQuery (2,811 js + 248 gsp-embedded script) = external-lib category (Track E), not an artifact |
| erlang-rabbitmq | 86.76 | 90.58 | +3.82 | K1 verified: erlang.instantiates 6,441 → 2,429 (quoted atoms bind; remainder is the out-of-scope `?MACRO` shape); edges +1,729. V1 purged 107 true third-party dep files. CORRECTION to the category analysis: the "86% vendored" claim was a path-regex artifact — `deps/` in rabbitmq is mostly the FIRST-PARTY umbrella layout (`deps/rabbit`, `deps/rabbitmq_management`); V1's manifest gate correctly kept it internal. Its per-project rate was already meaningful |
| prisma-calcom | 88.85 | 93.86 | +5.01 | B-M2 verified: files 8,397 → 6,836 (−1,561 duplicate `/../` rows, matching prediction exactly; their duplicate edges deduped too, edges −34k). B-M1′ verified: typescript.type_ref 23,739 → 9,563 (−14,176 — workspace imports bind through the collision-proof by_package). Unresolved −15,300 |
| zig-compiler-fresh | 79.01 | 91.33 | +12.32 | D-M1 verified at scale: c.type_ref 47,005 → 13,556, cpp.type_ref 36,606 → 5,543 (−64.5k bound), edges +46,872, unresolved −64,554. Biggest single-project move of the wave |

Wave-1 net: 5 projects, **−104.4k unresolved / +5 to +12pt per project**, all mechanism-verified at the ref level.

**Wave-2 recaptures (wave-2 binary; wave-3 fixes NOT included):**

| Project | Pre | Post | Δ | Notes |
|---|---:|---:|---:|---|
| clojure-babashka | 47.56 | 74.54 | +26.98 | :refer — 5,123 bound |
| go-pocketbase | 64.59 | 76.44 | +11.85 | range flow — unres −12,739 |
| perl-perl5 | 82.80 | 89.46 | +6.66 | macro catalog reach + suppression — unres −6,871 |
| pascal-doublecmd | 84.92 | 90.30 | +5.38 | overloads/SameDirUnique/arity |
| pascal-castle-fresh | 72.53 | 77.72 | +5.19 | unres −14,868 |
| lua-koreader | 34.84 | 38.40 | +3.56 | ambient stdlib; small new-attempt surfacing |
| dart-appflowy | 67.86 | 70.50 | +2.64 | dart hook; ceiling-limited (.pb.dart codegen backlog) |
| go-fiber | 85.85 | 87.18 | +1.33 | range flow |
| haskell-pandoc | 50.72 | 50.68 | −0.04 | expected flat — movers are wave-3 (bind arm) + E-M4 |
| svelte-shadcn | 59.13 | 56.91 | **−2.22** | ✅ VERDICT: **honest dedup / metric-attribution artifact, NOT a regression.** Proof from the fresh DB: (1) internal file set is COMPLETE and unchanged — index has 1,640 svelte / 1,025 ts / 11 js internal files vs 1,641 / 1,028 / 11 on disk; secondary-scan (B-M2) touched nothing here: the project has **zero** internal `/../` import specifiers (verified by replaying the pre-fold verbatim logic over the source — 0 would-be duplicate rows), so suspect (a) is ruled out. (2) External file set has no duplicates (516 of 517 distinct folded). (3) Suspect (c) is java-only — no java in svelte. (4) **Unresolved went DOWN 138, not up** — a real loss would move references INTO `unresolved_refs`; instead resolved-logical and unresolved sets are disjoint (overlap 1, noise). The ~1,900 vanished rows are redundant duplicate edge ROWS: 18,042 edge rows collapse to 10,951 distinct (source,target) logical refs (1.65× row multiplicity; 3,120 logical refs own 10,211 rows). The rate metric is `edge_rows / (edge_rows + unresolved_refs)`, so the wave's by_package dedup + path-fold removed duplicate edge rows and depressed the rate **without losing a single resolved reference**. No code change. Root metric limitation: rate counts edge rows, not distinct logical references |
| dotnet-aspnetcore | 97.78 | 97.89 | +0.11 | c.type_ref cleared (863→0); cpp.type_ref only −757 of 12,107 — D-M3 fixed DISCOVERY but binding through the Win SDK headers still under-delivers; follow-up = SDK-header admission/extraction depth diagnosis (wave 5) |

### Wave 4 scoped residuals (2026-06-13)

| Item | Status | Notes |
|---|---|---|
| svelte-shadcn anomaly | ✅ VERDICT: honest dedup | See the recapture row above — not a regression; no code change |
| prolog quoted-atom records (511) | ✅ VERDICT: NO-OP (borderline external), not the erlang shape | Drilled prolog-scryer: the 511 quoted targets are dollar-prefixed Scryer VM builtins (`'$fast_call'`, `'$module_call'`, `'$prepare_call_clause'`) — implemented in **Rust** (`Machine.fast_call` in `src/machine/system_calls.rs`), never defined as Prolog predicates. Both sides quote consistently (Prolog names symbols quoted, with arity: `'$default_attr_list'/2`), so the matched-pair strip the task gated on does NOT apply (def side carries quotes). Decisive: **0 of 172** distinct quoted targets gain a match under either a quote-strip OR a both-sides quote-normalization. `'$fail'`/`'$get_cp'` look strippable but `$fail` ≠ `fail` (different predicates) — stripping `$` would be a false bind. These are the runtime ABI surface; correctly unresolved (external). No extractor change |
| scala FQN package-segment leak (gatling 260) | ✅ DONE (latent path closed) | Live DBs (gatling/finatra/lila/trading) show **0** current pkg-segment type_ref leaks, but the latent path is real: `calls.rs::extract_type_refs_from_type_arguments` had no `stable_type_identifier` arm, so a FQN type arg (`List[org.apache.X]`) fell to the recursing default arm. Prefix segments survive today only because the grammar tags them `identifier` (not emitted), not by design. Added an explicit `stable_type_identifier` arm that emits the trailing simple name only and does not recurse — consistent with the extends/`with` walker (`collect_type_names_from_node`). Failing-first fixture `type_arg_fqn_emits_trailing_name_only` in `scala/calls_tests.rs` |
| rust `find_position_of` wildcard-visibility (1,364) | ⚠ DIAGNOSIS ONLY — needs new machinery | `use super::*` in `tests/action.rs` reaches `pub fn find_position_of` in the parent module `tests.rs`. TWO gaps: (1) EXTRACTION — `calls_imports.rs::use_wildcard` arm reads only the inherited `prefix`, never the `use_wildcard`'s own path child, so `use super::*`/`use crate::x::*` store `module_path=NULL` (848 such NULL wildcards in the DB; only 6 `super`/14 `crate` survive via the nested `scoped_use_list` path). (2) RESOLUTION — even with `module_path="super"`, neither existing wildcard mode binds it: `QnameUnder` fails because Rust top-level symbols carry bare qnames (`find_position_of`, not `tests::find_position_of`), and `super`/`crate` are relative module keywords mapping to a FILE location, not a qname namespace or file stem (`tests.rs` stem is `tests`, not `super`). Closing it needs a relative-module wildcard path: map `super`/`crate`/`crate::x` to that module's files, then match the bare target against symbols defined there. Per the no-build-without-report constraint: NO resolver machinery built. Failing `#[ignore]` fixture `wildcard_rust_super_glob_binds_parent_module_fn` in `default_resolver_tests.rs` |

---

## 8. Architect decision session (2026-06-13) — menu §1 resolved

All recommended options approved. Two overrides: **§6 → V2 implemented NOW** (not deferred); **§10 → recommended** (replace env probes with standard-path discovery, env as non-gating hint).

| # | Decision | Resolution | Implementation |
|---|---|---|---|
| 1 | Bicep ARM spec vendored | APPROVE | ✅ F-B landed (Wave 5) |
| 2 | zig vendored-subtree reclassification | BOTH (D-M1 done + reclassify) | ✅ landed (Wave 5, `toolchain_payload.rs`) |
| 3 | MATLAB on recapture machine | Borderline (no install); zero code | ✅ comment fixed (Wave 5); borderline doc → closeout |
| 4 | Dynamic-receiver depth (Lua/JS) | Approve line as drawn | Already coded (A-M6); borderline doc → closeout |
| 5 | Qt boundary three-way split | Approve | Already coded (D-M2 + E-M5) |
| 6 | Vendor policy generalization | **V2 NOW** (override) | ✅ V2 landed (Wave 5, go-vendor + composer-vendor) |
| 7 | `multi_candidate_ranking` opt-in | Approve (pascal first) | Already coded (A-M2) |
| 8 | Java `LOG` fix locus | Extractor fix | Already coded (B-M5a) |
| 9 | Template borderline calls | Document borderline; taglibs/connectors as data | Connectors coded (T1/T2/T5a); borderline doc → closeout |
| 10 | Env probes in locators | **Recommended** (override of "tolerable hint") | ✅ all 3 removed → standard-path discovery (Wave 5) |
| 11 | Externals hydration depth | Approve | Already coded (E-M3) |
| 12 | Pascal include-group boundary | SameDir first | Already coded (A-M1, SameDirUnique) |

### Wave 5 (2026-06-13) — decision-menu implementations + diagnosis closeout

| Item | Status | Notes |
|---|---|---|
| #1 BLOCKER: thymeleaf-myblog / vue-hoppscotch regression diagnosis | ✅ VERDICT: HONEST EXCLUSION (both) — NO code change | **T5a HtmlHooks EXONERATED** (HTML file counts unchanged: thymeleaf 90→90, hoppscotch 5→5). thymeleaf −19.6: the −155 lost files are 100% vendored **editor.md v1.5.0** library JS under `static/admin/plugins/editormd/` (ships its own `package.json`/`bower.json`/`LICENSE`); the wave-5 `vendored_self_declared` npm path (`npm_vendored_prefixes` → `is_under_self_declared_vendor`, `full.rs:461`) now vendors the WHOLE subtree, where the bb179b1b baseline captured a partial-vendoring intermediate (155 of 191 JS leaked internal). Only app ref is a global `editormd(...)` call — zero internal edge lost. vue-hoppscotch −3.77: −394 dropped TS are Prisma/GraphQL **generated code** + `.d.ts` + `dist/` over-counted at baseline (1025 indexed ≈ 1027 on-disk hand-written source; 6,891 of unresolved TS are real source, 88.43% is the honest source-only rate). vue rate genuinely +51 (33.39→84.56, B-M6). Both = `research_rate_regression_artifact` shape |
| F-B bicep vendored spec (decision 1) | ✅ DONE | NEW `assets/bicep/namespace_surface.json` distilled from Azure/bicep **v0.44.1** (`SystemNamespaceType.cs` + `AzNamespaceType.cs` + `LanguageConstants.cs`): 120 functions, 16 decorators, `sys`/`az` aliases. `bicep_runtime.rs::discover_bicep_source` falls back to the vendored asset (via `include_str!` + serde) when no in-tree clone found; **in-tree clone still takes precedence**; `extract_function_names` reused verbatim on the clone path. 4 sibling tests. ARCHITECT FLAG: `az`'s `list*` family is an upstream regex wildcard (`FunctionWildcardOverloadBuilder`) — can't be exact-name-matched; included base `list` + 4 common concretes (`listKeys`/`listSecrets`/`listAccountSas`/`listServiceSas`); arbitrary `list<X>` needs a generic wildcard-match engine mechanism (out of F-B scope) |
| V2 vendor-policy generalization (decision 6, override) | ✅ DONE | Generalized V1's manifest-gate to two more ecosystems in `vendored_self_declared.rs`: **go-vendor** gates on `vendor/modules.txt`; **composer-vendor** gates on `vendor/composer/installed.json`. Shared `walk_for_vendor_ledger` — recognition is self-declaring (ledger INSIDE the subtree), NEVER a path-segment name list. KEY SCOPING FINDING: the walker's `ROOT_ONLY_EXCLUDE_NAMES=["vendor","lib","libs"]` already drops root-level `vendor/`; V2 closes the **nested** `vendor/` gap (monorepo `services/api/vendor/...`). Host's own module (root `go.mod`/`composer.json`) is structurally never under a `vendor/` segment → never misclassified (rabbitmq-lesson protection, done structurally). 6 sibling tests incl. 3 precision guards |
| zig vendored-toolchain reclassification (decision 2) | ✅ DONE | Extended `toolchain_payload.rs::zig_payload_prefixes` (the EXISTING owner, gated on `lib/std` marker) to also recognize `lib/include` (213 bundled clang `.h`) + `lib/libtsan` (compiler-rt/TSan). No manifest exists for these — toolchain-subtree shape, not self-declaring; correct layer is `toolchain_payload` (not a duplicate `vendored_self_declared` path). zig's own `lib/std`/`lib/compiler`/`src/` stay internal. ARCHITECT FLAG: orchestrator agreed with the agent's location choice (avoids duplicating the `lib/std` gate) |
| Env probes → standard-path discovery (decision 10) | ✅ DONE | All 3 `BEARWISDOM_*` env reads REMOVED (each was a private duplicate of a standard toolchain var): `BEARWISDOM_QT_DIR`→`QTDIR`/`Qt6_DIR`/`Qt5_DIR` + autodetect roots; `BEARWISDOM_MATLAB_ROOT`→`MATLAB_ROOT` + `matlab -batch` + standard installs; `BEARWISDOM_MSVC_INCLUDE` (the one genuinely-gating early-return)→`VCINSTALLDIR`/`vswhere` + Windows Kits. Unsetting any no longer changes behavior. Stale `matlab_stdlib` comment (matlab_runtime.rs:11) corrected to present-tense. Test-only touch flagged: `compile_commands_tests.rs` 2 precedence tests retargeted to `QTDIR` |

_Suite verification: single serialized `cargo test -p bearwisdom --lib` — **7,244 passed, 0 failed, 41 ignored** (clean first compile, no orchestrator repairs; +12 from Wave-4's 7,232)._

**Category re-extraction (post-Wave-4 binary, all 251 app-class DBs, read-only):** corpus app-class rate **86.94 → 88.05 (+1.11pt)**, **−159,929** unresolved. Biggest drains: `native_simd` −62,995 (zig), `type_link_gap_in` −25,894, `call_link_gap_in` −23,707, `field_read_gap_in` −13,423 (pocketbase struct fields), `native_macro` −6,418, `artifact_quoted_atom` −5,004, `codegen_client` −2,476 (prisma), `artifact_pkg_segment` −1,570. Largest remaining: `call_link_gap_in` 362,958 (32.4%), `type_link_gap_in` 161,428, `no_candidate_calls` 118,724, `stdlib_builtin` 79,441 (matlab-platemo alone 51,272 — install-conditional borderline).

**Verification recaptures (Wave 3+4 binary, single-project):**

| Project | Pre | Post | Δ | Notes |
|---|---:|---:|---:|---|
| dart-serverpod | 66.02 | 76.29 | +10.27 | E-M3 leaf-walk verified post runaway-fix (bounded export-follow, shared seen) |
| astro-starlight | 81.01 | 87.98 | +6.97 | T1 mdx/astro connector |
| scala-gatling | 89.14 | 90.38 | +1.24 | scala FQN latent path |
| gleam-compiler | 80.99 | 82.00 | +1.01 | — |
| haskell-pandoc | 50.72 | 52.24 | +1.52 | A-M5 bind arm + E-M4 |
| kotlin-ktor | 81.64 | 81.80 | +0.16 | A-M3 scope functions |
| gsp-openboxes | 42.72 | 43.60 | +0.88 | T2 taglibs |
| thymeleaf-myblog | 96.04 | 76.44 | −19.60 | ✅ honest exclusion (editor.md full vendoring) — see Wave-5 diagnosis row |
| vue-hoppscotch | 92.06 | 88.29 | −3.77 | ✅ honest exclusion (generated/dist over-count at baseline); vue +51 from B-M6 |

### Wave 5 backlog (2026-06-13) — the items diagnosed-but-deferred across waves 1–4

| Item | Status | Notes |
|---|---|---|
| rust `use super::*` / `use crate::*` glob machinery | ✅ DONE | Both wave-4-diagnosed gaps closed. EXTRACTION (`rust_lang/calls_imports.rs`): `use_wildcard` now reads its own `named_child(0)` (`_path`: crate/super/self/identifier/scoped_identifier — grammar has NO `path` field, so the old `child_by_field_name("path")` could never see it), storing `super`/`crate`/`crate::x` instead of NULL. RESOLUTION (`type_checker/core/default_resolver.rs`): new additive `resolve_via_relative_module_wildcard` rung after `resolve_via_wildcard_import`, fires only when the wildcard `module_path` head is `super`/`crate` (Rust-reserved → `[generic]`, inert elsewhere, no profile field); maps `super`→parent-module roots (`<dir>.rs` + `<dir>/mod.rs`, climbing one level for mod/lib/main), `crate`→crate root, `crate::x::y`→that module's files; matches the bare target by (file, bare_name) since Rust top-level qnames are bare. Fixture `wildcard_rust_super_glob_binds_parent_module_fn` un-ignored + 3 precision guards (declines outside parent, scopes crate::x to named-module files, local def wins) |
| groovy GString error-recovery + `<ns:tag>` markup (wave-4 T2 OPEN items) | ✅ DONE | Probe-validated both. GAP 1 (`groovy/extract.rs`): GString/`out <<` closure bodies force tree-sitter error-recovery that shreds the taglib closure assignment into flat sibling tokens (no surviving `field_declaration`) — dominant custom taglibs (`warehouse:message` ×4,182) were never indexed. New `scan_taglib_closures_from_source` line-scans the member-declaration closure shape `(def|<TypeName>) <name> = {` at class-member indent (structural, no name list; `{`-gated so `[:]` excluded), emits each as `SymbolKind::Method` (Calls→method is kind-compatible). GAP 2 (`gsp/taglib.rs` + `mod.rs` + `hooks.rs`): markup `<ns:tag>` emitted zero refs (only `<g:render>` did); new `scan_markup_tags` recognizes `<ns:tag>` structurally (lowercase-ns + `:` + local; HTML has no colon), emits chain-less Calls ref. ORCHESTRATOR REPAIR (caught by the serialized cargo run): the agent's Gap-2 "Tier-1 bare-name bind" assumption was wrong — the generic bare-name rung correctly DECLINES a bare `Calls` ref to a class-MEMBER method in another file (a false bind for nearly every language). Fix: new `GspHooks::resolve_bare_post` (the scss T5a precedent; called generically by the resolve loop at `type_checker/engine.rs:337,364`) binds a chain-less `.gsp` Calls ref to a UNIQUELY-named indexed taglib Method (declines on zero/multiple — `ambiguous_method_name_declines` guard). Standard/logical `g:` tags still brand `grails-taglib` via `classify_external` (no in-index Method → no bind → brand fires). Logical tags kept markup-only (NOT in bare-expression `STANDARD_TAGS` — `if`/`collect` are groovy keywords/methods) |
| prisma delegate-structure extraction (wave-4 E-M4 backlog) | ✅ DONE — NOT synthesis | Premise corrected: calcom uses the NEW `prisma-client` generator (split-file, output to the workspace package, not `node_modules`). The delegate structure (`PrismaClient.user: UserDelegate`, `UserDelegate.findMany(): PrismaPromise<…>`) is FULLY PRESENT & well-typed on disk in `generated/prisma/internal/class.ts` + `models/*.ts` — no synthesis needed (the old `decision_prisma_drain_spec` synthesis assumption is WRONG for this generator). Root cause: the whole `generated/prisma/*` tree is gitignored; `secondary_scan::pull_gitignored_imports` was SINGLE-HOP — pulled `client.ts` (imported by project code) but never followed `client.ts`'s own imports (`./internal/class` → `../models/User`). Fix (`indexer/secondary_scan.rs`): single-hop loop → bounded BFS worklist (`MAX_PULLED_FILES=5000`), each pulled file enqueued so its imports are followed; ecmascript-family gate now uses `detect_language` (authoritative for transitively-pulled paths). **Prisma-AGNOSTIC** — fixes any gitignored generated-client chain (GraphQL/OpenAPI/Kysely/Drizzle). DOWNSTREAM (separate): `findMany` returns `PrismaPromise<GetResult<$UserPayload,T,"findMany">>` — recovering `User[]` from that deep conditional/mapped type is a type-eval barrier downstream of E-M4, not part of this fix |
| dart `.pb.dart` codegen indexing (wave-2 B-M3 backlog) | ⚠ NOT AN ENGINE GAP — architect call | Premise FALSIFIED for the checkout: `.pb.dart` files are gitignored (`.gitignore:82 lib/protobuf`) AND never generated — `protoc` was never run, the `.proto` sources are also gitignored (`**/resources/proto`), so there are 0 `.pb.dart`/`.g.dart`/`.freezed.dart`/`.proto` on disk. BearWisdom's dart discovery is innocent (`.dart` suffix at `dart/mod.rs:46` already admits `.pb.dart`; no codegen-specific gate). The ≈13k unresolved `package:appflowy_backend/protobuf/…` refs are references into ungenerated build artifacts. B-M3's ceiling ≈4,155 is correct and stands. ARCHITECT CALL: (1) regenerate codegen in the fixture (protoc → commit `lib/protobuf/**`; `.dart` suffix indexes them, no engine change) and re-measure, OR (2) accept the ceiling and close B-M3 as dependency-on-build-output borderline. Recommend (2) unless the protobuf surface is wanted as a real resolution target |
| Win SDK header admission depth (wave-2 D-M3 follow-up) | ⚠ DIAGNOSIS — generic engine task, ROI-gated | The 11,350 unresolved aspnetcore `cpp.type_ref`: ~9,140 ADMISSION (SDK headers not in index at all — 0 files under `Windows Kits`), ~890 EXTRACTION (SAL/calling-convention macro misparse: `IN`/`OUT`/`__stdcall` as false type_refs), 672 KEYING (project-internal name collisions), 558 ATL (component not installed). ROOT CAUSE: the C/C++ external-binding model has NO LIVE PRODUCTION CONSUMER — `msvc_sdk` builds a path-keyed (`#include`-path) `SymbolLocationIndex` via `build_c_header_index`, but the prod demand-pull (`expand.rs::expand_chain_reachability`) is driven only by chain-misses probing by SYMBOL NAME; a bare `cpp.type_ref` to `HRESULT` has no `current_type` and never reaches the pull. `Ecosystem::resolve_import`/`resolve_symbol` (the only path-keyed consumers) are called ONLY in `_tests.rs`. TRACTABLE as a single-layer fix (include-driven external admission for C/C++: resolve `#include` refs through the existing path-keyed index, pull + transitively follow includes — the closure expansion `demand.rs:19` marks "not yet implemented") — ceiling ~7,500–8,500 on aspnetcore, GENERALIZES to posix-headers/vcpkg across all C/C++. Bigger than one session; scope as its own generic-engine piece, NOT an aspnetcore %-chase. Cheap adjacent win available: SAL/macro suppression (~740 false type_refs, pure cpp-extractor, zero admission dependency) |

_Suite verification (wave-5 backlog): single serialized `cargo test -p bearwisdom --lib` — first run 7,259/1 (the GSP custom-tag↔closure integration test; one orchestrator repair = `GspHooks::resolve_bare_post`), re-run **7,261 passed, 0 failed, 40 ignored** (one fewer ignored than Wave-4: the rust glob fixture un-ignored)._

### Forks A–D execution (2026-06-13) — architect approved "start all in parallel"; D held by the recapture rule

| # | Fork | Status | Notes |
|---|---|---|---|
| A | bicep `list*` wildcard | ✅ DONE | NOT a generic core-lookup glob. New `wildcard_builtins: &[WildcardBuiltin{prefix,fold_to}]` profile axis (`type_checker/profile/language_profile.rs`), inert `&[]` in DEFAULT_PROFILE + 93 swept profiles, bicep-only `[WildcardBuiltin{prefix:"list", fold_to:"list"}]`. Ladder rung in `default_resolver.rs` after `strip_ambient_prefix` folds an open-ended `list[A-Z]…` Calls to the vendored `list` builtin (binds via `default_ambient_package`); enumerated concretes (`listKeys`) still bind first. Anchored `^list[A-Z]` so `listener`/`listing`/bare `list` decline. Inert + non-match + non-bicep guards |
| B | dart `.pb.dart` (B-M3) | ✅ CLOSED borderline | Documented in `BORDERLINE-RESOLUTION-2026-06-13.md` §1 — build-output dependency (ungenerated codegen, gitignored proto sources, engine innocent). Ceiling ≈4,155 stands. Regen path (protoc → commit `lib/protobuf/**`) noted as a separate fixture-prep task if the protobuf surface is ever wanted as a real target |
| C1 | C/C++ SAL/macro suppression | ✅ DONE | Extended D-M2's structural ERROR-sibling fingerprint (`c_lang/type_refs.rs`) to the SAL/calling-convention shapes the leading-position rule missed: `_In_`/`IN`/`__in HANDLE h` (ERROR sibling of the `parameter_declaration`, one level up), `_Inout_ int* p`, `EXTERN_C HRESULT __stdcall`, `_In_reads_(n)` macro_type_specifier (skips the count-arg subtree). `__stdcall`/`__cdecl` parse as `ms_call_modifier` (no leak path). Probe-validated; 4 precision guards (`const Foo`, real `Foo h`, punctuation-only ERROR, `else` keyword guard preserved). Lowercase-identifier residual (~153) honestly scoped OUT — different emitter, not on the type_ref path |
| C2 | C/C++ include-driven admission | ✅ DONE (first slice + closure) | Dead-consumer CONFIRMED against prod: `build_c_header_index` keys headers by include-path `(path,path)`; the C `#include` Imports ref `continue`d straight to `unresolved` (never recorded a chain miss); `Ecosystem::resolve_import`/`resolve_symbol` have ZERO prod callers. Fix LANDED: (1) `loop_body.rs` Imports branch records an include-driven chain miss for C-family headers (`module=target=include_path`) → the existing EXT-1 `index.locate(module,target)` hits the path-keyed registration with NO `expand` lookup change — one generic seam lights up msvc-sdk + posix-headers + vcpkg + qt uniformly; (2) `expand.rs::follow_header_includes` runs the transitive `#include` closure (the `demand.rs:19` "not yet implemented" gap), bounded `MAX_TRANSITIVE_PASSES=5` + `MAX_PULLED_FILES=5000`, deduped against `seen_paths`/`already_walked`. PERF: only new hot-loop work is one O(1) `record_chain_miss` push; all pull/parse is demand-time in `expand`, never per-ref. 8 tests (synthetic header fixtures, no SDK-install dependency). Memo `decision-2026-06-13-hwa`. SHARED-FILE FLAGS: `loop_body.rs` (hot loop, +6 lines), `expand.rs`; minor known duplication (`looks_like_header_include`/`header_include_shape` at two seams, left un-factored for surgicality) |
| D | Closeout #7 full-corpus recapture | ⏸ LAUNCHED then HALTED by architect | Release rebuilt (7,285/0 binary); recapture ran 16/251 projects, `bw` WS bounded ≤1.15 GB (watchdog never fired), then stopped on request. The recapture writes `baseline-all.json` only at end-of-run → the partial run wrote NOTHING; baseline-all.json intact at the pre-closeout working state (23 wave recaptures). Closeout PENDING. Cost estimate (Σ per-project `index_duration_ms`): **~12.4 h** sequential, slow tail = gsp-grails-core 130min / dart-serverpod 116min / aspnetcore 74min / ts-nextjs 30min; C2 include-admission adds time on C/C++. Rerun `bw quality-check --recapture` on the final binary when ready (background overnight, watchdog armed) |

_Suite verification (forks A/C1/C2): single serialized `cargo test -p bearwisdom --lib` — **7,285 passed, 0 failed, 40 ignored** (clean first compile, no orchestrator repairs; +24 from Wave-5-backlog's 7,261)._

All session code green: decision menu (12/12) + waves 1–5 + forks A/C1/C2 = **7,285/0**. Borderline documentation seeded (`BORDERLINE-RESOLUTION-2026-06-13.md`). **Only remaining program step: fork D** — full-corpus closeout recapture on the final code state (release rebuild in progress, then recapture with memory monitoring).
