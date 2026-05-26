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
- **G7 (TS, `if`/`switch` equality guards, named branches)** —
  discriminated-union branch selection now works end-to-end; the absent
  foundation was built. (1) `push_ts_field` stores a `literal_type` annotation
  (`kind: "circle"`) on the field's *signature* (no unresolvable TypeRef). (2) A
  `discriminant_guard_query` on `FlowConfig` extracts both `if (x.kind === "lit")`
  and `switch (x.kind) { case "lit": }` into a new `DiscriminantNarrowing`
  (separate `FlowMeta` channel, so no `Narrowing`-struct ripple; `!==` is
  intentionally unmatched; a `switch_case` node's range scopes each case). (3)
  `LocalTypeCache` carries them; `SymbolLookup::local_discriminant` surfaces the
  active `(prop, literal)`. (4) The walker's `narrow_union_by_discriminant`
  replaces a `Type::Union` receiver with the branch whose discriminant member's
  signature equals the literal, before the segment loop.
- **Anonymous-object-type union members now resolve** — `classify_alias_target`
  drops anonymous `object_type` branches (`head_type_name` is empty for them),
  which left `type S = {kind:"a";x}|{kind:"b";y}` as an empty `AliasTarget::Union`
  that resolved nothing. But `recurse_for_object_types` already flattens those
  members under the alias, so an all-anonymous-branch union (with ≥1 `object_type`
  branch and no nameable branch) now classifies as `AliasTarget::Object`: the
  alias stays `Class(S)`, its flattened members key under `Class(S)`, and `s.x`
  resolves. Primitive/literal unions (`string|number`) stay `Union`. This gives
  resolution, not branch precision (the union collapses to a flat object).
- **L2 (C# declaration-pattern narrowing)** — `if (x is Foo f) { f.Bar() }` now
  narrows the binding `f` to `Foo` in the block (a second `type_guard_query`
  pattern on the `declaration_pattern`, reusing the class-narrowing machinery).
  C# previously handled only the bindingless `if (x is Foo)`. Remaining L2:
  Kotlin smart-casts (grammar node names version-dependent, flagged in
  `kotlin/flow.rs`), Go type-switch, Rust `if let`/`match`, Ruby `is_a?`.

  Remaining G7 scope, each a foundation-build:
  - **anonymous-branch precision** — narrowing an anonymous union to the *right*
    branch still needs synthetic per-branch identities (kept as a `Union`); the
    above gives flat resolution only.
  - **early-return narrowing** (`if (s.kind !== "x") return;` then `s` narrowed)
    — control-flow analysis, not a lexical block scope.
  - **non-TS** (Rust `match`, Kotlin sealed `when`, Scala) — per-language
    `discriminant_guard_query` + literal-field capture; Rust enums are a
    different shape (variants, not a `kind` field).

- **G9 / L6 (TS/JS, arrow form, mid-chain)** — closure call-yield landed. The
  absent foundation — `ChainSegment` had no call marker — was added: an `is_call`
  field set by the chain builder on the `function` child of a `call_expression`
  (common.rs for JS, typescript `build_chain` for TS). `intern_type_str` parses a
  top-level `=>` into `Type::Function { return_ }` (depth-tracked so a nested
  arrow isn't mis-read), and `yield_type_of` peels a function-typed *value*
  member to its return type when invoked (a method's type is already its return,
  so it's untouched). So `obj.handler().name` resolves `name` on the handler's
  return type. Remaining: other languages' own chain builders
  (Go/Kotlin/Scala/Swift/C#), the root-call form (`f().x` — root resolver interns
  via `class`, not `intern_type_str`), and non-arrow syntaxes (Rust `Fn`,
  Kotlin `(T)->R`).
- **G10 (HKT resolution)** — already works via the existing machinery; no kinded
  `Type` variant needed. A higher-kinded application `F[A]` is just
  `Apply { base: Generic(F), args: [A] }`: `rebind_class_params` canonicalizes
  the Apply *base* to `Generic(F)`, and `substitute` rewrites that base, so
  `F[A]` with `F → List` becomes `List[A]`. Verified by tests. Arity (`F[_]`,
  the L7 piece) matters only for *kind-checking* (is `List` a valid `F[_]`?),
  not for resolution — so it stays out of scope.

**Deferred with reasons:**
- **G1 (D7)** — segment types intern nominal (`canonical_form.rs:197`), no
  `Cast` SegmentKind, turbofish/annotation share `type_args`, method-vs-class
  owner interacts with U2a. Needs a design pass before coding.
- **G8** — subsumed by D9; the string `expand_alias` can't represent a union
  head, and a first-branch heuristic would be wrong.

**Remaining — each is a substantial effort gated on a per-language feed or new
machinery, not a single-session sweep:**
- **L1 (Haskell only)** — needs a Haskell-specific path to extract type vars +
  their `(C a) =>` constraint context (no bracket clause to key off).

---

## Session 2026-05-26 — dispatch front + narrowing vocab + closures landed

**Landed (with commits):**
- **F1** (`085f3f24`) — per-language primitive disjointness, compare-time in
  `is_assignable_to_typed_with(.., prims)` bridging `Primitive` and nominal
  primitive-named `Class` via the profile `primitive_mapping`. Chosen over the
  roadmap's intern-time minting to avoid touching every extractor + the
  Class/Primitive dedup/round-trip risk. This is the unblock the prior
  "Remaining" note demanded (no `Type::Primitive` interning needed).
- **L4** (`00c48cb2`) — `resolve_arg_types` maps `call_args` → TypeId (literals →
  primitive, ident → `local_type`, else Unknown).
- **G4** (`00c48cb2`) — non-receiver axes route the final call segment through
  `select_method`; receiver axis stays on direct lookup (no big-corpus reroute
  risk). `select_multi_arg` falls back to receiver dispatch on no arg-type match.
- **G5-typed** (`00c48cb2`) — `find_on_chain` picks the same-arity overload whose
  param types accept the arg types (`type_hit > arity_hit > first`); additive,
  single-candidate sites unchanged. `args_assignable` moved to `subtype.rs`.
- **L5** (`134cfaad`) — generic most-specific multi-arg selection (CLOS-style),
  not per-language hooks (the algorithm is language-agnostic).
- **L2** (`637ea5f5`) — Java `instanceof Foo f` pattern binding, Ruby `kind_of?`,
  TS `typeof x === "…"`, Go `switch v := x.(type)`. TS/Go flow are gated off in
  prod (BW_TS_FLOW / Go OOM) so those two are dormant-but-ready; their flow
  modules are now `pub(crate)` for direct-static tests.
- **L6** (`2c20e746`) — root-call form `f().x`; `is_call` marked in Go/C#/Kotlin/
  Scala/Swift chain builders; `intern_type_str` parses `->` (Rust `Fn()->T`,
  Kotlin/Swift `(T)->R`; Scala `=>` already worked).
- **TS1** (`36a839a2`) — `keyof T` expands to a string-literal union from the
  target's member names.
- **G1 (turbofish)** (`636f9aba`) — method-own generics bind on the *primary*
  yield path. `owner_param_type_map` merges method + class params; the primary
  path rebinds the param-blind stored return (`Class("U")`) before substituting
  (mirroring the string path); a turbofish segment binds the method's own params
  to the call-site type args. Also repairs inherited-generic substitution on the
  primary path in production.

**Remaining — gated; needs an architect decision, a foundation, a lifted prod
gate, or the closeout recapture (NOT autonomous single-session work):**
- **G1 (D7) — complete for the resolution-meaningful parts.** Turbofish +
  primary-path canonicalization (`636f9aba`) and cast adoption (`1a7701d8`,
  TS/Rust/C#) landed — the two directions that change *mid-chain* resolution
  (a bound generic consumed by a later segment; a receiver retyped by a cast).
  The remaining LHS-annotation expected-type direction (`const x: User =
  repo.find()` binds `find()`'s unbound yield from `User`) is **subsumed for
  resolution**: it would only refine `resolved_yield_type` on the *final,
  unchained* segment, and the annotated LHS is already typed directly from
  `flow_binding_decl_type` into the local cache — so the refinement is
  redundant (and its source is flow-gated, dormant for TS). Not worth the
  RefContext→loop→walker plumbing for zero observable resolution gain.
- **G7 anonymous-branch precision / early-return narrowing — dormant + foundation.**
  Both extend the discriminant-narrowing path fed by TS `flow_config`, which is
  gated off in prod (BW_TS_FLOW, ts-immich hang). Early-return also needs new
  negation + post-block-scope narrowing semantics; anonymous-branch needs
  synthetic per-branch type identities (extractor change). Dormant until the TS
  flow gate lifts.
- **TS2 mapped beyond transparent — foundation.** Record value-type / key-remap /
  value-template need an index-signature type in the arena + extractor capture +
  member-lookup support. New machinery for a case (mapped types as chain
  receivers) that's rare in real code.
- **L3 non-TS discriminant — grammar-uncertain / low value.** Rust enums are a
  different shape (variants, not a kind field; `declared_type` already precise);
  Kotlin/Scala pattern-match guards face the same version-dependent grammar
  uncertainty as the deferred L2 Kotlin smart-cast.
- **R1 external arrow-return ungate — closeout-coupled.** `build.rs:370` /
  `augment.rs:190` gate hydration that "shifts the type maps" corpus-wide,
  deliberately behind `BEARWISDOM_COMPILER_RESOLVE` for A/B. Flipping it blind
  (mid-flight, unmeasurable) defeats the A/B; do it *with* the closeout recapture
  so its delta is observable.
- **L2 (Kotlin smart-cast, Rust if-let/match) / L1 (Haskell)** — grammar-uncertain
  or near-zero corpus payoff; left with their existing in-code rationale.
