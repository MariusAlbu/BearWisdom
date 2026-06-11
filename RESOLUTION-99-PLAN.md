# RESOLUTION-99-PLAN — every language to 99%+ through the generic engine

Built 2026-06-11 from an 8-agent sweep: resolver ladder, chain walker/members arena, extractor
verification, externals inventory, profile-data audit, embedded-template architecture, catalogue
diagnostics, and the language×bucket data matrix. All claims below were verified against **current
code** (file:line), not the stale catalogue. MATLAB is exempt (no install artifact; documented).

Companion: `RESOLUTION-FIX-PLAN.md` (live status doc). This file is the *forward* plan; corrections
to that doc's claims are in §1.

---

## 0. Ground truth

**Corpus (2026-06-09 reindex CSV, by-project denominators): 86.24%** — 1,498,534 unresolved /
10,888,324 attempts. Refs-to-99 across all sub-99 languages ≈ 1.39M; top-15 languages hold 62%.

Top-15 gap decomposition (refs that must bind or be re-scoped to reach 99):

| bucket | share | meaning |
|---|---|---|
| **D corpus-scope** | **~47%** (~403k) | vendored/generated/duplicated/mislabeled — architect scope decisions, no resolver change moves these |
| **B generic engine** | ~27% (~232k) | missing generic rungs/capabilities — the real engine work |
| **E externals** | ~16% (~135k) | walker exists, artifact absent (installs) + 3 missing walkers |
| **A extractor** | ~5% (~45k) | non-refs still emitted (mostly C/C++ macro qualifiers) |
| **C profile data** | ~5% (~47k) | unset profile fields, dead keyword data |

Two denominator traps that distort everything: (1) `make-*`, `nginx-*`, `cobol-gnucobol`,
`perl-perl5` are **C codebases** — their gap is the C engine's, not make/nginx/cobol/perl's;
(2) `ts-immich` is ~48k *Dart* refs filed under "ts". This is NOT a detection bug — per-file
language and embedded-region `origin_language` are attributed correctly in the DB
(`unresolved_by_lang_kind`, stats.rs:325, keys by `COALESCE(s.origin_language, f.language)`).
The gap is a **reporting asymmetry**: `ResolutionBreakdown` has no resolved-side per-language map
(only `resolved_by_strategy`), so no per-language *rate* exists; the corpus CSV carries only the
all-languages headline per project, and per-language corpus tables fall back to project-name
prefixes. Fix in §7 (edges_by_lang).

---

## 1. Status corrections to RESOLUTION-FIX-PLAN.md (verified against current code)

| claim in live plan | actual state | evidence |
|---|---|---|
| B5 Dart `package:self/`→`lib/` rewrite "✅ already coded" | **⬜ ABSENT** — `classify_dart_import_uri` (dart/hooks.rs:165) classifies ALL `package:` URIs external, incl. own package; no rewrite exists anywhere. `.dart` trim ✅ exists (extractor-side, symbols.rs:614,655) | resolver-ladder agent |
| Markdown fence `from_snippet` mis-tag "⬜ Bucket A" | **✅ FIXED** — `embedded_regions.rs:82` sets `region_is_snippet` from `EmbeddedOrigin::MarkdownFence` | extractor agent |
| Fortran dummy-args (catalogue said unfixed) | **✅ FIXED** — `collect_procedure_dummy_args` fortran/extract.rs:130,163 + test | extractor agent |
| Prolog `.pl`→Perl misdetection "engine bug" | **✅ FIXED in walker** (`is_likely_prolog`, walker.rs:86) — DBs predate it; realization = reindex | extractor agent |
| Haskell keywords stub "⬜ remaining ~12k" | **✅ redirect done** — haskell/mod.rs:86 returns `keywords::KEYWORDS`; ⚠️ but `profile.builtin_skip: None` — verify the decline actually fires (§3) | profile agent |
| C/C++ macro-qualifier TypeRef "absorbed by keyword skip" | **⬜ REMAINING ~30–40k** — type_refs.rs:83 only guards `__builtin_*` intrinsics; no MacroCatalog suppression. Separate fix from keywords | extractor agent |
| B1 TS alias keying / component-tag, B2 NamespaceScope, B3 prelude rung | **✅ all confirmed in code** with tests (B1 build-layer fixture missing — add) | resolver-ladder agent |
| Go composite-literal, zig nested containers, C# string-literal, clojure `:refer`+catch, freemarker/velocity/jsp/thymeleaf Object, lua `s:gsub` chain | **✅ all FIXED** with tests | extractor agent |

