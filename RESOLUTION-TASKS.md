# Resolution → 100% — master task list

Single source of truth for getting every language to 100% resolution on the
engine-only resolver. **Generic first, language second.** Work top-to-bottom;
don't re-discuss ordering per task.

## Why progress has felt non-palpable

Each past fix moved ~0.1–0.5pp because the remaining ~730k unresolved refs are
**distributed across ~10 distinct shapes**, and several "fixes" were refactors
(no resolution gain) or hit shapes that were already mostly resolved (ambient
test globals — verified already-working in 7 TS projects this session). To get
palpable movement we must attack the **big concrete pools**, not polish working
mechanisms. Five generic tasks (G1, G2, G6, G8, G9) cover the majority of the
unresolved mass.

## How to execute each task

Every task is independently shippable and follows the same loop:

1. **Diagnose** — confirm the lever on 1–2 real project `.bearwisdom/index.db`
   (SQL templates in `.claude/rules/sql-investigation.md`). Don't build until
   the diagnostic proves this shape is the blocker. This step replaces the
   re-discussion — it's a defined, bounded check, not an open debate.
2. **Implement** — at the seam named in the task. Generic seam unless the task
   is in the L-series.
3. **Verify** — single-project recapture on the named project, assert the
   target count drops and no regression. (`bw quality-check --recapture
   --project <name>`.)
4. **Mark done** here with the before→after on the probe project.

**Full-corpus recapture runs ONCE, at the end** (per the one-recapture rule).
Per-task verification is single-project only.

## Seam legend

Each task names the **trait/struct it adds or modifies** and the **layer**. The
"generic first" rule means: exhaust generic-engine + per-ecosystem changes
before any per-language change.

- **generic engine** — one change helps every language:
  - `DefaultResolver` strategy tower — `type_checker/core/default_resolver.rs`
  - `SymbolLookup` trait — `indexer/resolve/engine/lookup.rs` (+ prod impl
    `index/lookup_impl.rs`, build in `index/build.rs`)
  - chain walker / `RootResolver` — `type_checker/chain.rs`,
    `type_checker/core/chain.rs`, `type_env.rs`
  - scope-tree builder — `parser/`
- **per-ecosystem** — one change per dependency world (npm, Maven, CRAN…):
  - `Ecosystem` trait impls + manifest readers — `ecosystem/`
- **per-language** — touches one language only:
  - extractor — `languages/<lang>/extract.rs`
  - `LanguageEngineHooks` impl — `languages/<lang>/hooks.rs`
  - `locals.scm` tree-sitter queries; `LanguageProfile` data

---

## DONE (this session + prior)

All three new strategies are *generic engine* — `DefaultResolver` strategies in
`type_checker/core/default_resolver.rs`; the `path_alias` rename was on the
generic `SymbolLookup` trait.

- `resolve_via_self_keyword` — `this`/`self`/`Self`/`super` → enclosing type / parent. (G3)
- `resolve_via_enclosing_member` — inherited members via `parent_class_qname` climb. (G4)
- `resolve_via_aliased_import` — import specifier via path alias. (G5 strategy half)
- `path_alias` rename — TS-name purged from the generic `SymbolLookup` contract.
- Phases 2–5 — transitive re-export closure, inheritance walk, wildcard import,
  generated-code adapters (Prisma/protoc/OpenAPI), jar walker, manifest readers.

✅ **Session 2026-05-25 — 11 verified tasks, closeout recapture running:**
- G3/G4/G5 strategies: java-recaf 96.85%→**97.2%**; ts-ever-demand 92.3%→**95.5%** (2,256 path-alias refs).
- **svelte** hooks wiring: **1.38%→95.6%** (corpus svelte unresolved 16,040→1,677).
- **astro** hooks wiring: **~0%→~88%** (399→77).
- G8 C++ generics: lua-koreader 1,076 cpp refs (partial; member-scope residual = G2).
- G7 operator suppression: gleam 108→0 (generic across gleam/haskell/fsharp/scala).
- L2 VB.NET operator keywords: vbnet-compactgui 999→826.
- G11 ambient-path de-sprawl (behavior-preserving).
- **G1a vitest globals** (gate fix): ruby-chatwoot 3,917→0 — corpus-wide.
- **G1a-2 `@types/*` ext-path ambient** (one-line fix): paperless 1,315→0,
  project TS ~99.5% — **corpus-wide** (jest/node/mocha/jasmine globals across
  ALL TS/JS projects). Likely the highest-leverage fix of the session.
- Reverted (net-zero/honest): G6 go-locals (→G2), swift/groovy G12.
- **Closeout full-corpus recapture in progress** to realize the corpus-wide
  G1a + G1a-2 gains and validate the generic `is_ambient_path` change.
  Update `baseline-all.json` / `baseline-by-so2025.md` / `baseline-gaps.md`
  from its result.

---

## GENERIC TASKS (do these first)

### G0 — Establish the measurement loop
- **Seam:** none — tooling (`bw quality-check --recapture`). No trait.
- **Closes:** nothing directly; unblocks honest tracking.
- **Do:** build `bw` CLI; recapture one Java project (`java-recaf`) and one
  vite project to measure G3/G4/G5. Record the per-language deltas.
