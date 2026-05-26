# Type-inference engine — remaining work, generic vs language-specific

**Status:** task list for review. Derived from a verified audit of current code
(2026-05-25). Companion to `GENERIC-PARAM-UNIFICATION.md` and
`RESOLUTION-EXTERNAL-ROUTING.md`.

**Done already this session:** bounded-generic receivers (`f6f77196`),
forward-inference yield (`f6f77196`), string-hop generic args (`f6f77196`),
U1+U2a — substitution layer turned on (`d2cbec56`).

**Tagging principle.** The engine is one generic algorithm; what's per-language
is the *data* feeding it (`LanguageProfile` / `FlowConfig` / extractor output)
and, rarely, per-language code (`LanguageEngineHooks`). Many items below are a
generic engine change **plus** a per-language feed without which the generic
part is starved — those are split into a `G#` task and its `L#` follow-up(s).

Legend: `[generic]` engine-only · `[profile]` per-language data · `[hook]`
per-language code. Size: XS/S/M/L.

---

## A. Generic engine tasks (language-agnostic algorithm)

| id | task | gap (file:line) | fix shape | deps | size |
|----|------|-----------------|-----------|------|------|
| **G1** | **Expected-type / bidirectional (D7)** | `yield_type_of` (chain.rs ~421) never reads `seg.declared_type_id` / `seg.type_arg_ids` (types.rs:409-411) | when a yield is an unbound `Generic`/`Class`, bind it from the segment's LHS annotation / turbofish via `GenericEnv`; `x as T` adopts `declared_type_id` | — (U1/U2a in place) | S |
| **G2** | **Multi-level U2b: edge-arg-as-Generic** | `build_explicit` interns `"C<T>"` → `Apply{C,[Class("T")]}` (supertype.rs:206) — child's param is nominal | intern an inheritance edge arg naming the child's own declared param as the canonical `Type::Generic` | U1 (done) | S |
| **G3** | **Multi-level U3: walk composition** | `walk_up_with_args` surfaces only the direct edge's args, no composition (supertype.rs:88-99) | substitute each edge's args through the binding the child's args established; thread `arena`+params into the walk | G2 | S-M |
| **G4** | **Dispatch wiring (MultiArg/ReturnType)** | walker calls `lookup_with_binding` directly (chain.rs:350); `dispatch::select_method` (dispatch.rs:51, axis-aware) is dead | route chain.rs:350 through `select_method`; reconcile its return to the `(sym, owner, owner_args)` triple | L4 | M |
| **G5** | **Overload-by-args (Receiver path)** | `find_on_chain` returns first name+kind match (members.rs:298-308); `arg_types` never consulted | when multiple same-name/kind candidates, pick by `args_assignable` (already in dispatch.rs) | G4 (arg types) | M |
| **G6** | **F-bounded `T extends Cmp<T>`** | `substitute` only rewrites `Generic→env` (generics.rs:77); `GenericParamData.bound` (types.rs:77) is never read | members-lookup branch: resolve a constrained param's members on its bound with `T`→receiver substituted | L1 (to fire beyond TS) | S |
| **G7** | **Discriminated-union branch selection** | `Type::Union` lookup returns the first branch (members.rs:225-242); "narrows further" comment is dead | thread the active guard (discriminant prop + literal) into `walk_with_root`; Union arm selects the matching branch | L3 | M |
| **G8** | **Union/Intersection alias expander convergence** | string `expand_alias` bails (alias.rs:194); typed `expand_alias_typed` builds the variant (alias.rs:351-358) — split-brain | route both callers through one expander | — | XS |
| **G9** | **Closure / `Fn` call-yield arm** | `Type::Function` has no yield path; calling a closure yields nothing (members.rs:265) | `yield_type_of` arm: a call on a `Type::Function` receiver yields its `return_` | L6 (to build Function at all) | S |
| **G10** | **Higher-kinded `F[_]`** | `Type` has no kinded constructor; `GenericParamData` carries no arity (types.rs:70-78) | new kinded `Type` variant + substitution that applies a bound param to an arg | L7 | L |

## B. Per-language follow-ups (feed the generic tasks)

