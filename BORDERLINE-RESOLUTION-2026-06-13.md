# Borderline resolution — 2026-06-13

Closeout register of every reference category BearWisdom intentionally does **not** bind, with the principled reason and the concrete condition that would change the verdict.

The resolution contract is: **≥99% per language through the one generic engine, with documented borderline for what static resolution cannot honestly bind.** This document is that documentation. A category lands here when binding it would require either guessing between candidates (corrupting dead-code / diagnostics / reference queries) or source/data that does not exist on disk in the indexed checkout. Suppression is always structural; binding never guesses — it declines and is recorded here.

Cross-references: gap-closure master plan `GAP-CLOSURE-MASTER-PLAN-2026-06-12.md` (decision menu §1, tracks §2, impact math §5, status ledger §7–8). Decision numbers below refer to that plan's §1 menu unless noted.

---

## 1. Dart `.pb.dart` protobuf codegen — build-output dependency

**Category.** ~13k unresolved `package:appflowy_backend/protobuf/…` refs in the appflowy fixture.

**Plan refs.** Track B B-M3; Wave-5 backlog (§7, "dart `.pb.dart` codegen indexing"); outstanding fork B (§8).

**Why borderline.** These are references into **ungenerated build artifacts**. In the appflowy checkout:

- `.pb.dart` protobuf output is gitignored (`.gitignore:82` — `lib/protobuf`) **and never generated** — `protoc` was never run.
- The `.proto` sources are also gitignored (`**/resources/proto`), so there are **zero** `.proto` / `.pb.dart` / `.g.dart` / `.freezed.dart` files on disk.

There is nothing on disk to index. BearWisdom's dart discovery is **innocent**: the `.dart` suffix (`languages/dart/mod.rs:46`) already admits `.pb.dart`, and there is **no codegen-specific exclusion** anywhere in the engine. This is not an engine coverage gap — it is a fixture missing its generated sources. B-M3's bindable ceiling (≈4,155, the non-protobuf dart surface) is correct and stands; the ≈13k protobuf refs sit above that ceiling by definition.

**What would change it.** A separate **FIXTURE-PREP** task: fetch the upstream `.proto` sources, run `protoc` + `protoc_plugin`, and commit `lib/protobuf/**`. The `.dart` suffix then indexes the generated files with **no engine change**. This is fixture regeneration, not resolver work.

**Decision.** Close B-M3 as a **build-output-dependency borderline** (fork B, recommendation 2). Reopen only if the protobuf surface is wanted as a real resolution target, at which point the path is fixture regen, not the engine.

---

## 2. Lua / JS untyped dynamic dispatch — decision 4

**Category.** Untyped multi-candidate receivers: `self:emit_signal()`, dynamic `require`-routed dispatch (Bookshelf-through-dynamic-require). Lua bare-name method collisions.

**Plan refs.** Decision 4; Track A A-M6 (`core/chain.rs` flow); Track F F-L (lua ranking "bare collisions stay borderline"); §5 ("lua — dynamic self + bare-name collisions").

**Why borderline.** The generic engine binds receivers **only when flow-typed** — e.g. `local x = M.new(); x:method()`, where `x`'s type is established by an in-scope assignment. An untyped receiver (`self:emit_signal()`) or a receiver reached through a value-dependent `require` has **multiple candidate targets and no static evidence to pick one**. Binding it would be guessing; a wrong bind corrupts dead-code detection, diagnostics, and reference queries. The lua ranking ladder (receiver > alias > ambient) intentionally **declines** when only an ambiguous bare name remains rather than picking a candidate.

**What would change it.** Nothing static, by design — this is the line drawn in decision 4 ("Approve the line as drawn. Binding would be guessing"). Flow-typed receivers already resolve (A-M6); the residual is the genuinely untyped surface. It would only move if the source itself added a type annotation or a single-candidate narrowing that the flow pass can read.

**Decision.** Approved as drawn (decision 4). Flow-typed receivers bind; untyped dynamic dispatch and lua bare-name collisions are documented borderline.

---

## 3. Template / builder DSLs — decision 9

**Category.** Runtime-dispatched DSL surfaces that have no statically resolvable target symbol:

- **Liquibase / Grails builder DSL** — `column`, `changeSet`, etc., dispatched via Groovy runtime `methodMissing`. No method symbol exists; the name is interpreted at runtime.
- **Nix module-system argument reads** — module args are read at **eval time**; the binding is produced by the module system's fixpoint, not by a static declaration.
- **Blade / EJS / Handlebars framework helpers** — template-engine helper functions provided by the framework runtime, not defined in project source.
- **Ember `svg-jar`** — build-time helper with no in-source definition.