- **Result ✅ (2026-05-25):** `java-recaf` 96.85% → **97.2%** (+0.35pp,
  ~425 refs) from M-E/M-F/M-A — measurable positive movement, no regression.
  Recapture log confirms **G1b**: 28 JVM deps (junit/assertj/…) have no
  `-sources.jar`, so their symbols can't be indexed → `assertTrue` etc. stay
  unresolved until `mvn dependency:sources` is run. ts-ever-demand pending.
- **Verify:** numbers land in the tracker; no regression vs `baseline-all.json`. ✅

### G1 — Externals reachability for dev/test/uninstalled deps
- **Seam:** *generic engine* — modify externals reachability BFS seeding
  (`indexer/expand.rs` + manifest dep scope) so dev/test-scoped deps seed the
  walk; *per-ecosystem* — C/C++ header walking in that ecosystem's `walk_root`.
  No new trait. G1b is corpus prep (no code).
- **Closes (large):** JS `it`(3,146)/`describe`(1,416)/`$`(5,811 jQuery); C
  `CURL`/`CURLcode`/`curl_easy_*`(~5k libcurl headers); Java `assertTrue`(1,594)
  /`assertThat`/`isEqualTo`(716 AssertJ)/`andReturn`(1,210 Mockito); Kotlin
  `hasSize`(1,284)/kotest; PHP `assertEquals`(1,154 PHPUnit); Lua busted globals.
- **Where:** `ecosystem/` BFS seed + manifest readers; install-missing-deps.
- **Sub-tasks:**
  - **G1a** Extend externals reachability BFS to include **dev/test-scoped**
    manifest deps (devDependencies, Maven test scope, pip extras). Today the
    seed skips them, so vitest/jest/junit symbols never enter the index in
    projects that need them.
  - **G1b** Install missing test-project deps (per `feedback_install_dev_deps`):
    `npm install`, `pip install`, download `-sources.jar` for AssertJ/kotest.
    Corpus-prep, not code — but required for G1a to have symbols to reach.
  - **G1c** C/C++ system-header externals: index referenced `<curl/curl.h>`,
    `<lua.h>` etc. via `compile_commands.json` `-I` paths (mechanism exists for
    C/C++ — confirm it walks the headers, not just the project).
- **Diagnosed ✅ (2026-05-25):** kotlin-ktor has **0** junit/assertj/kotest/mockk
  external files indexed vs **3,230** other externals → the Maven externals
  pipeline works; test-scoped deps are excluded by the BFS. ~1,166 unresolved
  asserts in this one project (assertTrue 745, assertNotNull 230, assertFalse 191).
- **Verify:** kotlin-ktor `assertTrue`/`hasSize` count drops after G1a+G1b.
- **G1a vitest-globals fix ✅ implemented (2026-05-25, recapture pending):**
  found a concrete, contained slice. Projects with vitest `globals: true` inject
  `describe`/`it`/`expect` ambiently; `node_modules/vitest/globals.d.ts` declares
  them via `declare global { … }` but was **never indexed** — the existing
  `demand_pre_pull_test_globals` gate (`package_declares_globals`) only checked
  the package's *entry* `.d.ts`, and vitest's entry has no `declare global`
  (only the separate `globals.d.ts` does). **Fix:** added `globals.d.ts` /
  `dist/globals.d.ts` to `candidate_globals_entry_files` (`ecosystem/npm/mod.rs`)
  so the gate passes → the probe indexes it → `is_ambient_path` (globals.d.ts
  rule) + `resolve_via_ambient_package` resolve the globals. +1 test (6/6 pass).
  Pre ruby-chatwoot: **3,917** unresolved test-globals (node_modules present).
### G1a-2 — `@types/*` ambient path missed for externals-index format  *(corpus-wide bug)*
- **Seam:** *generic engine* — `SymbolIndex::is_ambient_path` (`classify.rs`).
- **Found (2026-05-25):** jest/node/mocha globals come from `@types/*`, which
  TS auto-includes as ambient. `is_ambient_path` matched `@types/` only as
  `/@types/` — but externals are indexed as `ext:ts:@types/jest/index.d.ts`
  (colon before `@types`, NO leading slash). So **every `@types/*` package in
  the externals index was missed by the ambient check** → their globals
  (`describe`/`it`/`expect`/`process`/`Buffer`/…) never resolved via
  `resolve_via_ambient_package`. (vitest escaped only because its path ends
  `/globals.d.ts`, a different rule.) Confirmed: paperless-ngx has the
  `describe` symbol indexed from `ext:ts:@types/jest/index.d.ts` but 1,315
  unresolved refs.
- **Fix ✅ (recapture pending):** match `@types/` as a path component —
  added `:@types/` and `@types/`-prefix forms. One line. **Corpus-wide** —
  affects every TS/JS project using `@types/jest`, `@types/node`,
  `@types/mocha`, etc. ambiently. Verify on paperless (1,315). Generic change →
  closeout full recapture validates breadth + no regression.
