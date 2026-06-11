# Unresolved-ref resolution plan — live corpus synthesis

Rebuilt from the live indexes (258 DBs) on this pass — the prior `_catalogue/` was deleted.
Data layer: `_catalogue/build_catalogue.py` → `_summary.json` + 80 per-language slices. Diagnoses:
15 `_catalogue/diag_*.md`, each root cause grounded in `file:line` and **verified against the live
DB** (regression = own-project symbol exists with ~0 incoming edges vs thousands of call sites).

## Top-line: the corpus shrank, and the lever moved

**Live total: 2,365,966** unresolved — down from the 4.03M snapshot. The engine improved
materially since: **nim 70k→11k**, **fortran 59k→7k** (flat-namespace flag flipped on, confirmed),
**swift 5.1k→0.75k** (TypeRef over-emission fix landed), **proto 276→4**, ada gained `builtin_skip`,
pascal's `TVector3` kind-table fixed. Several recommendations from the old plan are now **done** —
do not re-propose them.

The big correction: the old plan led with "`builtin_skip` epidemic ≈ 600–700k." **That is wrong now.**
Most of that either already landed or was never the real mechanism. The dominant engine-addressable
work this pass is **generic-resolver rules**, not profile data.

| Category | ~Share of 2.37M | Engine bug? | Where |
|---|---|---|---|
| **D. Corpus / measurement scope** (split below) | **~55–60%** (~1.35M) | Mostly no — ~155k (D1a) is clean manifest-driven ecosystem work; ~465k (D1b) is first-party toolchain payload, corpus-scope, **not a dependency** | Option 4 |
| **E. Disk-absent externals** (bind path exists, deps not walked) | **~15%** (~0.35M) | No | Option 5 |
| **B. Generic resolver missing rules** | ~13–15% (~330k) | Yes — **now the big lever** | Option 1 |
| **A. Extractor bugs** | ~3–4% (~85k) | Yes | Option 3 |
| **C. Language-specific (profile)** | ~2–3% (~55k) | Yes — cheapest, mostly done | Option 2 |

C/C++ is still 875k (37%), ~70% of it zig-compiler's vendored cross-compile libc/libc++ counted as
internal — the single biggest number remains measurement noise.

---

## Implementation status

Three waves landed on `feat/resolution-engine`: **R1** (read-only audit of what was genuinely
undone), **R2** (`45bbfe88`), **R3** (`b31b81ab`) — plus a Bucket-E externals install and a DB
lock-contention fix (`605010ed`).

Legend: ✅ done (unit-green) · 🟡 partial · ⛔ rule-blocked · ⬜ remaining · 📏 rate-unmeasured

> **⚠️ MEASUREMENT CAVEAT — read first.** Every ✅ below is **unit-green**; corpus rate is NOT fully
> re-measured. The release binaries were **rebuilt 2026-06-11** (R2/R3 + busy_timeout + MATLAB/Robot
> changes) — the "release rebuild" half of the closeout is done, and the 9 touched projects
> (matlab×3, robot×3, php×3) were reindexed on the new engine (numbers inline below). The **full
> corpus has NOT been recaptured** since the 2026-06-09 baseline (`unresolved-reindexed-projects.csv`);
> that single closeout recapture is still gated on the remaining engine items (B4, B5). Until it
> runs, the corpus-wide rate is unknown and per-project numbers are point-in-time on the stated engine.

> **META: the catalogue was STALE.** Confirmed by the R1 audit — built from pre-existing DBs, it
> systematically OVERSTATED remaining work. The two headline "levers" (B1 ~90k, B3 ~108k) were
> ALREADY CODED, as were 6/7 B2 flips, 5/8 Bucket-A classes, B5 Dart/PHP, and the C kind-tables.
> None needed reimplementing — only a reindex to realize. Always check current code + single-project
> reindex before implementing.