**Plan refs.** Decision 9; Track T (T2 GSP, T5a scss/html, T1 mdx/astro — the *bindable* siblings); I2/T3 ("Nix module args, Liquibase/Grails builder DSL → documented borderline"); §5 ("groovy/gsp runtime methodMissing DSL"); Wave-4 B-M6 (vuetify-plugin components + `$t`/`$emit` globals — "no config source / runtime contract").

**Why borderline.** Each of these resolves the call **at runtime** via reflection / `methodMissing` / eval-time fixpoint / framework helper registration. There is no static target symbol to point an edge at. Decision 9 explicitly splits these from the cases that *are* bindable because their targets are in-index: GSP standard `g:` taglibs ride profile data (Robot precedent, T2); scss `@include`→`@mixin`, html `<script>` host links, and MDX/Astro components get connectors (T1/T5a) because those targets are real indexed symbols. The runtime-dispatched DSL surfaces above have no such target.

Note the GSP outcome after T2: standard/logical `g:` tags brand as the `grails-taglib` external (correct — they have no in-index definition), uniquely-named **custom** taglibs now bind via `GspHooks::resolve_bare_post` (their closures are indexed), and the remaining runtime-`methodMissing` builder calls stay borderline.

**What would change it.** Per-framework: a config-driven or profile-data surface **when the targets are actually in-index** (the T-track approach — never a hand-maintained name list). Where the dispatch is genuinely runtime-only with no on-disk definition (Liquibase/Grails `methodMissing`, nix eval-time reads, `svg-jar`), nothing static resolves it; it would only move if the framework emitted a declaration the extractor could read.

**Decision.** Document all runtime-dispatched DSL calls as borderline (decision 9). Taglibs-with-indexed-targets and connector-reachable component/mixin/script targets are bound via profile data / connectors, not listed here.

---

## 4. C / C++ POSIX-on-Windows — D-M4

**Category.** Unresolved C/C++ refs to POSIX system headers (`/usr/include/**`) on the Windows index host.

**Plan refs.** Track D D-M4 ("documented borderline; locator correct; `/usr/include` absent on host; NO synthetic stubs").

**Why borderline.** The C/C++ external locator is **correct** — it walks real headers when they are present. On the Windows recapture machine the POSIX include tree (`/usr/include`) **does not exist**, so there is no source for the locator to walk. Per the ecosystem-files-are-locators-only contract, the engine does **not** synthesize stubs for absent headers — a synthetic POSIX surface would be a hand-maintained builtin list, which is explicitly prohibited. The unresolved refs are honest: the headers are not on this host.

**What would change it.** Index on a host where `/usr/include` is present (a Linux/POSIX machine or a mounted sysroot), or point the locator at an on-disk POSIX sysroot. Zero code either way — the locator already handles real headers.

**Decision.** Documented host-limitation borderline (D-M4). No synthetic stubs.

*Adjacent but distinct — Win SDK header admission.* The aspnetcore `cpp.type_ref` residual (~11.3k) is **not** this category and is **not** a borderline: it is a diagnosed generic-engine gap (the C/C++ external-binding model has no live production consumer — the path-keyed `SymbolLocationIndex` is reached only from tests; bare `cpp.type_ref` chain-misses never trigger the demand-pull). It is tractable as an include-driven external-admission feature and is scoped as its own generic-engine wave (fork C, §8), not closed here. A cheap adjacent cpp-extractor win (SAL / calling-convention macro suppression, ~740 false type_refs) is also engine work, not borderline.

---

## 5. MATLAB stdlib — install-conditional — decision 3

**Category.** `stdlib_builtin` MATLAB refs (matlab-platemo alone is 51,272 of the corpus `stdlib_builtin` bucket).

**Plan refs.** Decision 3; Track F (MATLAB row — "Locator correct, install-conditional"); §0 discovery 4 ("MATLAB's locator is correct but there is no install on the index machine — install-conditional borderline, NOT missing code"); §5 ("matlab — install-conditional").

**Why borderline.** The MATLAB locator is **correct** — it discovers the `toolbox/` tree via standard-path discovery (`MATLAB_ROOT` / `matlab -batch` / standard installs; the `BEARWISDOM_MATLAB_ROOT` env probe was removed in Wave 5 per decision 10). There is **no MATLAB install on the index machine**, so the toolbox source the locator would walk is absent. As with POSIX headers, the engine does **not** synthesize a stdlib surface — the historical `matlab_stdlib.rs` synthetic list is confirmed deleted and nothing reopens it (the stale comment at `matlab_runtime.rs:11` was corrected to present-tense in Wave 5).