- **RESULT ✅ (2026-05-25):** paperless-ngx jest globals **1,315 → 0**; project
  TS **59,908 resolved / 284 unresolved (~99.5%)**. Corpus-wide @types globals
  (jest/node/mocha/jasmine) now resolve. Likely the highest-leverage fix of the
  session. Done.
- **Follow-on (G1a-3, same vein):** `is_ambient_path` item 1 (tsconfig `types`
  match) also uses a `node_modules/<pkg>/` needle that misses the `ext:ts:…`
  format — same ext-path bug. Lower priority (explicit `types:[…]` is rarer).

- **RESULT ✅ (2026-05-25):** ruby-chatwoot test-globals **3,917 → 0**;
  `vitest/globals.d.ts` now indexed (`ext:ts:vitest/globals.d.ts`),
  `describe`/`it`/`expect` resolve as ambient. **Corpus-wide** — every
  vitest-`globals:true` project benefits (Vue/TS projects especially). A
  one-line gate fix. Done.

### G2 — Receiver-type → method-chain resolution  *(biggest, hardest)*

**STATUS (2026-05-25): receiver-chain layer built + tested; ceiling reached on
the rust probe.** Three increments landed, all with sibling tests:
- **Bug B — generic-type member dual-keying** (`type_checker/core/members.rs`,
  *generic engine*): `MembersIndex` keyed members of `IndexWriter<D>` only under
  the parameterized scope, so a receiver normalized to the bare base never
  matched. Now dual-keyed (bare + parameterized). rust-tantivy engine_chain
  +548; zero regression.
- **Bug A — `let x: T` annotation capture** (generic flow machinery:
  `indexer/flow.rs` `@type` capture + per-language `flow.scm` + resolve-loop
  seeding of the LocalTypeCache): annotated locals are typed without resolving
  the initializer. `add_document` 0→473 type-driven (was name-fallback).
- **Gap #3 — `?`-unwrap peel** (`@rhs_unwrap` query capture + `first_generic_arg`
  peel at record-time): `let x = expr()?` records `T`, not `Result<T>`.
  Mechanism done + tested; no rust-tantivy movement (see finding 2).

**Two findings that reframe G2's measurement:**
1. **Name-fallback strategies prop the rate.** rust-tantivy sits at 97% largely
   via `rust_same_file_name_fallback` (resolves by name, no type check). G2
   upgrades those to correct type-driven resolution (right overload) but the
   *rate* barely moves — G2 is a **precision** win on such projects, not a rate
   win. Pick a probe whose unresolved chains are genuinely unresolved.
2. **The residual is inference-bound, not receiver-bound.** rust-tantivy's
   remaining chains are `let reader = index.reader_builder()…try_into()?` /
   `collect::<T>()` — the type comes from the *expected type of the binding*,
   i.e. bidirectional (Hindley-Milner) inference, a separate and larger task
   than receiver-type chain walking. G2's receiver layer has hit its ceiling
   here.

**Remaining G2 work:** G2b (external stdlib in MembersIndex — *gated on the
resource-monopoly constraint, discuss first*); expected-type inference (new,
larger); per-language reference-type strip (`-> &Schema` → `Schema`, Rust
extractor). Below is the original plan.

- **Seam:** *generic engine* — populate the existing `SymbolLookup`
  `return_type`/`field_type` maps for externals in `index/build.rs`; wire
  `type_env.rs` substitution into the chain walker (`type_checker/chain.rs`).
  No new trait (accessors exist). Needs *per-language* extractors to emit
  signature TypeRefs for external symbols (G2a) and *per-ecosystem* stdlib
  indexing (G2b).
- **Closes (large):** rust `map`/`clone`/`push`/`to_string`(~2k+ Iterator/Option/Vec);
  ocaml `fold_left`/`iter`/`length`/`printf`(~1.6k stdlib); dart `all`/`only`/
  `symmetric`; TS/JS `findOne`/`dispatch`/`toPromise`/`lean` (Mongoose/RxJS
  builder chains); go `Request`/`Client` typed receivers.
- **Where:** chain walker (`type_checker/chain.rs`) + externals `return_type`/
  `field_type` maps (the gating constraint in CLAUDE.md) + `RootResolver`.
- **Sub-tasks:**
  - **G2a** Emit external method `return_type` / field `field_type` edges from
    extractor signatures so they survive the resolve-loop `ext:` filter and
    populate `SymbolLookup::return_type_name`/`field_type_name`. **This is the
    keystone** — without it, no chain hops through library APIs.
  - **G2b** Stdlib-as-externals receiver coverage: ensure rust-std/ocaml-stdlib
    types (`Vec`, `Option`, `List`) are indexed with their methods so the
    receiver-type lookup finds them.
  - **G2c** Generic substitution through the chain: `Repository<User>.findOne()`
    → `User` (the `type_env.rs` substitution wired into external return types).
- **Diagnose:** rust project; confirm `Vec`/`Option` method symbols exist as
  externals but their `return_type` map is empty.
- **Verify:** rust unresolved `map`/`clone`/`push` count drops.