| id | task | feeds | where | tag |
|----|------|-------|-------|-----|
| **L1** | Capture generic-param **bounds** in non-TS extractors (Scala/Kotlin/Java/…); TS already sets `GenericParamData.bound` | G6 | `languages/<lang>/extract|symbols.rs` | [hook] |
| **L2** | `typeof x === "…"` and `x is Foo` **type-predicate** guard queries (TS marks them future, flow.rs:41) | narrowing | `languages/<lang>/flow.rs` `type_guard_query` | [profile] |
| **L3** | **Discriminant** guard extraction (`switch (s.kind)`, `if (s.tag === …)`) | G7 | `languages/<lang>/flow.rs` | [profile] |
| **L4** | **Call-argument type inference** — resolve each `ChainSegment.call_args` expression to a TypeId | G4, G5 | resolve loop / root resolver (mostly generic) + per-language `call_args` extraction | [generic]+[profile] |
| **L5** | Refined **most-specific overload scoring** in per-language dispatch hooks (R/Lisp) | G4 refinement | `languages/<lang>/hooks.rs` | [hook] |
| **L6** | Parse **function-type syntax** (`Fn(..)->T`, `()=>T`, `impl Fn`) into `Type::Function` instead of `Class(..)` (intern_type_str fallback, types.rs:217) | G9 | `intern_type_str` + per-language syntax | [generic]+[profile] |
| **L7** | Emit **kinded param arity** for `F[_]` (Scala/Kotlin parse `type_parameters`) | G10 | `languages/{scala,kotlin}/symbols.rs` | [hook] |

### B.1 — Per-language scope (which languages each follow-up actually touches)

`✓` = verified from code · `~` = inferred from language syntax/feature-set (needs a per-extractor confirm).

- **L1 — bound capture.** The gap-B signature parser already handles `<T extends B>`
  and `<T: B>`, so bounds flow as long as the extractor puts the bound in the signature.
  Splits by **bound syntax**:
  - `~` already-parseable (verify the extractor includes the bound): **Java**,
    **Kotlin** (`<T : B>`), **Swift** (`<T: B>`), **Rust** (`<T: B>`), **Scala**
    (`[T <: B]` — the `:` in `<:` triggers the bound branch; verified), **Dart**
    (`extends`). TypeScript ✓ done.
  - `~` NOT caught — the bound lives **outside** the `<>`/`[]` param clause, so it
    needs extractor/clause-position work, not a separator tweak: **C#**
    (`where T : B`), **Go** (`[T Constraint]`, space-separated), **Haskell**
    (`(C a) =>` constraints).
- **L2 — guard vocabulary.** `✓` exactly the 14 languages with a `FlowConfig`: **c,
  csharp, go, java, kotlin, php, python, ruby, scala, typescript, r, groovy, lua, rust**.
  Each already has a `type_guard_query` but covers only part of its guard syntax —
  extend per language: TS `typeof`+`x is T` (audited: instanceof-only today), Python
  `isinstance`, Go type-switch, Rust `if let`/`match`, C#/Java pattern `is`, Ruby `is_a?`.
- **L3 — discriminant extraction.** `~` discriminated-union languages that have a
  FlowConfig: **TypeScript** (tagged unions), **Rust** (enums), **Kotlin** (sealed),
  **Scala** (sealed/ADT).
- **L4 — call-arg typing.** All languages (arg resolution is generic); per-language only
  to confirm `ChainSegment.call_args` is populated — most extractors already do.
- **L5 — dispatch scoring.** `✓` exactly **4**: **Clojure, R, Prolog** (MultiArg),
  **Haskell** (ReturnType). Every other language is `Receiver`.
- **L6 — function-type parsing.** `~` function-type-bearing languages: **Rust**
  (`Fn/FnMut`), **TypeScript/JS** (`()=>T`), **Kotlin** (`(T)->R`), **Scala** (`T=>R`),
  **Swift**, **Go** (`func(...)`), **C#** (`Func/Action`), **Python** (`Callable`),
  **Dart**. High-value first: Rust, TS, Kotlin.
- **L7 — HKT.** `~` **Scala** (`F[_]`) and **Haskell** only. Kotlin has no native
  higher-kinded types.

## C. TypeScript-specific features (`AliasTarget` family — only the TS extractor emits these)

| id | task | gap | size |
|----|------|-----|------|
| **TS1** | `keyof T` expansion | capture-only (alias.rs:196) — a `keyof` is a string-literal union; low chain value | M |
| **TS2** | Mapped beyond transparent (`Record`, key-remap, value-template) | only `{source}[K]` collapses (alias.rs:137) | M |
| **TS3** | **Bug:** `instanceof` guard query matches `===` too (typescript/flow.rs:44 — no keyword filter) → bogus `x→y` narrowing | filter on the `instanceof` keyword | XS |

## D. Gated routing (separate axis, not pure inference)

| id | task | gap |
|----|------|-----|
| **R1** | Ungate external arrow-return hydration (S5a) after validation — `-> T` returns only hydrate under `BEARWISDOM_COMPILER_RESOLVE` (build.rs:362) | one-line ungate |

## E. Cleanup (in passing)