**What would change it.** Install MATLAB base on the recapture machine (or point the locator at an on-disk `toolbox/` tree). Estimated ~0.55pt conditional corpus gain, **zero code** either way (decision 3).

**Decision.** Install-conditional borderline (decision 3). Install if cheap; otherwise documented here.

---

## 6. Prolog quoted-atom Scryer VM builtins — Wave-4 verdict

**Category.** 511 unresolved quoted-atom refs in prolog-scryer (172 distinct quoted targets); dollar-prefixed names such as `'$fast_call'`, `'$module_call'`, `'$prepare_call_clause'`, `'$fail'`, `'$get_cp'`.

**Plan refs.** Wave-4 scoped residuals (§7, "prolog quoted-atom records (511)"); §4 residuals (prolog quoted atoms NO-OP); Track K K1 (the *erlang* quoted-atom case this was initially mistaken for).

**Why borderline.** These are **Scryer's runtime VM ABI**, implemented in **Rust** (`Machine.fast_call` in `src/machine/system_calls.rs`), never defined as Prolog predicates. They are correctly external — there is no Prolog definition to bind to. This is **not** the erlang quoted-atom shape (K1): both sides quote consistently here (Prolog names symbols quoted, with arity — `'$default_attr_list'/2`), so the matched-pair quote-strip that fixed erlang does not apply. Decisive measurement: **0 of 172** distinct quoted targets gain a match under either a quote-strip or a both-sides quote-normalization. Names like `'$fail'` / `'$get_cp'` look strippable but `$fail` ≠ `fail` (different predicates) — stripping `$` would be a **false bind**.

**What would change it.** Nothing in static Prolog resolution — these names have no Prolog-source definition; they are the Rust-hosted runtime surface. They would only become a resolution target if the Scryer Rust runtime were itself indexed as a cross-language external, which is out of scope.

**Decision.** NO-OP / correctly-external borderline (Wave-4 verdict). No extractor change.

---

## 7. Per-language residual caps named by the plan

These are the residuals the plan calls out as borderline-documentable once each language's mechanism work has landed (§5, "Languages whose residual is then borderline-documentable"). They overlap the categories above; collected here for the per-language gate accounting.

| Language | Residual category | Reason | Section |
|---|---|---|---|
| **lua** | Untyped dynamic `self:` dispatch + bare-name method collisions | Multi-candidate, no static evidence; binding would guess | §2 (decision 4 / F-L) |
| **nix** | Module-system arg reads | Eval-time fixpoint injection; no static declaration | §3 (decision 9 / I2/T3) |
| **matlab** | stdlib / toolbox refs | Locator correct, no install on index host | §5 (decision 3) |
| **groovy / gsp** | Runtime `methodMissing` builder DSL | Runtime-dispatched; no target symbol (custom taglibs with indexed closures now bind via T2) | §3 (decision 9) |
| **c / cpp** | POSIX-on-Windows headers | Locator correct, `/usr/include` absent on host; no synthetic stubs | §4 (D-M4) |
| **prolog** | Scryer `'$…'` VM builtins | Rust-hosted runtime ABI; correctly external | §6 |
| **dart** | `package:…/protobuf/…` (`.pb.dart`) | Ungenerated build output; nothing on disk to index | §1 (fork B) |

Languages **not** documented borderline (pushed to or near ≥99 by mechanism work per §5): pascal, typescript, dart (non-protobuf surface), go, svelte, erlang, haskell, bicep, java. The 14 baseline ≥99 languages are the regression floor (untouched by inert profile axes).

---

## 8. No-candidate frontier (not yet borderline — the long-term margin)

The plan's §5 names a residual ~10–14% of current unresolved as the "no-candidate" frontier: macro expansion, build-step codegen, remote flake inputs. This is **not** closed as borderline here — the plan's stated policy is that each item "either gets a locator when real source exists on disk, or a documented cap." It is recorded as the frontier so the per-language gate accounting does not silently absorb it. Items graduate into this document (with a concrete change-condition) only as each is individually adjudicated.

---

## Borderline categories covered

1. Dart `.pb.dart` protobuf codegen — build-output dependency (fork B)
2. Lua / JS untyped dynamic dispatch (decision 4)
3. Template / builder DSLs — Liquibase/Grails, nix module args, blade/ejs/handlebars, ember svg-jar (decision 9)
4. C / C++ POSIX-on-Windows host limitation (D-M4)
5. MATLAB stdlib — install-conditional (decision 3)
6. Prolog quoted-atom Scryer VM builtins (Wave-4 verdict)
7. Per-language residual caps (gate accounting; overlaps §1–6)
8. No-candidate frontier (not-yet-borderline; recorded for accounting)