### G6 — Scope-local binding completeness
- **Seam:** *generic engine* — new build.rs repo-local locals.scm override
  mechanism (`queries_locals/<lang>.scm`); *per-language* — the query itself.
  No trait.
- **Go attempted, REVERTED (2026-05-25) — needs a dedicated debug session:**
  diagnosis stands — tree-sitter-go ships no `queries/locals.scm` so go had
  `LOCALS_SCM=None`, and **every** local (`x := …`, range vars, params) leaks
  as a `calls` ref (go-pocketbase `expected` 2,421, `email`/`record`/… 100s).
  Built: (1) build.rs `queries_locals/` override mechanism, (2) `go.scm` (scope
  + def + ref captures), (3) compile+resolve test — **all pass in isolation**
  (query compiles against the grammar, resolves `expected` in a snippet).
  **But it did NOT move go-pocketbase's counts** across three recaptures
  (identical 2,421). Two blockers found, second undiagnosed:
  1. Go's extractor attaches a `chain` to *every* `Calls` ref (`go/refs.rs:109`),
     and `filter_local_refs` keeps all chain-bearing refs — so locals.scm can
     never touch them.
  2. Relaxing the keep-rule to *single-segment* chains (a sound generic change)
     **still** didn't move the count → a further issue (likely `LocalResolver`
     resolving 0 on real go files, or these refs bypassing `filter_local_refs`).
  Reverted the whole attempt (build.rs override, go.scm, test, chain relaxation)
  to keep the tree clean — no unverified/inert code.
- **MISDIAGNOSIS CORRECTED ✅ (2026-05-25):** the go `expected` mass is **NOT
  locals.** Source inspection (`core/otp_query_test.go:58`) shows they are
  `s.expected` — **struct-field access on a range variable** (`for _, s := range
  scenarios` where `scenarios` is `[]struct{…; expected []string}`), emitted as a
  2-segment chain `[s, expected]`. `LocalResolver` correctly does NOT treat them
  as locals (verified: it resolves real `:=` locals fine). These need
  **range-variable type inference + anonymous-struct field resolution** — a
  **G2 (chain/type-inference)** problem, Go-flavored. **G6/locals.scm applies to
  far fewer go refs than the gap report implied;** the headline counts
  (`expected`/`email`/`record` = `s.field`) belong to G2. Re-scope G6 down and
  fold the go field-access mass into G2.
- **Closes:** go `expected`(2,477)/`expectedErrors`/`expectedConfig`; fortran
  `ptr`/`data`/`x1`; python `_`(763); swift locals. These should never reach
  the resolver — they're locals/params the scope builder didn't bind.
- **Where:** scope-tree builder + per-language `locals.scm`. Generic mechanism,
  per-language data.
- **Diagnose:** go-pocketbase; confirm `expected` is a `:=` local in a
  table-driven test that `locals.scm` isn't binding (vs a struct field).
- **Verify:** go `expected` unresolved count drops to ~0.

### G8 — Generic/template-parameter capture
- **Seam:** *per-language* extractor (`languages/<lang>/extract.rs`) — populate
  `ExtractedSymbol.generic_params`. Consumer
  `DefaultResolver::resolve_via_generic_param` is already *generic engine*. No
  new trait — pure extractor capture feeding an existing strategy.
- **Implemented ✅ (2026-05-25, pending verify) — two parts:**
  1. *per-language* (`c_lang/templates.rs`): `push_template_decl` parsed the
     param list for default-type TypeRefs but never recorded param *names*.
     Added `collect_template_param_names` → **folds them into the inner
     symbol's signature** as `<T, charT>` (NOT the `generic_params` field —
     that's interned `GenericParamId`, unavailable at extract time). The index
     build already parses generic params out of signatures into the string map.
  2. *generic engine* (`default_resolver.rs`): `resolve_via_generic_param` now
     also checks the **source symbol's own** `generic_params(qname)` string map.
     The scope-chain path deliberately skips the source qname, and the arena
     path only fires for extractors that intern IDs — so a template *function*'s
     own-signature params (`OutputIt fill_n(OutputIt …)`) were unreachable.
     Benefits every language whose generics live in the signature.
  - Tests: +2 in `c_lang/extract_tests.rs`, +1 in `default_resolver_tests.rs`. 42/42 + 10/10 green.
  - **RESULT ✅ partial (2026-05-25):** lua-koreader recapture — `engine_generic_param`
    resolved **1,076 cpp refs**. T1 1,687→848 (−50%); lString16 1,355→1,355
    (unchanged ✓ — scope correct, concrete types untouched). T2/charT barely
    moved.
  - **Residual (separate, deeper C++ task):** remaining T2/charT live in
    **fields/typedefs/variables inside template classes** (`re_bmh<charT,
    utf_traits> bmdata`) — the member's `scope_path`/scope-chain doesn't connect
    to the enclosing template's params. This is C++ member-scope-tree depth in
    `c_lang`, not the generic resolver. File under a focused C++ session.