**Meta-lesson re-confirmed:** the catalogue overstates; always verify current code before scheduling work.

---

## 2. The five generic-engine levers (B-class) — all [generic] or [profile data], zero hooks

### E1. Reachability-bounded external member admission (B4) — the biggest single lever
```rust
// type_checker/core/members.rs:104-119 — the ONLY external admission today:
let is_external = pf.path.starts_with("ext:");
let ext_trait_scopes = is_external.then(|| trait_interface_qnames(pf));
// → external Class/Struct methods skipped wholesale; receiver typed `Kysely` has no
//   `selectFrom` in MembersIndex → chain stops at hop 1.
db.selectFrom("user")        // ❌ MembersIndex.lookup MISS (members.rs:104)
self.assertEqual(...)        // ⚠️ recovered ONLY via fragile qname-climb (chain.rs:1187) —
                             //    no overload arity pick, no generic-arg binding
```
Everything else is already in place — **no return_type problem**: `build.rs:258` metadata pass has
no `ext:` skip; `SymbolTypeMap` ingests externals; `SupertypeGraph` includes ext edges (EXT-3,
supertype.rs:452).

**Fix:** widen the gate to `trait_scopes ∪ reachable_ext_types` where
`reachable_ext_types = {parent of any internal→external supertype edge} ∪ {ChainMiss current_types}`.
Write-set bound: |reached ext types| × avg methods ≈ 1k–6k vs ~1M wholesale.

