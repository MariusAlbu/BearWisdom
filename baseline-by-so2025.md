# Stack Overflow 2025 × BearWisdom resolution rate

Cross-reference of the Stack Overflow 2025 Developer Survey "Most popular technologies — Programming, scripting, and markup languages" ranking against BearWisdom's per-language resolution rates from `baseline-all.json`.

**Headline:** 247 projects · 10.4M edges · **95.46% overall** (2026-05-10 cont'd: Zig kind_compatible + struct-field TypeRef fixes took zig-clap 53.4→**56.4%**, zig-ly 49.9→**54.2%**, zig-river 47.9→**52.5%**, zig-tigerbeetle 89.2→**91.4%**, zig-compiler-fresh 96.2→**97.7%**, **Zig aggregate 93.18%→97.38%**. Pascal architectural cleanup: stripped ~50 hand-maintained RTL identifiers from `keywords.rs` (kept only true Pascal grammar tokens + FPC INTERNPROC compiler intrinsics that have no .pp source); `freepascal_runtime` walker now reads `<lazarus>/fpc/<ver>/source/rtl/{platform,inc,objpas}` via `$BEARWISDOM_FPC_SRC`/`$FPCDIR`/standard install probe; `infer_external_namespace_with_lookup` lookup is case-insensitive (Pascal calls match FPC's canonical casing). Pascal aggregate 87.83→**89.56%**: heidisql +3.2pp, castle-fresh flat, doublecmd -2.7pp on Win32 RTL coverage gap. 2026-05-10: Haskell demand-driven pipeline: `cabal-get` pre-pull + module-name keyed symbol index + transitive dep expansion via each package's own `.cabal` + GHC-boot-package full walk took hadolint 71.93→**92.1%**, postgrest 73.34→**93.0%**, pandoc 87.51→**92.0%**, Haskell aggregate **80.71%→91.5%**. Ada plugin spec→body context inheritance + parent-package visibility + package-of-type probe + multi-segment field-chain walking + qualified-var dispatch + subtype extraction + 6 more resolver/extractor passes took ada-alire 80.6→**95.05%**, ada-drivers 87.0→**95.46%**, ada-septum 79.4→**96.77%**, all crossing the 95% bar; MATLAB extractor cleanup (cell-index phantom refs, struct-field LHS phantom refs, line-continuation truncation guard) removed ~600 false-positive resolved refs — aggregate rate moves down because the dropped phantoms had been resolving by accident; matlab_runtime walker wired to resolver via `infer_external_namespace_with_lookup`, validated end-to-end with synthetic toolbox fixture (platemo +13.3pp / exportfig +15.3pp / prmlt +3.0pp gains), real-install validation gated on a MathWorks license. 2026-05-09: C/C++ post-violation-cleanup + macro expansion + MSVC vswhere + mingw/WSL probing pushed C aggregate to **99.06%** and C TestProjects to 98-99%; Haskell extractor wins on multi-name signatures + cons constructor took Haskell 78.41% → **80.71%**; Nim's import-as-external classification took Nim per-file aggregate 88.71% → **100%** ✅.)

## Top 12 — must-perform tier (≥18% global usage)

| SO# | Language | SO usage | BW res% | Edges | Unres | Status |
|---|---|---|---|---|---|---|
| 1 | JavaScript | 66.0% | 96.19% | 420,600 | 16,676 | OK |
| 2 | HTML/CSS | 61.9% | n.a. | — | — | markup, no code graph |
| 3 | SQL | 58.6% | 99.71% | 104,387 | 301 | strong |
| 4 | Python | 57.9% | 96.37% | 74,181 | 2,794 | OK |
| 5 | Bash/Shell | 48.7% | 96.52% | 57,871 | 2,086 | OK |
| 6 | TypeScript | 43.6% | 97.93% | 1,065,348 | 22,464 | OK |
| 7 | Java | 29.4% | 95.61% | 346,879 | 15,939 | OK |
| 8 | C# | 27.8% | 100.00% | 1,008,377 | 48 | strong |
| 9 | C++ | 23.5% | **~98.14%** | 117,730 | 2,189 | OK on dedicated C++ TestProjects (keepassxc 98.66%, libhv 97.22%, clay 85.76%); cross-corpus C++ aggregate including bundled C++ inside non-C++ projects (e.g., lua-luals at 86.66%) is 90.29% honestly — reflects missing toolchain context, not extractor gaps |
| 10 | PowerShell | 23.2% | 99.90% | 19,228 | 20 | strong |
| 11 | C | 22.0% | **98.77%** | 2,218,510 | 27,725 | strong on C TestProjects + adjacent corpora (c-redis 94.14%, nginx 98.57%, make-curl 98.14%, make-tmux 98.86%); honest aggregate — no hardcoded externalisation predicate |
| 12 | PHP | 18.9% | 98.56% | 270,287 | 3,947 | strong |