- **Closes:** cpp `T1`(1,692)/`T2`/`charT`/`OutputIt`/`iterator_t`(~6k);
  any language whose extractor drops type params.
- **Where:** per-language extractor — capture `template<typename T>` /
  generic-param declarations into `ExtractedSymbol.generic_params`. The
  resolver consumer (`resolve_via_generic_param`) **already exists**; this is
  pure extractor capture.
- **Confirmed gap:** `c_lang` sets `generic_params: Vec::new()` everywhere.
- **Diagnose:** none needed — code-confirmed this session.
- **Verify:** cpp `T1`/`charT` unresolved count drops (cpp-fresh / zig-compiler).

### G9 — Bare type_ref resolution in monorepos  ✅ LARGELY DONE (by G5)
- **Seam:** *generic engine* — no new code. The existing
  `DefaultResolver::resolve_via_aliased_import` (G5, built this session) +
  `SymbolLookup::resolve_path_alias` resolve the `@modules/*` path-alias
  imports. Residual is *per-ecosystem* path-alias coverage for monorepos whose
  alias config isn't parsed yet.
- **RESULT ✅ (2026-05-25):** ts-ever-demand recapture — `engine_aliased_import`
  resolved **2,256** of these refs. Order 1,270→28, Carrier 417→15, Product
  581→188, User 457→220. Project rate **92.3% → 95.5% (+3.2pp)**. Confirms G5
  is the lever; G9 needed no separate work.
- **Closes:** TS `Order`(1,270)/`Product`(581)/`User`(457)/`Carrier`(417);
  `FormlyFieldConfig`(890). **Not decorator/DI — that label was wrong.**
- **Diagnosed ✅ (2026-05-25):** in ts-ever-demand every `Order` ref is a **bare
  `type_ref` with no import module**, the internal `Order` *class exists* with a
  `package_id`, 14 workspace packages are detected, and there are **multiple
  same-named internal candidates** (entity class + `.tsx` component + type alias
  + graphql type). So: `resolve_via_unique_internal_name` bails (ambiguous) and
  the bare ref doesn't connect to the right candidate across workspace packages.
- **Import shape ✅ (2026-05-25):** dominant form is
  `import Order from '@modules/server.common/entities/Order'` (a **tsconfig
  path-alias**, `@modules/*`) plus relative `import Order from '../entities/Order'`
  — **all DEFAULT imports** (`import X from`, not `import { X }`).
- **Ties to G5:** the `@modules/*` path-alias imports are the exact target of
  `resolve_via_aliased_import` (built this session). The 1,270 count was measured
  on the OLD index — recapturing ts-ever-demand tests whether G5 already closes
  them. Two variables it settles:
  1. does `@modules/` resolve through the `path_aliases` map (is it a captured
     tsconfig `paths` entry)?
  2. ~~default-import capture~~ **RULED OUT ✅** — `build_file_context_inner`
     collects every ref-with-module into the import table; default imports
     emit `{target_name:"Order", module:"@modules/…"}` (hooks.rs:1061). So the
     import table HAS `Order → @modules/server.common/entities/Order`.
- **Reduces to path-alias coverage:** relative `../entities/Order` should
  already resolve via `file_import` (stem-match). The dominant `@modules/*`
  form needs `resolve_via_aliased_import` (G5) + `@modules/*` present in
  `path_aliases`. If recapture doesn't drop the count, the fix is ensuring
  `@modules/*` is parsed into `path_aliases` (tsconfig `paths` / vite alias)
  for this project — a population gap, still generic.
- **Verify:** recapture ts-ever-demand; `Order`/`Product`/`User` drop. **Queued
  behind the java-recaf G0 recapture.**

### G5b — Svelte resolution path ($lib + hooks wiring)
- **Seam:** part 1 *per-language* (`languages/svelte/hooks.rs`); part 2
  *per-ecosystem* (`ecosystem/manifest/`). Feeds existing
  `SymbolLookup::resolve_path_alias` + `resolve_via_aliased_import`.
- **Part 1 — hooks wiring ✅ (2026-05-25):** `SvelteResolver` (mod.rs) already
  delegated `build_file_context`/`resolve` to the TS resolver, but `SvelteHooks`
  never called it — so `.svelte` files got NO import table (legacy fallback)
  and the proven G5 strategy never saw their imports. **Wired `SvelteHooks` to
  delegate `build_file_context` + `resolve_ref` (mirroring `VueHooks`).** This
  is why svelte sat at 1.38%.
- **RESULT ✅ (2026-05-25):** svelte-shadcn recapture **1.38% → 95.6%** (224 →
  38,697 edges). Svelte-language unresolved corpus-wide **16,040 → 1,677**.
  `Field` 1,547→0, `Card` 1,138→0, `Button` 828→1. The hooks wiring alone did
  it — `$lib` resolves through G5 + the existing tsconfig-paths capture; no
  separate $lib population needed.
- **Residual follow-up:** namespace-import member access (`import * as
  DropdownMenu from '$lib/…'` then `<DropdownMenu.Root>`) — `DropdownMenu` 484,
  `InputGroup` 220, `Picker` 183 still unresolved. Generic shape (TS/JS too):
  resolve `Alias.Member` where `Alias` is a wildcard/namespace import. Separate
  small task.