| id | task |
|----|------|
| **C1** | Stale comments: chain.rs:25 ("narrowings not consulted" — root resolver does), chain.rs:27-31 (`select_method` "wired" — it isn't), members.rs:228 (dead "narrows further") |

---

## Recommended sequence

1. **G1 (D7)** — smallest, biggest corpus-wide reach, data already extracted, unblocked by U1/U2a.
2. **G2 → G3 (multi-level)** — finishes the generic-inheritance story; no per-language feed needed.
3. **G8** + **TS3** — XS quick wins (alias consolidation, the instanceof bug) while context is warm.
4. **L4 → G4 → G5 (dispatch + overload)** — needs call-arg typing first; affects 4 axis-languages + overload mis-pick everywhere.
5. **G6 + L1 (F-bounded)** — generic substitution, then roll bound-capture across extractors.
6. **L2/L3 → G7 (narrowing / discriminated unions)**.
7. **G9 + L6 (closures)**, then **G10 + L7 (HKT)** last.
8. **R1** after the routing spine's validation.

Only **G1, G2, G3, G8** are pure generic wins with no per-language dependency.
Everything else is generic engine + a per-language feed, or TS-specific.

---

## Implementation status

Prior to this roadmap: bounded-generic / forward-inference / string-hop
(`f6f77196`), U1+U2a substitution-on (`d2cbec56`).

**Landed:**
- **G2 + G3** — multi-level generic inheritance composition (`a05235f5`).
- **G6** — F-bounded member resolution already worked via the fix-#2 leaf +
  gap-B bound capture + the `Apply` arm; verified by a test, no new prod code.
  Fires for `extends`/`:`-bound languages.
- **TS3** — `instanceof` guard query no longer matches `===` (`a05235f5`).
- **C1** — stale comments corrected (`a05235f5`).
- **L1 (C# / Go / Rust `where`)** — bound capture extended past the inline
  `<T extends B>`/`<T: B>` clause so G6 fires for these too. `merge_where_bounds`
  reads a trailing `where T : B` clause (C#, Rust single-line); the C# extractor
  now appends its dropped `type_parameter_constraints_clause`; the bracket parser
  reads Go's space-separated `[T Constraint]` and ignores `out`/`in` variance and
  `<T = Default>`. Haskell deferred — its params live outside any `<>`/`[]`
  clause (`(Ord a) =>` context), needing a separate extraction path for near-zero
  corpus payoff.
- **G5 (arity path)** — same-name overloads on one type are disambiguated by the
  call's argument count: `find_on_chain` prefers the candidate whose
  `signature_arity` matches an `arg_count` threaded down from the walker. The
  count comes from `ref_ctx.extracted_ref.call_args` on the final segment, used
  only when non-empty (empty is ambiguous — zero args vs. not extracted — so it
  stays first-match, no regression). The public `lookup` passes `None`. Type-based
  G5 (same arity, different param types) and **G4** (axis dispatch via
  `select_method`) still need full **L4** call-arg type inference.

**Deferred with reasons:**
- **G1 (D7)** — segment types intern nominal (`canonical_form.rs:197`), no
  `Cast` SegmentKind, turbofish/annotation share `type_args`, method-vs-class
  owner interacts with U2a. Needs a design pass before coding.
- **G8** — subsumed by D9; the string `expand_alias` can't represent a union
  head, and a first-branch heuristic would be wrong.

**Remaining — each is a substantial effort gated on a per-language feed or new
machinery, not a single-session sweep:**
- **G4 + type-based G5** dispatch + overload — gated on **L4** (typing
  call-argument expressions), itself a real inference task; the selector
  (`dispatch.rs`) and the arity path of G5 are already built.
- **G7** discriminated-union — the largest non-HKT item, 5 subsystems. The
  foundational gap: `intern_type_str` falls back to `Class(input)`, so a union
  branch's discriminant field (`kind: "circle"`) is NOT a `Type::Literal` today
  (`Type::Literal` is minted only for call-args, `inference.rs:105`). A real win
  needs all of: literal-field capture (extractor + intern), **L3** flow
  discriminant extraction, a discriminant-carrying `Narrowing` (today
  `narrowed_type` is a flat `String`), walker threading, and Union-arm selection.
  Partial = inert. Note the `instanceof`/class narrowing case already works via
  the existing narrowing path — the gap is specifically property-discriminant
  guards (`s.kind === "..."`).
- **G9 / L6** closures — corpus-wide function-type parsing + a call-yield arm;
  yield-precision payoff (rarely new edges).
- **G10 / L7** HKT — new kinded `Type` variant; largest, rarest.
- **L1 (Haskell only)** — needs a Haskell-specific path to extract type vars +
  their `(C a) =>` constraint context (no bracket clause to key off);
  **L2** guard vocabulary across the 14 flow languages.