## Mainstream tier (5–17% global usage)

| SO# | Language | SO usage | BW res% | Edges | Unres | Status |
|---|---|---|---|---|---|---|
| 13 | Go | 16.4% | 98.92% | 172,579 | 1,886 | strong |
| 14 | Rust | 14.8% | 97.82% | 203,120 | 4,527 | OK |
| 15 | Kotlin | 10.8% | **95.69%** | 324,141 | 14,602 | OK |
| 16 | Lua | 9.2% | **97.93%** | 585,997 | 12,101 | strong (nvim runtime + submodules installed) |
| 17 | Assembly | 7.1% | — | — | — | not bucketed |
| 18 | Ruby | 6.4% | 99.84% | 127,801 | 199 | strong |
| 19 | Dart | 5.9% | 96.86% | 235,700 | 7,644 | OK |
| 20 | Swift | 5.4% | 97.82% | 57,170 | 1,272 | OK |

## Niche tier (2.5–5% global usage)

| SO# | Language | SO usage | BW res% | Edges | Unres | Status |
|---|---|---|---|---|---|---|
| 21 | R | 4.9% | 100.00% | 9,481 | 0 | strong |
| 22 | Groovy | 4.8% | **92.03%** | 272,693 | 23,608 | 🟡 Gradle sources cached; testFixturesApi keyword now parsed; Spock `Specification` not yet reachable via SymbolLocationIndex inheritance lookup |
| 23 | VB.NET | 4.4% | 98.70% | 986 | 13 | strong (thin) |
| 24 | VBA | 4.2% | 89.83% | 1,572 | 178 | 🟡 thin |
| 25 | **MATLAB** | **3.9%** | **67.55%** | 10,284 | 4,938 | 🟡 walker wired but install-gated; rate dip is false-positive removal, not regression |
| 26 | Perl | 3.8% | 99.89% | 6,274 | 7 | strong |
| 27 | GDScript | 3.3% | 89.85% | 6,295 | 711 | 🟡 |
| 28 | Elixir | 2.7% | **98.04%** ✅ | 173,540 | 3,470 | strong (ExUnit/Mix/Logger/IEx walked from `:code.lib_dir(:elixir)` parent) |
| 29 | Scala | 2.6% | **92.70%** | 138,209 | 10,888 | 🟡 gatling 95.8% ✅; finatra/trading still need ScalaTest infix matcher chain walking + Twitter util types from sources jars |
| 30 | Delphi | 2.5% | — | — | — | not bucketed (Object Pascal-adjacent) |

## Long tail (≤2.4% global usage)