**Ordering decision (architect):** MembersIndex builds *before* SupertypeGraph (engine.rs:126 vs :144).
- *(a)* split `build_explicit` out of `build_multi`, run it first (explicit pass doesn't need MembersIndex)
- *(b)* keep order; second `ingest_files` admission pass after the graph exists (append-only, incremental-safe)

Recommendation: **(b)** — zero re-ordering risk, reuses the existing augment contract (engine.rs:202).
Depth-1 first; measure on ts-rallly + an RxJS-heavy Angular project; then decide the fixpoint.
**Unlocks:** TS ORM/builder chains, python `self.assertEqual`, php `$this->assertSee`, java/kotlin
test bases, lua typed receivers — the cross-language ~95k scripting class + TS E-chunk.
**Failing test first:** internal class extends external base with method `m` returning `T`; chain
`obj.m().n()` must bind both hops. ~1 session (+1 fixpoint follow-up).

### E2. Bare-call implicit-self synthesis (chainless calls)
```rust
assert_that(...)   // gdscript, in class extends GdUnitTestSuite
// ❌ no receiver → never reaches ChainWalker; flat-global rung can't see inherited members
```
**Fix [generic]:** when the chainless bare path declines AND the enclosing symbol's `scope_path`
names a type with supertype edges → retry as synthetic `[SelfRef, name]` chain through the existing
walker. Additive fallthrough at `engine.rs:292/:307`; no per-language code; pairs with E1 (the
inherited member is often external). **Unlocks:** gdscript/ruby/python/lua inherited bare calls.
**Failing test:** gdscript class extending a base with `assert_that`; bare call binds. ~1 session.

### E3. Embedded-origin miss → host-hook fallback (Phoenix B5, option a — DECIDED by evidence)
```text
<%= downloads_link(ep) %>   // .heex — ref_origin_language="elixir"
// ❌ dispatches to Elixir resolver (engine.rs:246 keys hooks on file_ctx.language);
//    HeexHooks.resolve_ref (colocated *View binding) is UNREACHABLE
```
Blast-radius audit: every other host hook either has **no** `resolve_ref` (ejs/gsp/nunjucks/pug/
jinja/mdx/markdown/handlebars/liquid/twig) or is scope-directed and declines (php/java). The
fallback is a strict widening: one `.or_else(|| host_hook.resolve_ref(host_file_ctx, …))` at the
embedded-miss point in `loop_body.rs` (~:583–721); `host_file_ctx` is already built (:443).
This is also the **generic seam for template context-binding** (ejs `locals`, jsp EL model) —
prove on heex, promote to a `context_provider` profile field once a second host uses it.
**Failing test:** heex + colocated view fn → edge with strategy `heex_colocated_view_fn`. ~0.5–1 session.

### E4. Dart self-package import rewrite [profile data on existing rung]
`package:<own-pkg>/x.dart` → bind under `lib/` via the existing `resolve_via_module_anchor`
`ByNameUnderModuleDir` path; own package name is already in ProjectContext (pubspec). No dart hook.
**Failing test:** `import 'package:myapp/models/user.dart'` + `lib/models/user.dart` → edge, not
external. ~0.5 session. (~12–16k dart B refs.)

### E5. Alias-expansion residue (TS-family) [generic]
`expand_alias_typed` gaps that block real chains: non-transparent `Mapped` (`Record<K,V>` value
projection), `infer` / template-literal `Other` arms (alias.rs:133–163, 368, 408). Union/
Intersection/Keyof already work on the live typed path. ~1 session.

Smaller B items found by the sweep: per-package alias **build-layer regression test** (B1 fix has
none); union-fallback prefix for packages with empty alias snapshots (lookup_impl.rs); shell
`source`-graph imports (~1.4k); ada use-member walk; OCaml `open` (see §3).

---

## 3. Profile-data sweep (C-class) — pure data, one agent per family, parallel

Verified unset/dead, with source of truth (no third-party lists anywhere):

| # | lang | change | refs | evidence |
|---|---|---|---|---|
| 1 | **C/C++** | `c_lang/mod.rs:148` keywords() returns 12-entry inline stub; redirect to `keywords::KEYWORDS` (400+ entries, currently DEAD). Purge the nlohmann-specific template-param names while touching it (§5) | ~35–45k | profile agent |
| 2 | **Erlang** | `name_normalization: Spec{strip_chars:['\'']}` — quoted atoms `'P_basic'` never match stripped symbol names | ~7.1k (regression) | erlang/profile.rs:62 |
| 3 | **OCaml** | `open M` scope-injection: arm `file_scoped_imports`/wildcard for open'd modules | ~11k | ocaml/profile.rs:94 |
| 4 | **Elixir** | investigate two regressions: `use M` transitive injection (~9k) + short-alias type_refs missing despite `alias_module_qname:true` (~9.4k) — root-cause before choosing mechanism | ~18k | diag + profile agents |
| 5 | **R** | `builtin_skip` wrapping `keywords::KEYWORDS` (language spec set) | ~3–5k | r_lang/profile.rs:72 |
| 6 | **Ruby / Starlark / Nim** | `builtin_skip` closed spec sets (Starlark ~15 names from Bazel spec) | ~2k | profile.rs each |
| 7 | **Nim** | KIND_TABLE Calls row + `EnumMember` | ~1.5k | nim/profile.rs:13 |
| 8 | **VBA** | `namespaceless_global_type_lookup: Global` (no import mechanism in the language) | ~3k | vba 83–96% |
| 9 | **VB.NET** | `name_normalization` case-insensitive (spec) | small | vbnet/profile.rs |
| 10 | **SQL** | `name_normalization` case-insensitive (ANSI) | small (already 99.4) | sql/profile.rs |
| 11 | **Odin** | `module_skip` for `system:`/`vendor:` foreign imports | ~0.5k | odin/profile.rs:51 |
| 12 | **Fortran** | declare Inherits row in KIND_TABLE (works by accident today) | hygiene | fortran/profile.rs:9 |
| 13 | **Dockerfile** | `FROM <image>` registry refs → module_skip/decline shape | ~0.4k | dockerfile/profile.rs:44 |
| 14 | **Haskell** | verify keywords redirect actually fires in decline path (`builtin_skip: None` in profile — confirm `keywords()` is consulted; wire if not) | up to ~12k | haskell/mod.rs:86 vs profile.rs:72 |

~1–2 sessions total, parallelizable, near-zero regression risk, each with a profile_tests case.

---

## 4. Extractor residue (A-class)

| item | mechanism | refs | status |
|---|---|---|---|
| **C/C++ macro-qualifier TypeRef** (`pTHX_`/`LUA_API`/`WINAPI`/`__aio`/`GLAPI`) | suppress in `sweep_typerefs` when name ∈ file's MacroCatalog (type_refs.rs:83 — guard currently only covers compiler intrinsics) | **~30–40k** | the big A item; also drains the perl-perl5/make-curl "fake language" gaps |
| C/C++ `#define`-alias callables extracted `kind=variable` (`ngx_free`) | extractor kind fix or kind-table row | ~6–10k | not previously planned |
| Erlang `Fun(X)` variable-as-call over-emission | extract.rs arm | ~3k | new |
| Nim nested decls >2 deep (indent gate extract.rs:132), triple-quote string Calls (:193) | extractor | ~1.5k | new |
| Clojure dotted-ns heads as Calls (`sci.core`) | extract.rs | ~1.9k | new |
| HTML uppercase legacy tags as components (`<A>`,`<BR>`) | html/extract.rs:199 gate | ~2.9k | new |
| Swift `swift_type_name` accepts `simple_identifier` | calls.rs:318 | ~0.75k | confirmed remaining |
| Angular `$any`/template `#refs` over-emit | template extractor | ~0.7k | remaining |
| Robot `GROUP` missing from control-flow skip list (RF 7) | robot/extract.rs:366–384 | small | new |
| Kotlin `scan_type_refs_inner` recurses into string interpolation (no literal guard) | kotlin/extract.rs:845 | unknown, likely small | new (NF-1) |
| YAML trailing-colon keys as Calls | locate actual source | ~60 | low |

~2–3 sessions, each fix = failing fixture test → guard → single-project reindex spot-check.

---

## 5. Rule-violation purge (engine-purity debt — required, MATLAB-precedent applies)

Hand-maintained third-party API lists found (forbidden; fix = manifest-driven classification at the
ecosystem layer, purge list in the SAME PR, single-project verify so the regression is honest):

| location | contents | replacement |
|---|---|---|
| `fsharp/keywords.rs:118–243` | `List.map`/`Seq.*`/`Async.*` + entire Expecto API | FSharp.Core via dotnet-stdlib walker (exists); Expecto via NuGet walker |
| `elixir/predicates.rs:80–258` ALWAYS_EXTERNAL | 80 Hex package roots (Phoenix, Ecto, …) | `mix.exs` deps via hex manifest reader (stdlib subset `Kernel`/`Enum`… is legal, keep) |
| `php/predicates.rs:44–53` ALWAYS_EXTERNAL | Illuminate/Symfony/Doctrine/… | composer manifest only |
| `kotlin/predicates.rs:28–42` ALWAYS_EXTERNAL | io.ktor/org.springframework/com.fasterxml | Maven/Gradle manifest (jvm platform roots `java`/`javax`/`androidx` are legal, keep) |
| `swift/predicates.rs:26–71` ALWAYS_EXTERNAL_MODULES | RxSwift/Alamofire/Firebase/… (~50) | `Package.swift` deps via spm walker (platform frameworks Foundation/UIKit legal, keep) |
| `c_lang/keywords.rs:400–462` | nlohmann/json-specific template-param names | delete; the generic `EndsWith("Type")` template-param predicate already covers the pattern |

~1–2 sessions. Expect honest per-language dips where manifests are absent on disk — that's E, not
a reason to keep the lists.

---

## 6. Externals (E-class)

**Installs (operational, no code):** re-attempt gsp-openboxes + jupyter-ml npm (broken manifests —
stash-to-.legacy rule applies); jQuery/jQuery-UI/DataTables for gsp-openboxes (script_tag_deps.rs
hydrates once on disk — this alone is most of gsp-openboxes' 35%); pip robotframework + Browser/
Selenium per robot project; melos bootstrap dart-appflowy/dart-frog; Gradle toolchain for JVM
projects. ~1 session, parallel with anything.

**Investigation:** php-livewire 91.3→62.8 / smarty 58.1→29.1 unreconciled (old-engine install
figures vs fresh reindex) — determine whether composer-install effect didn't persist or R2/R3
changed PHP resolution.

**Missing walkers (each discussion-gated per the no-new-walkers rule):**
1. **gradle-classes** — Gradle cache layout jar cracking; `gradle_caches_root()` +
   `resolve_gradle_sources_jar()` helpers already exist; activation `TransitiveOn("maven")` like
   maven-classes. Unblocks JVM test-DSL (~30k: JUnit/ScalaTest/Spock) → kotlin/scala/groovy 99%.
2. **ocaml-stdlib** — `LanguagePresent("ocaml")`, probe `ocamlfind printconf` (~part of 11k).
3. **nim-stdlib** — probe `nim --lib` (~3k).
4. **Lua stdlib** — DECISION NEEDED: no disk artifact (C-implemented). Options: walk Lua-source
   `luaL_Reg` tables from a lua checkout (real discovery), or document lua as borderline. The
   MATLAB precedent forbids a name list.
5. Conan (C++) — defer; vcpkg headers already covered.

---

## 7. Corpus-scope decisions (D-class — architect calls, ~47% of the top-15 gap)

No resolver change moves these. Each needs a decision + small accounting/locator change:

| decision | refs | proposed treatment |
|---|---|---|
| **Dart generated code** (`*.g.dart`, `*.freezed.dart`, `generated/`) | ~31k (immich 64%) | exclude from app-rate denominator (build_runner output is machine-written); keep indexed |
| **Jupyter locale duplication** (16 notebooks × 50 `translations/*`) | ~39k → real ~0.8k | de-dup in metric (4b extension) |
| **C-codebase relabeling** — perl-perl5, make-curl/tmux, nginx, cobol-gnucobol report under C | ~100k visibility | NOT a decision — a stats-layer task: add `internal_edges_by_lang` to `ResolutionBreakdown` (symmetric SQL to `unresolved_by_lang_kind`, stats.rs:325, same `COALESCE(s.origin_language, f.language)`), derive `rate_by_language`, wire CLI/MCP/quality-check (wire-up-everywhere), emit in the corpus CSV; per-language tables then come from the DB, not project-name prefixes. Their engine fixes are §3#1 + §4 C items |
| **Framework-source repos (D2)** — gsp-grails-core, prisma-prisma, odin-compiler, ts-nextjs, zig | large | report separately from "application rate"; their refs still benefit from engine fixes |
| **Vendored-no-manifest** — thymeleaf-myblog CodeMirror copies | ~7k | extend vendored detection or accept; no manifest exists (not D1a-able) |
| **prolog-swipl `tests/xsb`** quadratic fixtures + stale-detection DBs | ~28k | reindex (detection fix landed) + fixture-noise accounting |
| **keepassxc Qt/STL** | ~25k | E-install (Qt on disk → qt_runtime walker exists) — choose install over scope-out |
| **r-shiny == rmarkdown-shiny double count** | ~7k | drop one from corpus |

~1 decision session (architect) + ~1 implementation session.

---

## 8. Per-language path to 99 (master table)

Class: α = engine+profile alone · β = needs E · γ = needs D decision · δ = documented borderline.

| language | rate | refs-to-99 | class | path |
|---|---|---|---|---|
| ts | 88.6 | 139k | α/γ | B1 realized (recapture) + E1 chains + D (nextjs harness) |
| pascal | 75.0 | 99k | α/γ | `.inc` attribution realized + Interface kind-row ✅ + D (castle = engine source) |
| dart | 72.1 | 73k | γ | D generated-exclusion + E4 rewrite + pub/Flutter walkers (exist; install) |
| lua | 45.4 | 72k | β | E1+E2 receiver typing + stdlib decision (§6.4) + luarocks installs |
| kotlin | 79.7 | 72k | β | B3 realized + E1 + gradle-classes walker (§6.1) |
| cpp | 51.5 | 62k | γ | §3#1 keywords + §4 macro-qualifier + D vendored scope + Qt install |
| gsp | 72.5 | 59k | γ | jQuery install (openboxes) + D2 (grails-core) |
| haskell | 64.7 | 40k | α | §3#14 verify + explicit-import B/C + cabal walker ✅ already landed |
| jupyter | 47.0 | 40k | γ | locale de-dup; residue is CRAN installs |
| go | 78.8 | 39k | α/γ | A fix realized (recapture) + kind-table ✅ + D (pocketbase test harness) |
| odin | 77.3 | 39k | γ | toolchain-payload scope (landed; recapture) + module_skip |
| perl/make/nginx/cobol | 74–76 | ~110k | γ | = C-language fixes (§3#1, §4) + relabel reporting |
| prolog | 78.0 | 32k | γ | reindex (detection landed) + xsb accounting |
| javascript | 86.8 | 31k | γ | D (CodeMirror vendored) + E1 ORM chains + ghost externals |
| elixir | 82.6 | 30k | α | §3#4 use/alias investigation + E3 phoenix |
| erlang | 91.3 | 24k | α | §3#2 quote-strip + §4 Fun(X) + OTP walker ✅ exists |
| r | 75.3 | 22k | α/β | r_stdlib ✅ landed + E1/E2 + CRAN installs |
| ocaml | 76.8 | 22k | α/β | §3#3 open-tracking + ocaml-stdlib walker |
| scala/groovy | 86.5/81.8 | 39k | β | B3 free-ride + gradle-classes |
| clojure | 51.1 | 17k | α | `:refer` fix realized (recapture) + §4 dotted-ns |
| python | 80.3 | 17k | α/β | E1 (self.assertEqual) + pip installs |
| nix | 44.3 | 24k | α | let-alias realized (recapture) + `with lib.types` combinators |
| svelte/vue/astro | 61–92 | ~32k | α | B1 component-tag realized (recapture) + E1 |
| smarty/pug/twig/nunjucks/liquid/ejs | 27–88 | ~7k | β/α | embedded-JS externals install; ejs = E3 context binding |
| gdscript | 86.3 | 1.3k | α | B2 realized + E2 |
| swift | 96.5 | 0.9k | α | §4 simple_identifier |
| fsharp | 92.0 | 8k | α | §5 purge + dotnet-stdlib (exists) |
| dotnet | 97.2 | 30k | α | free-rides B1 (squidex SPA) + B3 (String/Object) |
| vba | 96.0 | 4k | δ | §3#8 flip; COM late-bound dispatch = documented ceiling |
| **matlab** | 63.5 | — | **EXEMPT** | no toolbox artifact; honest post-purge numbers stand |

**Free-rider cascades:** B1-recapture → dotnet/vue/svelte/astro/react · B3 → scala/groovy/csharp ·
E1 → python/php/r/lua/ts · §3#1+§4-C → perl/make/nginx/cobol/cpp/c.

---

## 9. Sequencing (step → verify pairs; sessions = architect+AI wall-clock)

```
P0  D-decisions (§7)                              [architect, 1 session of calls]
    └ verify: decisions recorded; accounting items implemented (~1 session)
P1  E1 depth-1 B4 admission (ordering (b))        [1 session]   ← failing test first
    E2 implicit-self synthesis                    [1 session]   ← independent, parallel agent
    E3 phoenix host-hook fallback                 [0.5 session] ← independent, parallel agent
    └ verify: lib tests green; single-project reindexes: ts-rallly, elixir-changelog, gdscript-beehave
P2  C-sweep §3 (one agent per family, parallel)   [1–2 sessions]
    A-residue §4 (parallel by language)           [2–3 sessions]
    └ verify: profile_tests + fixture tests; spot reindex c-redis (macro-qualifier), erlang-rabbitmq
P3  §5 purge + manifest classification            [1–2 sessions]
    E4 dart rewrite + E5 alias arms               [1 session]
    └ verify: single-project: php-monica, kotlin-ktor, swift-*, dart-serverpod
P4  E installs (§6 operational) + discussion-gated walkers (gradle-classes first)  [2–3 sessions]
    └ verify: kotlin-okhttp/scala-gatling test-DSL edges appear
P5  THE corpus recapture (ONE, at the end — per the standing rule)                [1 session]
    └ verify: per-language table §8 against 99; document δ/γ residuals per the 100%-or-documented rule
```
Total ≈ **12–16 sessions**. Architect bottleneck: P0 decisions, E1 ordering call, walker
discussions (§6), purge-regression acceptance (§5). Everything else is execution.

## 10. Guardrails (unchanged, restated as gates)

- Every fix lands **through the generic engine**: [generic] rung/capability or [profile data];
  hooks only where data can't express it (none required by this plan).
- Failing test first, sibling `_tests.rs`, no inline test edits without the split.
- No new walkers without discussion; no hand-maintained third-party lists (§5 removes the existing ones).
- No env-var gates. No subset baselines. **One** full recapture, at closeout (P5).
- Single-project reindexes are the per-fix verification instrument.
- Comments describe contracts only — no phase tags, no PR references, in any code this plan produces.