- **Closes:** svelte `$lib/...`(~14k of svelte's 16k); any framework whose
  alias config isn't parsed yet.
- **Where:** `ecosystem/manifest/js_config_aliases.rs` — add SvelteKit
  (`svelte.config.js` / `$lib` default) alias parsing into `path_aliases`.
  Strategy (`resolve_via_aliased_import`) already consumes it.
- **Diagnose:** svelte-shadcn; confirm `$lib` imports unresolved and no
  `path_aliases` entry for `$lib`.
- **Verify:** svelte-shadcn `Field`/`Card`/`Button` count drops sharply.

### G7 — Operator-token suppression
- **Seam:** *generic engine* — a punctuation-only-token check at ref emission
  (operators are primitives, not symbol references). No per-language data
  needed: the shape is structural (target is all operator chars, optionally
  paren-wrapped). No new trait.
- **Diagnosed ✅ (2026-05-25):** operators ARE emitted as `calls` refs — gleam
  bare (`+`,`==`,`<>`,`/`,`<`,`>`), fsharp paren-wrapped (`(=)`,`(+)`,`(>)`,
  `(*)`). **Separate bug (not G7):** fsharp also emits mis-parsed `(Fn(K))` /
  `(maps:get(…))` artifacts — extractor capturing parenthesized expressions as
  call targets. File under fsharp extractor cleanup.
- **Implemented ✅ (2026-05-25, pending verify):** added unconditional
  `filter_operator_refs` in `indexer/local_refs.rs` (drops `Calls` refs whose
  target — optionally paren-wrapped — is all `+-*/<>=!&|^%~.:?@`, with no
  module/chain), wired at both ref-filter sites (`parse_file.rs`,
  `embedded_regions.rs`). Runs for every file (unlike `filter_local_refs`,
  gated on `locals.scm`). +3 sibling tests.
- **RESULT ✅ (2026-05-25):** gleam-compiler — operator-calls **108 → 0**,
  gleam-language unresolved **101 → 21** (−79%). Generic: also clears
  haskell/fsharp/scala operator noise corpus-wide. Done.
- **Closes:** gleam `<>`/`+`/`==`/`-`/`*`; haskell `<#>`/`.=`/`-<`/`###`;
  fsharp `(=)`. ~1.5k, trivial.
- **Where:** `LanguageProfile` operator-token set; suppress emitting pure
  operators as `Calls` refs at extraction.
- **Verify:** gleam/haskell/fsharp operator unresolved → ~0.

### G10 — Generated-code adapter verification
- **Seam:** *per-ecosystem* — existing `Ecosystem` impls (`ecosystem/prisma_client.rs`,
  `protoc_generated.rs`, `openapi_generated.rs`) + a protobuf well-known-types
  bundle. No trait change.
- **Closes:** TS `PrismaClient`(1,524)/`findUnique`(938)/`findMany`(872);
  proto `google.protobuf.Timestamp`/`Duration` well-known types.
- **Where:** adapters (`prisma_client.rs` etc.) exist but need generated output
  on disk; add protobuf well-known-types bundle.
- **Do:** run `prisma generate` in prisma-calcom; confirm adapter walks
  `node_modules/.prisma/client`.
- **Verify:** prisma-calcom `PrismaClient`/`findUnique` count drops.

### G12 — Custom-resolver → DefaultResolver fallback wiring  *(repeatable)*
- **Seam:** *per-language* `hooks.rs` `resolve_ref` — append a
  `DefaultResolver::resolve_all()` fallback (like `VueHooks`) so languages with
  hand-rolled resolvers still get the generic tower (self-keyword,
  enclosing-member, aliased-import, generic-param, ranked-candidates).
- **Why generic:** an audit found these hooks don't reference `resolve_all`:
  angular, dart (96% ✓ via own path), groovy (82%), handlebars, heex, markdown,
  mdx, pug, robot, starlark, swift (93%), yaml. Those with a *custom* resolver
  but no fallback miss every generic strategy. (svelte was the extreme case:
  resolver defined-but-unwired → 1.38% → 95.6%.)
- **swift attempted → REVERTED (net-zero):** appended a `DefaultResolver`
  fallback after swift's `swift_by_name`; recapture showed swift unresolved
  **4,151 → 4,151 (zero change)**, `init` unchanged. The engine routes
  chain-less refs through `bare::resolve_bare` (always-on) and only falls back
  to the hook's `resolve_ref`; swift's greedy `swift_by_name` already resolves
  everything the generic strategies would, and the residual (`init`, etc.)
  needs real type inference (G2), not a fallback. Reverted.
- **groovy:** removed dead code (unused `effective_target`) and simplified
  `resolve_ref` to a clean `JavaResolver.resolve` delegate. Did NOT add the
  fallback (would be net-zero like swift).
- **CORRECTED understanding:** the svelte win (1.38→95.6%) was its
  **`build_file_context`** (which gave `.svelte` files a populated import
  table), NOT the `resolve_ref` delegation. So G12 = svelte only; the
  resolve_ref-fallback idea is moot for languages with working custom resolvers.
  **Real lever for below-target custom-resolver languages is G2 (type inference),
  not wiring.**

### G11 — Ambient-path de-sprawl  ✅ DONE (2026-05-25)
- **Seam:** *per-ecosystem* — moved the hardcoded framework ambient-path lists
  out of the generic `classify.rs` into `ecosystem/ambient.rs`.
- **Done:** added `ecosystem/ambient.rs` (`AmbientPathMarker { contains,
  ends_with }` + `FRAMEWORK_AMBIENT_MARKERS` + `is_framework_ambient_path`),
  registered the module, and replaced the two hardcoded blocks in
  `SymbolIndex::is_ambient_path` with one call. **Behavior-preserving** (markers
  encode the exact same matches), +4 sibling tests. ~0 metric — pure
  de-sprawl, matching the `path_alias` cleanup ethos. NOTE: chose a static
  ecosystem-layer marker list over a per-project `Ecosystem` trait method to
  avoid regression-prone build plumbing in a metric-sensitive function; the
  markers self-gate by file existence so a static union is correct.
- **Closes:** nothing — architectural hygiene only.
- **Where:** `classify.rs::is_ambient_path` — move hardcoded Nuxt/Vue/Next/
  Svelte/`globals.d.ts` path lists (items 4–5) into ecosystem-declared
  `ambient_path_markers()`, unioned at index build like `pruned_dir_names()`.
- **Do last / skip if time-constrained.** Matches the `path_alias` cleanup ethos.

---

## LANGUAGE-SPECIFIC TASKS (only after generics)

Each is genuinely language-unique — no generic mechanism covers it.

**Seam (default for this section):** *per-language* — the extractor
(`languages/<lang>/extract.rs`) and/or the `LanguageEngineHooks` impl
(`languages/<lang>/hooks.rs`). No trait additions; these implement existing
traits for one language. **Exceptions called out inline below:** L5 (R) and L7
(Lua KOReader) are *per-ecosystem* (`Ecosystem` impl — discuss before adding,
per the no-walkers rule); L2 (VB.NET operators) shares the *per-language data*
mechanism from G7.

### L1 — Bicep (44% → target)
ARM-template binding model + intrinsics (`description` 6,303, `resourceGroup`,
`resourceId`, `uniqueString`). Dedicated extractor pass for ARM symbol/binding
shape. Largest single language gap.

### L2 — VB.NET (46%) ✅ implemented (2026-05-25, recapture pending)
- **Seam:** *per-language* extractor (`languages/vbnet/extract.rs` + `keywords.rs`).
- `NameOf`/`CType`/`GetType`/`TryCast`/`DirectCast`/`AddressOf` parse as
  `invocation` nodes but are operators, not calls — added `OPERATOR_KEYWORDS`
  and guarded the invocation emission so they don't produce `Calls` refs.
  +1 test (26/26 vbnet pass).
- **RESULT ✅ (2026-05-25):** vbnet-compactgui — operator keywords **→ 0**
  (NameOf/CType/GetType/TryCast/DirectCast all gone); vbnet unresolved
  **999 → 826** (−173). Done.
- **Original note:**
`NameOf`/`CType`/`GetType`/`TryCast` operator keywords mis-emitted as `Calls`.
Extractor fix (rhymes with G7 but VB-syntax-specific).

### L3 — Odin (79%)
`msgSend`(2,368)/`GetDeviceProcAddr` — Obj-C/Vulkan proc-address FFI binding
pattern. Engine path under-developed.

### L4 — Prolog (74%)
`tnot`(3,007)/tabling predicates. Predicate-dispatch engine path.

### L5 — R (75%)
tidyverse externals (`ggplot`/`aes`/`mutate`/`slice_head`/`read_csv`). Needs
CRAN-package discovery (no R externals walker today — discuss before building
per the no-walkers rule).

### L6 — Groovy (82%)
Gradle/Liquibase DSL (`column` 2,872, `changeSet`, `addForeignKeyConstraint`) +
Spock `Mock`. DSL-block symbol resolution.

### L7 — Lua (87%)
KOReader externals (`saveSetting`/`emit_signal`) + busted globals (overlaps G1).

### L8 — MDX (43%) / Astro (0%)
JSX-in-Markdown component resolution (`LinkCard`/`TabItem`/`Card`).
- **Astro ✅ implemented (2026-05-25, recapture pending):** astro was the
  svelte case — `.astro` frontmatter is sub-extracted as TS (imports present)
  but the plugin had **no `language_hooks()`**, so no `build_file_context` → no
  import table → every `<Component>` ref unresolved (0%). Added `astro/hooks.rs`
  (`AstroHooks` delegating `build_file_context` + `resolve_ref` + `classify_external`
  to the TS resolver) and registered `language_hooks()`. Pre astro-starlight:
  100 resolved / 399 unresolved (top: `Card` 91, `StarlightIcon`, `Icon`).
- **Astro RESULT ✅ (2026-05-25):** astro-starlight — astro-language resolved
  **100 → 566**, unresolved **399 → 77** (−81%). **Astro ~0% → ~88%.** Done.
- **MDX is NOT the same:** mdx is fully wired (TS-backed `build_file_context` +
  `resolve_ref`); its 43% is genuine JSX-in-MD component-extraction work
  (`LinkCard`/`TabItem` in `.mdx`), a real per-language task — not a quick fix.

### L9 — GSP / JSP (16%)
Grails tag/resource resolution (`resource`/`hasErrors`/`createLink`).

### L10 — Zig (94%) / Fortran (94%) / OCaml (88%) / Clojure (87%) / Nim (93%)
Per-language extractor depth: Zig comptime/`usingnamespace`, Fortran
module-procedure-chain, OCaml functor-scoped opens, Clojure macro/ns,
Nim stdlib (`Rune`). Each a focused session.

### L-env — Install-gated
MATLAB (61% — no MathWorks license), Pascal OpenSSL/ICU/Castle, AssertJ/kotest
sources jars. Document as env-blocked; revisit when installs available.

---

## Per-language tracker

Updated after each task's single-project verify. Source of truth is
`baseline-all.json`; this is the working scoreboard. `*` = below 95%.

| Language | Baseline | Current | Target | Blocking tasks |
|---|---:|---:|---:|---|
| csharp | 100.00% | 100.00% | 100% | — |
| ruby | 99.83% | — | 100% | minor (ENV/ARGV builtins) |
| php | 98.40% | — | 100% | G1 (PHPUnit asserts) |
| c | 97.53% | — | 100% | G1c (libcurl headers), G2 (libc) |
| typescript | 96.38% | — | 100% | G9 (decorator/DI), G10 (Prisma), G2 (chains) |
| fsharp | 96.38% | — | 100% | G7 (operators) |
| kotlin | 96.36% | — | 100% | G1 (kotest sources jars) |
| dart | 96.29% | — | 100% | G2 (chains), G5b (mockito `_i1`) |
| pascal | 96.00% | — | 100% | L-env, generic-param (G8) |
| java | 95.93% | — | 100% | G1 (AssertJ/Mockito) |
| rust | 95.86% | — | 100% | G2 (stdlib chains) |
| erlang | 95.91% | — | 100% | record/atom refs |
| haskell | 95.96% | — | 100% | G7 (operators) |
| javascript | 92.55% | — | 100% | G1 (test globals/jQuery), G2 |
| go* | 92.87% | — | 100% | G6 (scope locals), G2 |
| zig* | 94.75% | — | 100% | G8 (generic params), L10 |
| nix* | 92.59% | — | 100% | builtins/lib chain |
| python* | 95.05% | — | 100% | G6 (`_`), G2 |
| scala* | 95.01% | — | 100% | G1, G2 |
| elixir* | 94.69% | — | 100% | macro/import resolution |
| cpp* | 94.33% | — | 100% | G8 (templates), G2 |
| nim* | 93.75% | — | 100% | L10, G2 |
| swift* | 93.75% | — | 100% | G6, G2 |
| fortran* | 94.19% | — | 100% | G6, L10 |
| ada* | 93.75% | — | 100% | field-chain depth |
| ocaml* | 88.43% | — | 100% | G2 (stdlib), L10 |
| clojure* | 87.64% | — | 100% | L10 |
| lua* | 86.97% | — | 100% | G1, L7 |
| robot* | 84.96% | — | 100% | keyword resolution |
| groovy* | 82.08% | — | 100% | L6 |
| markdown* | 81.78% | — | 100% | link/code-fence refs |
| vue* | 83.89% | — | 100% | G5b, ambient components |
| jupyter* | 79.58% | — | 100% | host-language cell refs |
| odin* | 79.52% | — | 100% | L3 |
| r* | 75.81% | — | 100% | L5 (tidyverse) |
| prolog* | 74.10% | — | 100% | L4 |
| matlab* | 61.90% | — | 100% | L-env |
| vbnet* | 46.22% | — | 100% | L2 |
| bicep* | 44.17% | — | 100% | L1 |
| mdx* | 43.08% | — | 100% | L8 |
| gsp* | 16.40% | — | 100% | L9 |
| svelte* | 1.38% | — | 100% | G5b |

(Languages already ≥95% not blocking and not listed individually roll up under
the generic tasks; full per-language list in `baseline-gaps.md`.)

---

## Execution order (commit to this)

1. **G0** measure current work (grounds everything).
2. **G1** reachability — unblocks the most refs across the most languages.
3. **G2** chain/return-type — keystone for stdlib + ORM + builder chains.
4. **G8** generic-param capture — mechanical, cpp + others.
5. **G9** decorator/DI — concrete TS mass.
6. **G6** scope-local binding.
7. **G5b** svelte $lib, **G7** operators, **G10** generated-code (small, parallelizable).
8. **G11** ambient de-sprawl (cleanup).
9. **L1…Ln** language-specific, highest-gap first (Bicep, VB.NET, GSP, MDX, …).
10. **Full-corpus recapture** — once, at the end.