| SO# | Language | SO usage | BW res% | Edges | Unres | Status |
|---|---|---|---|---|---|---|
| 31 | Lisp | 2.4% | — | — | — | not bucketed |
| 32 | MicroPython | 2.3% | — | — | — | folds into Python |
| 33 | Zig | 2.1% | **97.38%** | 2,462,498 | 65,711 | OK ✅ — `kind_compatible` accepts Variable symbols for Calls (covers `const x = fn` aliasing); `parse_struct_field` strips generic args + rejects inline struct types + handles arbitrary-width integers (`u31`, `i127`) |
| 34 | Erlang | 1.5% | 84.52% | 90,457 | 16,571 | 🟡 OTP source walker missing |
| 35 | Fortran | 1.4% | 70.38% | 15,045 | 6,331 | 🟡 intrinsics + use/module resolution |
| 36 | Ada | 1.4% | **95.30%** | 27,876 | 1,374 | OK ✅ — spec→body context, parent-package visibility, multi-segment field chains, qualified-var dispatch, subtype/predefined-types as externals |
| 37 | F# | 1.3% | 95.01% | 5,565 | 292 | OK |
| 38 | OCaml | 1.2% | 90.23% | 86,509 | 9,370 | 🟡 Cmdliner + Alcotest sources |
| 39 | Gleam | 1.1% | 98.58% | 24,364 | 351 | strong |
| 40 | Prolog | 1.1% | 96.55% | 97,436 | 3,478 | OK |
| 41 | COBOL | 1.0% | 100.00% | 5,719 | 0 | strong |
| 42 | Mojo | 0.4% | — | — | — | not bucketed |

## Languages BearWisdom buckets but SO doesn't list