### ✅ Already coded (R1 audit found done — NOT reimplemented), 📏 unrealized
- **B1** import/alias rebind (~90k) — TS path-alias monorepo keying (keyed by `file_package_id`,
  `build.rs`/`lookup_impl.rs`) + component-tag rebind (`resolve_via_component_import`, net-new in
  the checkpoint). Both unit-tested.
- **B3** implicit-prelude qualify (~108k JVM) — `implicit_prelude_namespaces()` java/kotlin/elixir/r.
  (sql-flyway java 9,963→2,343 when reindexed.)
- **B2** flips — gdscript (beehave 1,700→293), jinja (4,064→97), hcl (aws-vpc 538→2), cmake, graphql, proto.
- **Bucket A** (5/8) — go composite-literal, fortran dummy-args, zig nested-decl, clojure `:refer`, markdown fence.
- **B5** Dart `package:self/`→`lib/`, PHP `\`-FQ normalization.
- **C** kind-tables — Pascal Calls→Interface (doublecmd 13,942→12,980), Go/Odin Calls→Variable.
- **D1a** `.gitmodules` vendored submodules → `origin='external'` (lua-luals 140,123→8,675, koreader 99,764→32,265).

### ✅ R2 — `45bbfe88`
- **B2 prisma** directory-scoped mode — `namespaceless_global_type_lookup` bool → enum
  `NamespaceScope{Off,Global,DirectoryScoped}` across all profiles (13 flat-namespace → Global,
  prisma → DirectoryScoped + sibling-dir bind filter, rest → Off); failing-test-first.
- **B5 SQL** schema-prefix leaf-retry — generic dotted-miss → `by_name(leaf)`.
- **4b corpus-scope** — zig `lib/libc*`/`libcxx*` + odin `core/`+`vendor/` reclassified
  `origin='external'` via per-language ecosystem locators (`ecosystem/toolchain_payload.rs`); jupyter
  dedup. (Architect-approved mechanism: manifest-less toolchain payload marked external.)
- **C** Haskell Prelude+operators split (library types stay external), PHP constructs, Elixir Kernel forms.
- **B4** Lua chain-None receiver (`s:gsub()` → `[recv, method]`; fixed `get_method_table` field).
- **B5** Nix let-alias RHS capture + head-alias enable.
- **A** Clojure catch/with-open scope, C# string-literal descent guard, Freemarker/Velocity `Object` suppression.
- Verified: `cargo test -p bearwisdom --lib` 6779/0.

### ✅ R3 — `b31b81ab`
- **A** JSP + Thymeleaf synthetic-`Object` leak (same fix as Freemarker/Velocity).
- **B5 Pascal** `{$include}`/`{$i}` → attribute `.inc` fragments to the including unit.
- **B4** external return_type/field_type on the **DB-augment path** (signature-derived).
- **B2** profile_tests coverage — cmake/graphql/proto + gdscript/jinja/hcl.
- 🟡 **B5 Phoenix** HEEx→`*View` connector — **partial** (`<.component>` tag path landed; embedded
  `<%= view_fn() %>` binding does not fire — see ⬜ Remaining for the root cause found 2026-06-11).
- Verified: `cargo test -p bearwisdom --lib` 6805/0.

### ✅ Bucket E — disk-absent externals INSTALL (operational)
Installed deps for **11 projects** (7 php composer + 4 npm) so existing walkers hydrate them.
Measured on the OLD engine (single-project reindex):
- php-livewire 55.41%→**91.30%**, smarty-smarty 27.47%→**58.06%**, php-laravel 97.75%→**99.43%**.
  ⚠️ **Unreconciled (2026-06-11):** the fresh reindex (new engine, `vendor/` present + stubs) got
  php-livewire **62.8%** and smarty-smarty **29.1%** — far below these OLD-engine install figures,
  which never entered the corpus CSV. Either the composer install effect didn't persist or R2/R3
  changed PHP composer resolution. Treat the 91.30/58.06 figures as suspect pending investigation.
- dart-serverpod (melos-bootstrapped + reindexed) recovered; edges up sharply, but app-internal
  unresolved ~unchanged → modest app-rate gain. **The Dart `package_config.json` walker already
  existed** (`pub_pkg/discovery.rs`) — the "no walker" claim was stale; it was an install, not code.
- 2 npm failed on broken manifests (gsp-openboxes, jupyter-ml-for-beginners).

### ✅ DB hardening — `605010ed`
`busy_timeout` 5s→30s. dart-serverpod's "Failed to write packages" was **not** a logic bug — it was
SQLite `SQLITE_BUSY` (a cached IndexService's watcher reindexing during melos-bootstrap file churn,
contending with an explicit force-reindex). Mitigation only.

### ✅ "Rule-blocked" bucket — RESOLVED via existing walkers + provisioning (the label was stale)
The "library API name lists, forbidden in `is_*_builtin`" framing was wrong for 4 of 5: the
manifest/install walkers already exist and are registered. Reverified 2026-06-11.

**Manifest- vs install-walking distinction.** "Manifest walking" only literally applies to the
third-party half (Haskell library types via `.cabal`, R CRAN via `DESCRIPTION`). The stdlib half
(R base, MATLAB builtins, PHP core fns) has *no project manifest* — it's discovered by walking the
language *install* (`<R_HOME>/library`, `$MATLABROOT/toolbox`, a stubs checkout). Same "discover
real symbols, never hardcode" rule, different artifact. All are install/disk-gated.

- **R base** — `ecosystem/r_stdlib.rs` already walks `<R_HOME>/library/*/NAMESPACE` exports and
  shells `Rscript -e getNamespaceExports("base")` for the no-NAMESPACE `base` pkg (sanctioned
  `parse_metadata_only`, same lane as NuGet DLL metadata). R 4.6 present. **Already landed** in the
  2026-06-09 reindex: r-dplyr 42.5→61.2, r-ggplot2 44.2→73.3, r-shiny 74.6→86.0. Reconfirmed flat
  (r-dplyr 61.6). The "no Rscript" parking was a stale-machine artifact.
- **Haskell library types** — `ecosystem/cabal.rs` reads `.cabal build-depends` + the GHC package
  DB (`package.conf.d/*.conf`). GHC 9.12.1 + 226-pkg cabal store present. **Already landed**:
  hadolint 40.5→61.3, pandoc 37.5→62.3, postgrest 51.3→79.7. Reconfirmed flat (79.8).
- **PHP stdlib** — `ecosystem/php_stubs.rs` walks a JetBrains phpstorm-stubs checkout. Cloned to
  `~/phpstorm-stubs` (auto-discovered). Fresh reindex (new engine, stubs + R2/R3): php-livewire
  55.4→62.8 (+7.4), smarty-smarty 27.5→29.1 (+1.6), php-monica 95.7→95.8 (already saturated). Stdlib
  stubs move low-rate stdlib-heavy PHP modestly; high-rate apps are near their ceiling.
- **MATLAB** — `matlab/keywords.rs` WAS the real violation: ~200 stdlib FUNCTION names
  (`zeros`/`plot`/`sprintf`/`eig`/…) stuffed into the `keywords()` skip set. **Purged** to the
  rule-legal subset (reserved words, classdef-block kw, fn-arg kw, boolean literals, primitive
  class names, operator-method names). Resolution now routes through `matlab_runtime` (walks
  `$MATLABROOT/toolbox`). No MATLAB install here → **honest regression** (fresh reindex, new engine):
  exportfig 44.7→13.7, platemo 64.9→14.7, prmlt 51.8→16.4. The list was masking ~58k unresolved
  builtin-call refs across the three (platemo alone 5,153→55,379). The numbers are now honest, not
  fake; real resolution needs a MATLAB (or Octave-source) toolbox on disk.
- **Robot** — `library_map.rs` already resolves `Library X`→project-internal `.py` (robot-framework
  86.8% via vendored `src/robot`). **Extended** to also consult site-packages (`ext:`) `.py` with a
  non-`ext:`-preferred tiebreak (vendored copy still wins). Unit-tested; fresh reindex confirms
  **no regression** (browser 60.5, cookbook 42.3, framework 86.8 — all flat). **UNMEASURED upside** —
  no robot project ships its Python keyword libs (no venv), so realizing edges needs pip-installing
  robotframework + the keyword libs (Browser/Selenium) per project.

### ⬜ Remaining
- **B4 externals-walker emission** — externals whose return/field type has no parseable signature
  stay unresolved (ext-ref filter survival per CLAUDE.md). The deep pipeline lever; larger build.
- **B5 Phoenix** — the connector is more broken than "partial nested context." **Root cause (2026-06-11):**
  `HeexHooks.resolve_ref` (the colocated-`*View` binding) only ever receives the host extractor's
  `<.component>` tag refs. The actual view-helper calls — `<%= downloads_link(ep) %>` etc. — are
  emitted by the *embedded-region* pipeline with `ref_origin_language="elixir"`, so the resolver
  dispatches them to the Elixir resolver and the heex hook never sees them. Confirmed: `downloads_link`
  (view-own fn, 11 template calls in elixir-changelog) has **0 incoming edges** even on a fixed binary.
  - ✅ **Landed:** nested-context view-path mapping (`templates/a/b/x.html.heex → views/a/b_view.ex`;
    was single-level only). Correct + unit-tested, but a no-op until the routing below is fixed.
  - ⬜ **Needs an architectural call (high blast radius):** route embedded view-helper calls through the
    colocated-view binding. Options: (a) embedded refs fall back to host-language hooks when origin-language
    resolution misses — generic, touches ALL embedded hosts (vue/svelte/markdown/jsp/…); (b) the Elixir
    resolver consults `colocated_view_file` when the host file is a `.heex` under `templates/` — narrow but
    leaks Phoenix logic into the Elixir layer; (c) heex extractor emits the embedded calls as heex-origin
    refs — duplicates embedded-Elixir extraction. Only 2 corpus projects (elixir-changelog 75.4,
    elixir-plausible 83.6); the ~3k estimate assumed the binding fired.
- **Install-gated (walker complete, artifact absent)** — MATLAB toolbox (no free install; Octave-
  source could proxy), Robot Python keyword libs (pip-install per project), Gradle JVM (no gradle
  toolchain), Qt/STL (system lib), dart-appflowy/dart-frog (unbootstrapped melos). The walker
  exists; the dep/toolchain just isn't on disk. (R/CRAN moved to ✅ above — R 4.6 + Rscript present.)
- **MCP force-reindex exclusivity** — force-reindex of a cached+watched project should take exclusive
  DB access (evict the cached IndexService / pause the watcher), not lean on busy_timeout. Track-F follow-up.
- **D2 framework-source repos** (Struts/Next.js/etc.) — corpus-composition accounting, not a fix.
- **THE CORPUS RECAPTURE** — the measurement step above; converts every ✅ into a real rate and
  restores the dogfood MCP tools (down since the server was stopped to diagnose the dart lock).

---

## Bucket B — Generic resolver missing rules. Now the dominant engine work.

These reduce to **five generic capabilities**, each draining a named set of languages. Several are
one engine rung; several are regression-killers (own symbols that exist but don't bind).

### B1. Bind a bare-usage ref through the file's import/alias table — **~90k, the single biggest lever**
The symbol IS imported in the file, but the bare *usage* ref (`module=NULL`) never re-binds through
the import/alias table.
- **TS path-alias / workspace-package** (~65k, **regression**): `@/utils/logger`, `@calcom/types`
  resolve at <3% of sites (`Logger` 43 edges vs 1728 misses). `resolve_via_aliased_import` /
  `resolve_via_workspace_package` mis-key the per-package alias snapshot by `file_package_id` at
  monorepo scale. `[generic]`.
- **Component-template tags** (~22k, **regression**): svelte `<Card.Root>`→bare `Card` (namespace
  re-export), Vue `<NextButton>`→`Button.vue::Button` (634 edges from the script import, tag stays
  `module=NULL`). `process_element` emits the tag as a `Calls` ref that never consults the file's
  `import * as Ns` / aliased-default table. `[generic]` rung in `default_resolver.rs run_ladder`.

### B2. `namespaceless_global_type_lookup` for genuinely flat-namespace languages — **~14k, near-zero risk**
The terminal rung `resolve_via_namespaceless_global` (`default_resolver.rs:1918`) already works
(proven on prolog/matlab/r/nim/fortran); it's just gated off for several flat-namespace langs.
Flip the profile flag where the project is one flat namespace:
- **GDScript** (~3.3k, **severe regression**): `gdscript/profile.rs:71` `false` → every cross-file
  `class_name` / `extends GdUnitTestSuite` binds 0 sites. (Godot engine API IS present on disk — the
  earlier "API absent" claim was wrong.)
- **Jinja/Ansible** (~6.8k, **regression**): sibling-YAML vars (`matrix_user_uid`) exist as internal
  `field` symbols (incoming=0); `jinja/profile.rs:74` `false` blocks them.
- **CMake + Terraform** (~3.7k, **regression**): `${VAR}` / `var.*` set in a sibling/parent file;
  one flag covers both.
- **GraphQL** (~95, regression). **Prisma needs the *directory-scoped* variant** (a project-global
  flip regresses prisma-prisma's fixture models) — so add a directory/package-scoped mode, not a
  blanket flip. `[profile]` + one scoped-mode `[generic]`.

### B3. Implicit-prelude-namespace qualify rung — **~108k (JVM), substrate already on disk**
JDK `String.java` IS indexed (`ext:java:jdk`, 144 files), but bare `String`/`Object`/`Throwable`
(java 15,782 sites) never bind: `chain_qualification: SamePackageAndImports` only qualifies via
*explicit* imports, so nothing forms `java.lang.String`. Add a generic rung driven by a new
`[profile] implicit_prelude_namespaces` field (`java.lang`, Kotlin/Scala preludes). Kotlin stdlib
(`listOf`/`List`) rides the same rung (~13k). `[generic]` + `[profile]`. Not a regression (toolchain
types), but produces real edges and is the largest single addressable block.

### B4. Implicit-self synthesis + external-member projection for chainless calls — **~95k, highest-value/hardest**
`s:gsub()`, `self.assertEqual()`, `$this->assertSee()`, gdscript bare `assert_that()`: a chainless
call synthesizes no `SelfRef` segment (`chain.rs`/`engine.rs:296`), so it never roots on the
enclosing type and external members aren't projected. Drives the bulk of lua/python/php/r. **Not
`builtin_skip`-able** — bare `insert`/`find` collide with real own-project methods, so the only
correct fix is typing the receiver. Pairs with the documented external-`return_type`-via-signature
gap (external receivers expose no member yields through the `ext:` filter). `[generic]`, large.

### B5. Misc generic rungs — **~15k**
- **Nix let-alias / with-scope rewrite** (~22k counted, drains to builtin once rewritten): extractor
  drops the `let cfg = config.x; l = lib` RHS so `head_alias` can't rewrite `l.mkOption`→`lib.mkOption`
  (`is_nix_builtin` already declines the rewritten form). Capture RHS + generic head-alias-follows-binding.
- **C/C++ macro-defined callees** (~6–10k, regression): widen `macro_catalog` discovery beyond the
  2-level sibling walk to `compile_commands.json -I` roots.
- **SQL schema-prefix leaf-retry** (~141, regression), **PHP `\`-FQ normalization** (~0.6k, regression),
  **Pascal `{$include}` `.inc`→unit attribution** (~25–35k, regression — orphaned from the unit's
  `uses` name so the working wildcard-import rung can't reach them), **Dart `package:self/`→`lib/`
  rewrite** (`default_resolver.rs:3178` doesn't even trim `.dart`; ~regression), **Phoenix
  template↔view-module binding** (~3k, regression — confirmed `AdminHelpers`/`*View` internal, incoming 6/45/88).

**Net B ≈ 320–350k**, heavily regression-bearing. B1+B2+B3 alone ≈ 210k and are the cleanest.

---

## Bucket C — Language-specific (profile data). Cheapest, but smaller than the old plan claimed.

Much already landed. What remains is genuinely closed-set, mostly mechanical:
- **`builtin_skip` wiring** where the data exists but is dead: **Haskell** (`mod.rs:78 keywords()`
  returns a 25-name inline stub, NOT `keywords::KEYWORDS` which already lists `$`/`return`/`fmap` —
  redirect it; ~12k), **PHP + R + Ruby** closed core sets (~18k+0.4k), **MATLAB/Robot/Starlark**
  closed stdlib sets (~10k). All `[profile]`; **never** a hand-maintained library list (project rule).
- **Kind-table gaps:** Pascal Calls→**Interface** (~1.5k; TypeAlias/Class already added),
  Go/Odin Calls→**Variable** (closure locals, ~5k; zig already allows it), Nim enum-member,
  Fortran Inherits row.
- **`module_skip`** for odin `system:` foreign imports, dockerfile registry-image `FROM` shape.

**Net C ≈ 50–60k.** Pure data, parallelizable one-agent-per-family, near-zero regression risk.

---

## Bucket A — Extractor bugs. Stop emitting non-refs.

| Class | lang | ~count | mechanism (file:line) | regress |
|---|---|---|---|---|
| Composite-literal keys / non-callee selectors → `Calls` | go | **~20k** | `go/refs.rs:46,80–123` defaults to `EdgeKind::Calls` | yes (phantom) |
| Macro-qualifier tokens → TypeRef (`pTHX_`/`LUA_API`/`WINAPI`) | c/cpp | ~30–40k | `type_refs.rs:79` sweep; suppress when name is a key in the file's `MacroCatalog` | no |
| Reader-conditional `:require` drops `:refer` imports | clojure | ~7.4k | `extract.rs:776` — own cross-file calls get no import edge | yes |
| `{$include}` nested decls / dummy-arg & associate locals | fortran | ~5k | `collect_local_decls` misses procedure dummy args (`extract.rs:121`) | yes |
| Nested container decls not descended; flat qnames | zig | ~4.5k | `extract_struct_body` (`zig/extract.rs:300`) never emits `Register.Encoded` | yes |
| XML tag names inside verbatim-string test fixtures → Calls | csharp | ~1.8k | extractor descends into string-literal nodes | no |
| Synthetic `${}` wrapper return-type token → TypeRef (`Object`) | freemarker/velocity | ~1.5k | `freemarker/mod.rs:69`, `velocity/mod.rs:110` | no |
| `catch`/`with-open` bound vars → Calls | clojure | ~1.6k | `extract.rs:459` binding-form arms miss `catch` | yes |
| Markdown fence top-level ref mis-tagged `from_snippet=0` | markdown | ~8k | `embedded_regions.rs:167 unwrap_or(0)` | no (mis-count) |
| Residual: swift `swift_type_name` accepts `simple_identifier`; YAML keys→calls; robot `GROUP`/`IF`→calls; angular `$any`/`mat-*` over-emit | several | ~2k | per-diag | mixed |

**Net A ≈ 80–90k.** The C/C++ macro-qualifier class (~35k) overlaps Bucket C (keyword-set decline also neutralizes it).

---

## D — corpus / measurement scope (NOT one thing; "shouldn't be indexed" is the wrong frame)

D is not "items that shouldn't be indexed" — it is **refs the application-resolution-rate metric
shouldn't count**. It decomposes four ways, each with a different correct treatment:

- **D1a. Manifest-backed vendoring → ecosystem-discoverable, mark `origin='external'` (~155k, CLEAN ECOSYSTEM WORK).**
  lua-luals `3rd/lpeglabel`/`3rd/EmmyLuaCodeStyle` C++ (~103k) and lua-koreader's `base` crengine
  C++ (~55k) are **git submodules** (verified `.gitmodules`: 21 and 6 submodules) — the file declares
  path + upstream URL, which is exactly the manifest the ecosystem layer wants. Reading `.gitmodules`
  (plus Go `vendor/modules.txt`, Cargo vendored-sources, npm `bundledDependencies`) and marking those
  subtrees external fits the existing locator model cleanly — composer/rubygems already do the
  analogous thing for `vendor/`. Manifest-driven, no heuristic, no language rule.
- **D1b. First-party shipped toolchain payload → corpus-scope, NOT a dependency (~465k).**
  zig `lib/libc/*`+`lib/libcxx*` (~430k, verified verbatim musl/glibc/mingw/LLVM) and odin `core/`+
  `vendor/` (~34k) have **no manifest, no submodule, no fetch** (verified: neither repo has a
  `.gitmodules`). They are **not dependencies** — zig/odin OWN and maintain these copies (patched
  musl/glibc/LLVM; odin's own stdlib) and SHIP them as toolchain payload so `zig cc`/odin can
  cross-compile. There is no npm/nuget/registry to discover because nothing was ever externally
  resolved — so this is **not** ecosystem work. It's corpus-representativeness: a compiler/toolchain
  repo is mostly shipped payload, a poor proxy for "a C/C++ application." Treatments: scope these
  repos out of the application-rate metric, and/or accept the refs ARE the same generic C-resolution
  gaps (macro callees, SDK typedefs) at huge volume — they improve with the real C engine fixes
  (A/B/C), not by "externalizing a dependency" that doesn't exist. zig's own `src/*.cpp` and `lib/std`
  stay internal regardless.
- **D2. The repo *is* the library/framework — legitimately internal, real misses.** Struts,
  Grails-core, Next.js, Prisma-compiler, svelte-shadcn, odin `core/` (Odin's own stdlib),
  babashka/SCI, swipl runtime. Not vendored *into* anything — the whole repo is that codebase; its
  refs are genuinely internal and genuinely attempted (verified for JVM). Their misses are REAL
  engine gaps (B/C/E), just dense and stdlib-skewed. **Excluding them hides work, it doesn't remove
  noise.** Fix = corpus composition / report separately, NOT exclusion.
- **D3. Correctly-unresolved-by-design (~15k+).** bicep gallery (no compiler clone in-tree → ARM
  grammar has no target), `process.env` synthetic re-emits. The engine is right to emit no edge;
  fix = accounting (some already excluded). Nothing to index/not-index.
- **D4. Redundancy + one MIS-FILED bug.** jupyter = the same 16 notebooks copied 50× under
  `translations/*` (~37k — redundancy, 50×-weighted), markdown doc-fences (~37k — **already
  excluded** via `stats.rs:36 CODE_REF_FILTER`). **Mis-filed:** prolog `boot/tabling.pl`→Perl
  misdetection (~3.7k) is an **engine/detection bug, not corpus** — moved to the extractor/detection
  wave (Option 3).

**Net:** the flat "D ≈ 55–60%, not engine" line oversold it, but only ~155k (D1a) is actually
ecosystem-addressable (manifest-backed submodules). ~465k (D1b) is first-party toolchain payload —
corpus-scope, with no dependency to discover. The rest is unrepresentative-but-real framework source
(D2), correct-by-design (D3), and redundancy/already-handled (D4) — plus a ~4k engine bug that was
mis-filed here.

## E — disk-absent externals (~0.35M)
- JVM test-DSL (JUnit/ScalaTest/Spock, no Maven/Gradle classes-jar walker, ~30k), scripting library
  APIs (tidyverse/Django/PHPUnit, ~80k), Dart pub+Flutter (no `package_config.json` walker, ~20k),
  Qt/STL for keepassxc (~25k), MATLAB toolbox, Robot Browser lib, .NET BCL for VB-family.
  **Fix = install/walk deps**; two net-new manifest-driven locators (Dart `package_config.json`,
  CRAN `DESCRIPTION`) are discussion-gated per the no-new-walkers rule.

---

## Options (sequenced)

**Option 1 — Generic-resolver wave (Bucket B). The main event now.** Five capabilities, several
independent and parallelizable, each a failing test → one rung:
  - B2 namespaceless flag flips (gdscript/jinja/cmake/terraform/graphql + prisma dir-scoped) — ~14k,
    **near-zero risk, do first within this wave.**
  - B1 bare-usage rebind through the import/alias table (TS aliases + template tags) — ~90k.
  - B3 implicit-prelude qualify rung (JVM) — ~108k.
  - B4 implicit-self + external-member projection (scripting) — ~95k, hardest.
  - B5 misc rungs (nix alias, pascal `.inc`, dart `package:`, macro-catalog, sql/php/phoenix).
  **~320k, mostly regressions.** ~10–16 sessions; B2 alone is ~1 session.

**Option 2 — Profile-data sweep (Bucket C).** `builtin_skip` wiring (Haskell redirect, PHP/R/Ruby/
MATLAB/Robot/Starlark closed sets) + kind-table flips (Pascal Interface, Go/Odin Variable). ~55k,
pure data, near-zero risk, parallel one-agent-per-family. **~2–3 sessions.**

**Option 3 — Extractor wave (Bucket A).** Go false-Calls, zig nested-decl, clojure `:refer`+locals,
C# string-literal descent, freemarker/velocity wrapper, fortran locals, markdown fence-tag, swift/
yaml/robot residuals. ~85k (less ~35k absorbed by Option 2's C/C++ keyword-skip). **~4–6 sessions.**

**Option 4 — Reclassification + corpus scope (split; see the D decomposition).**
  - **4a. Ecosystem reads vendoring MANIFESTS → `origin='external'` (~155k, clean ecosystem work).**
    For D1a: extend the ecosystem layer to read `.gitmodules` (and Go `vendor/modules.txt`, Cargo
    vendored-sources, npm `bundledDependencies`) and mark the declared submodule/vendored subtrees
    external. Manifest-driven, fits the existing locator model, no provenance heuristic, no language
    rule. composer/rubygems already do the analogous thing for `vendor/`.
    **LANDED (`.gitmodules` slice):** `ecosystem/vendored_submodules.rs` reads `.gitmodules`;
    `indexer/full.rs` classifies any file under a declared submodule subtree `origin='external'`
    (`ext:submodule:` path), alongside the existing vendored-C path. Verified on the affected
    projects: lua-luals 140,123→8,675 internal-unresolved (−131,448; 4,229 files externalized),
    lua-koreader 99,764→32,265 (−67,499; 1,058 files); zero submodule-origin unresolved remain in
    either. Go `vendor/modules.txt` / Cargo vendored-sources are follow-on extensions of the same reader.
  - **4b. Corpus scope + accounting (not resolver, not ecosystem).** Scope out first-party toolchain
    payload (D1b — zig/odin shipped libc/stdlib, ~465k) and framework-source repos (D2) from the
    application-rate metric or report them separately; de-dupe jupyter locales (D4); confirm bicep /
    `process.env` / markdown-fence exclusions (D3). Accounting/curation, no engine code. **~1 session.**
  (The prolog `.pl`→Perl detection bug that was mis-filed in D moves to Option 3.)

**Option 5 — Disk-absent externals (not resolver).** Install/walk deps; add Dart + CRAN locators
after discussion. ~0.35M recovered as real external edges.

### Recommended order
**4b → 1(B2) → 2 → 1(B1,B3) → 4a → 1(B4,B5) → 3 → 5.** Cheap accounting/dedup first so the metric
stops lying (4b); take the free namespaceless flips (B2) and the cheap data sweep (2); then the
high-value generic rungs (B1 import-rebind, B3 prelude-qualify); slot the vendored-upstream
reclassification (4a) once its provenance signal is agreed; then the hard receiver-projection (B4)
and the long tail (B5, 3); externals install (5) in parallel anytime. The old plan's "builtin_skip
first" is superseded — the generic rungs are where the volume now lives.