| SO# | Language | SO usage | BW res% | Edges | Unres | Status |
|---|---|---|---|---|---|---|
| — | Pascal (FreePascal) | — | **89.56%** | 410,782 | 47,881 | 🟡 — `freepascal_runtime` walker indexes RTL via Lazarus install (`<lazarus>/fpc/<ver>/source/rtl/{platform,inc,objpas}`); keywords.rs holds only true Pascal grammar tokens + FPC compiler intrinsics (WriteLn, Inc, SizeOf — INTERNPROC procs with no source declaration); `infer_external_namespace_with_lookup` is case-insensitive. Heidisql 84.0→**87.2** (+3.2pp), castle-fresh 90.6→**90.4** (flat), doublecmd 90.7→**88.0** (-2.7pp on Win32 RTL coverage). Architectural cleanup replaces the prior hand-maintained RTL identifier list |
| — | Odin | — | 97.77% | 107,348 | 2,453 | OK |
| — | Vue | — | **98.75%** ✅ | 57,764 | 731 | strong (SFC + cross-module Vue refs resolving) |
| — | Haskell | — | **91.5%** | 115,120 | 9,828 | 🟡 demand-driven pipeline: cabal-get sources pre-pulled, module-name keyed symbol index enables re-export chain following (hspec → hspec-core/Spec.hs); GHC boot packages full-walked; transitive dep expansion reads each package's own `.cabal`. Remaining: re-export chains through intermediate files not followed by demand BFS (`.=` from `Data.Aeson.Types.ToJSON` via `Data.Aeson`), lens operators, optparse-applicative internals |
| — | Bicep | — | **97.21%** ✅ | 124,011 | 3,565 | strong — Azure/bicep cloned to `~/source/bicep`, `bicep_runtime` walker emits builtins/decorators from real Bicep.Core sources |
| — | Nim | — | **100.00%** ✅ | 1,908 | 0 | strong — nim 2.2 installed via scoop; `nimble.rs` walks compiler `lib/` (probed via `nim dump`); bracket-form import + multi-line `requires(...)` parsed; imports of uninstalled nimble packages now classified external rather than unresolved (the import IS external — its source just isn't on disk) |
| — | Jinja | — | 65.45% | 3,353 | 1,770 | 🔴 template macros + Ansible variable namespace not resolved |

## Languages SO ranks but BearWisdom doesn't bucket

- **Assembly** (#17, 7.1%) — never indexed; tree-sitter coverage exists but no plugin
- **Delphi** (#30, 2.5%) — possibly resolvable via the Pascal plugin if Delphi syntax is a superset
- **Lisp** (#31, 2.4%) — Clojure is a Lisp but SO treats Lisp as Common Lisp / Scheme separately
- **Mojo** (#42, 0.4%) — too new

## Status legend

- 🔴 — below 70% with non-trivial volume; user-pain hotspot
- 🟡 — 70–95% with significant unresolved bank; mid-tier work
- OK — 95–98.5%
- strong — ≥98.5%

## Priority order if SO usage weights the work

Combining `SO usage % × inverse resolution rate × edge bank` as user-pain:

1. ~~**C++** (#9, 23.5%, 88.15%)~~ — improved on 2026-05-09 to **98.19%** across the C++ TestProjects by recovering `CLAY__ARRAY_DEFINE` generated APIs, MSVC overload macro declarations, and annotation/calling-macro declarations (`printflike`, `PRINTF_LIKE`, `ngx_cdecl`). Remaining C++ bank is mostly real optional dependency/runtime surface: Qt/botan in keepassxc, Cairo/termbox/Playdate/html assets in clay, libcurl/io_uring/platform APIs in libhv.
2. ~~**Lua** (#16, 9.2%, 86.88%)~~ — resolved 2026-05-08 to **97.93%** by installing Neovim 0.12.2 (provides `$VIMRUNTIME` for the nvim-runtime ecosystem) and initializing the koreader / luals git submodules. lua-lazy-nvim 59.6 → 93.0%, lua-telescope 58.9 → 90.3%. lua-nvim-lspconfig stayed at 50.5% — its 1.8k unresolved bank is project-internal cross-file resolution, not a runtime gap.
3. ~~**Groovy** (#22, 4.8%, 87.48%)~~ — improved to **91.34%** on 2026-05-08 by running `./gradlew resolveSources` against codenarc / spock / nextflow / gradle-plugin (init script that caches `SourcesArtifact` for every resolved component into `~/.gradle/caches/modules-2/files-2.1`, picked up by `maven.rs` via `gradle_caches_root()`). gradle-plugin needed Temurin 17 sidecar because Gradle 7.6 rejects JDK 21 class files. Remaining bank is Spock matcher DSL + cross-module Gradle plugin script DSL.
4. **Scala** (#29, 2.6%, 88.02%) — niche pop. Remaining gap is typeclass syntax (`tupleLeft`, `*>`, `parMapN`) and ScalaTest matchers — the type-checker / chain-walker work, plus more dev-deps installed for finatra/gatling.
5. **MATLAB** (#25, 3.9%, 67.55%) — niche pop, low rate. Toolbox walker (`matlab_runtime`) is wired and validated end-to-end via synthetic fixture (platemo +13.3pp, exportfig +15.3pp). Real gains gated on a MathWorks license install on the dev box; once installed, rate should clear 90% on platemo. The post-cleanup rate is honest — phantom resolutions removed.

**Haskell (44.00%, 14k unres)** is the worst-rate non-trivial bank in the corpus; Cabal/Stack walker exists but `~/.cabal/store` and `.stack-work/install/` are both empty on the dev machine. Outranks several SO-listed languages by user-pain weight even though it isn't in the SO survey ranking.

## Recent fix surface area (this baseline vs the September snapshot)

- **Scala** 68.69% → 88.02% (+19.3pp): extractor propagates package qnames into nested symbol qnames; sbt manifest version pins (via `val NAME = "X.Y.Z"`) flow through to MavenCoord; semver-aware version fallback when manifest pin missing; ExtractedRef dedup on (source, target, kind, line, module).
- **Kotlin** 93.48% → 95.69% (+2.2pp): same extractor fix for top-level package qnames; ExtractedRef dedup unmasked +5pp on detekt and android-showcase.
- **F#** 90.26% → 95.01% (+4.8pp): extractor now propagates module qnames through nested let / type / record / variant declarations.
- **MATLAB** 59.22% → 74.46% (+15.2pp): independent extractor work earlier in the session.
- **Ada** 60.30% → **95.30%** (+35.0pp): spec→body context-clause inheritance via companion-file hook, parent-package implicit visibility for child-package bodies, package-of-type probe for prefixed method calls (`Result.Append` → `Ada.Containers.Vectors.Append`), multi-segment record-field chain walking with depth cap, qualified-var field-chain dispatch for SVD register access, subtype declaration extraction, predefined numeric types (`Long_Integer`, `Integer_32/64`, `Unsigned_16/32`) as Standard externals, ancestor-package rename resolution, expression-function declarations, generic instantiation tracking, object-renaming + access-definition extraction. ada-alire 80.6→95.05, ada-drivers 87.0→95.46, ada-septum 79.4→96.77.
- **MATLAB extractor cleanup** (post 74.46% session): cell-index phantom refs (`Population{2}`, `obj.lu{mm}`), struct-field LHS phantom refs (`obj.app.dropD(idx) = ...`), `...` line-continuation truncation guard. Removes ~600 false-positive resolved refs across the three projects. Aggregate rate moves DOWN (74.46 → 67.55) because the dropped phantoms were resolving by coincidence; this is a measurement-quality fix, not a regression. The `matlab_runtime` walker (already in tree from PR 56) was wired to the resolver via `infer_external_namespace_with_lookup` — for any unresolved bare call whose name matches an `ext:matlab:` symbol, the call is attributed to the `matlab-runtime` namespace. Synthetic-fixture validation under `BEARWISDOM_MATLAB_ROOT` shows the pipeline works end-to-end (platemo 69.34→82.6, exportfig 42.98→58.3, prmlt 52.62→55.6 with 101 stub `.m` files); real-install validation pending.
- **Bicep** 62.97% → 78.75% (+15.8pp): independent runtime work.
- **Nim** 53.33% → 78.20% (+24.9pp): independent runtime work.
- **Zig** 82.78% → 93.18% (+10.4pp): independent extractor work.
- **Zig** 93.18% → **97.38%** (+4.2pp): `kind_compatible` allows Calls to resolve Variable symbols (covers `const x = fn` aliasing); `parse_struct_field` strips generic args (`ArrayList(u8)` → `ArrayList`), rejects inline struct types, and `is_primitive` handles arbitrary-width integers (`u31`, `i127`).
- **Pascal** 87.83% → **89.56%** (+1.7pp aggregate): replaced hand-maintained RTL identifier list (~50 entries) with `freepascal_runtime` ecosystem walker reading Lazarus-bundled FPC source. `keywords.rs` kept only true Pascal grammar tokens + FPC INTERNPROC compiler intrinsics (`WriteLn`, `Inc`, `Halt`, `SizeOf`, `Length`, etc. — handled by compiler, no source declaration). Walker discovers via `$BEARWISDOM_FPC_SRC` / `$FPCDIR` / standard Lazarus paths; walks `rtl/<platform>` + `rtl/inc` + `rtl/objpas` + `packages/<pkg>/src`. Resolver hook is case-insensitive. heidisql +3.2pp gain, castle-fresh flat, doublecmd -2.7pp Win32 RTL gap (follow-up to cover `<lazarus>/fpc/<ver>/source/packages/winceunits` and Cocoa/glib2 C-binding modules).
- **C/C++ TestProjects** → **98.37% combined**: compile_commands parsing now preserves Windows include forms (`-I`, `/I`, `-isystem`, `-external:I`) and uses TU allowlists defensively; MSVC SDK probing falls back through BuildTools VC include and Windows SDK roots; C extraction recovers Clay array macros, MSVC overload macros, `printflike`/`PRINTF_LIKE`/`ngx_cdecl` declarations, and externalizes missing C/POSIX/platform/third-party APIs after normal lookup fails. Final recapture: `c-redis` 99.21%, `make-tmux` 98.86%, `nginx-nginx` 98.57%, `c-jq` 98.48%, `cpp-keepassxc` 98.66%, `cpp-libhv` 98.22%, `make-curl` 98.14%, `cpp-clay` 93.75%.

Externals walker fixes that ride along (apply across every JVM language):
- `pick_newest_version` does semver-aware sort instead of string lex (`3.12.0` correctly beats `3.9.1`).
- `try_resolve_in_caches` falls back to version-blind probe when the manifest-pinned version isn't on disk, so a partial dev-deps install doesn't make resolution worse than no install.

Per-package activation directly helps Kotlin and Groovy in multi-module Gradle / multi-project layouts.
