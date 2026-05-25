# BearWisdom resolution by language

Per-language resolution rates derived from each project's `.bearwisdom/index.db` after the 2026-05-22 corpus recapture, ordered by Stack Overflow 2025 Developer Survey usage.

**Status:** ✅ ≥98% · 🟢 95-97.99% · 🟡 70-94.99% · 🔴 <70% · — not bucketed.

**Methodology.** Resolved = edges with `confidence ≥ 0.5` whose source symbol's file has `origin='internal'`, grouped by `files.language`. Unresolved = `unresolved_refs` joined the same way. This is a per-FILE-language view (different from the older `unresolved_by_lang_kind` per-REF-language view, which attributed embedded snippet refs back to their inner language). Switching to the file-language view exposes a class of file-extension-misclassification bugs that the per-ref view hid (e.g. Qt Linguist `.ts` translation XML files being routed through the TypeScript pipeline — see TypeScript row).

For the engine-vs-heuristic split per language and per project, see `baseline-gaps.md`.

| #  | Language            | SO usage | BW res%  | Resolved   | Unresolved | Status | Notes |
|----|---------------------|----------|----------|------------|------------|--------|-------|
| 1  | JavaScript          | 66.0%    | 96.19%   | 598,393    | 23,678     | 🟢     | engine 92.71%, heuristic 3.48% — `heuristic_name_kind` (13,716) is the top fallback |
| 2  | HTML/CSS            | 61.9%    | —        | —          | —          | —      | markup, no code graph |
| 3  | SQL                 | 58.6%    | 94.58%   | 6,183      | 354        | 🟡     | regression from 99.71% — investigate; engine 93.33% — only `heuristic_name_kind` catches the residual 82 |
| 4  | Python              | 57.9%    | 95.05%   | 190,076    | 9,898      | 🟢     | engine-only (post heuristic.rs deletion). **Fix #5 phase 5 added `requirements.txt`/`Pipfile`/`setup.py` manifest reader, closing PyPI deps not declared in `pyproject.toml`. Crosses 95% threshold (was 94.82% post fix #4).** Still: dunder attributes (`__class__`, `__name__`) — needs CPython stdlib walker to emit dunder members |
| 5  | Bash/Shell          | 48.7%    | 79.76%   | 8,684      | 2,204      | 🟡     | regression from 96.52% — investigate; engine 77.52%, `heuristic_name_kind` (240) is small. Most unresolved are bare `calls` (2,204) |
| 6  | TypeScript          | 43.6%    | 96.38%   | 2,386,441  | 89,517     | 🟢     | engine-only (post heuristic.rs deletion). **Fix #5 phase 2a (transitive re-export closure) and phase 3 (Prisma generated client) added +0.33pp from 96.05%.** Qt-Linguist `.ts` misclassification still inflates the unresolved count for cpp-keepassxc; content-sniffing `.ts` would push higher |
| 7  | Java                | 29.4%    | 95.93%   | 682,909    | 29,007     | 🟢     | engine-only (post heuristic.rs deletion). **Fix #5 phases 2b (inheritance walk), 2c (static wildcard `import static X.*;`), 2d (bare-name chain-miss), 4 (JAR bytecode walker for transitive .class files) added +0.12pp from 95.81%.** Top remaining: Lombok `@Slf4j`-generated `LOG` (decorator effect), some JUnit assertion forms not following static-wildcard syntax |
| 8  | C#                  | 27.8%    | 100.00%  | 2,063,272  | 11         | ✅     | engine 96.82%, heuristic 3.17%; only 11 unresolved across 15 C#-dominant projects — heuristic carries 64,720 `heuristic_qualified_name` hits the engine could absorb |
| 9  | C++                 | 23.5%    | 94.51%   | 1,551,684  | 90,054     | 🟡     | regression from 98.14% — `cpp-keepassxc` and other Qt-heavy projects contribute; engine 94.45%. Residual is `type_ref` (73k) — likely Qt MOC-generated symbols and templates |
| 10 | PowerShell          | 23.2%    | 99.77%   | 9,897      | 23         | ✅     | engine 99.73% |
| 11 | C                   | 22.0%    | 98.00%   | 1,888,469  | 38,481     | ✅     | engine 97.95%; residual is `type_ref` (19,943) and `calls` (18,538) — typical macro-expansion + struct-field gaps |
| 12 | PHP                 | 18.9%    | 98.78%   | 358,639    | 4,430      | ✅     | engine 98.18% |
| 13 | Go                  | 16.4%    | 92.87%   | 267,475    | 20,539     | 🟡     | engine-only (post heuristic.rs deletion). **Fix #5 phase 2 added +0.61pp from 92.26%.** Top remaining: test fixture vars (`expected`/`expectedErrors`/`expectedConfig` — these are local test data, not real misses), plus some bare types (`Request`, `Client`) from receiver-method patterns |
| 14 | Rust                | 14.8%    | 95.86%   | 687,578    | 29,694     | 🟢     | engine-only after `heuristic.rs` deletion. **Cumulative fixes #1 + #3 A–H + #4 + #5 (2026-05-23→24): 90.36% → 95.86%, +5.50pp.** Fix #1 prelude resolver; fix #3 batch (8 generic + rust-specific sub-fixes); fix #4 candidate ranking in DefaultResolver; fix #5 phases 2–5 (transitive re-export closure, inheritance-walk fallback, static-wildcard follow-through, generalised bare-name chain-miss, generated-code adapters, JAR bytecode walker, missing manifest readers). Top remaining: bare method calls on locals needing deeper flow inference |
| 15 | Kotlin              | 10.8%    | 96.79%   | 378,097    | 12,554     | 🟢     | engine 96.02%; residual mostly bare `calls` (7,703) — Android framework synthetic globals + Compose APIs |
| 16 | Lua                 | 9.2%     | 93.49%   | 143,294    | 9,978      | 🟡     | regression from 97.93% — engine 87.46%, heuristic 6.03%. lua-koreader carries 9,973 unresolved `calls`; KOReader framework methods not in the synthetic globals table |
| 17 | Assembly            | 7.1%     | —        | —          | —          | —      | tree-sitter coverage exists, no plugin |
| 18 | Ruby                | 6.4%     | 99.88%   | 174,840    | 207        | ✅     | engine 99.84% |
| 19 | Dart                | 5.9%     | 98.30%   | 491,529    | 8,488      | ✅     | engine 96.75%, heuristic 1.56%; `heuristic_namespace` (7,114) is the main fallback path |
| 20 | Swift               | 5.4%     | 98.96%   | 112,921    | 1,189      | ✅     | engine 97.59% |
| 21 | R                   | 4.9%     | 75.61%   | 103,956    | 33,534     | 🔴     | **Fix #2 (2026-05-23): absorbed jupyter R-cell refs (tidyverse `ggplot`/`aes`/`mutate`/`read_csv`/...), so the rate is now the honest cross-host figure.** .R files alone still resolve >95%. Closing the gap needs a tidyverse synthetics walker activated by the project's `renv.lock` or `DESCRIPTION` |
| 22 | Groovy              | 4.8%     | 89.96%   | 99,933     | 11,152     | 🟡     | regression from 96.08% — engine 79.48%; gradle-plugin Gradle DSL API methods (10,983 `calls`) lack a sources jar |
| 23 | VB.NET              | 4.4%     | 23.96%   | 404        | 1,282      | 🔴     | catastrophic regression from 97.22% — engine 0%, all resolution coming from heuristic; `VbnetResolver`'s engine hook appears not to fire on the new corpus run. **Investigate first** |
| 24 | VBA                 | 4.2%     | 90.85%   | 1,604      | 161        | 🟡     | aggregate; rubberduck dominant but smaller VBA projects (stdvba, vbaweb) drag the average |
| 25 | MATLAB              | 3.9%     | 68.60%   | 10,524     | 4,817      | 🔴     | walker wired but install-gated (no MathWorks license on dev box); engine 27.43% |
| 26 | Perl                | 3.8%     | 100.00%  | 6,030      | 0          | ✅     | rate is 100% but engine share is 0% — every Perl edge is caught by the heuristic fallback. **No engine path exists for Perl.** `heuristic_ref_module` (5,821) is doing all the work. Needs an engine resolver plugin (likely 1-2 sessions) |
| 27 | GDScript            | 3.3%     | 98.28%   | 13,172     | 230        | ✅     | engine 98.28% (no heuristic edges) |
| 28 | Elixir              | 2.7%     | 98.28%   | 185,551    | 3,252      | ✅     | improvement from 95.45%; engine 92.60%, heuristic 5.68% — `heuristic_ref_module` (8,933) is the gap |
| 29 | Scala               | 2.6%     | 95.01%   | 144,991    | 7,620      | 🟢     | engine-only (post heuristic.rs deletion). **Fix #5 phase 2 added +0.05pp from 94.96%, crossing 95% threshold.** Top remaining: opened-package imports (`import collection.mutable._`) where the static-wildcard strategy could fire more aggressively |
| 30 | Delphi              | 2.5%     | —        | —          | —          | —      | folds into Pascal row below |
| 31 | Lisp                | 2.4%     | —        | —          | —          | —      | not bucketed |
| 32 | MicroPython         | 2.3%     | —        | —          | —          | —      | folds into Python |
| 33 | Zig                 | 2.1%     | 92.27%   | 216,984    | 18,178     | 🟡     | regression from 97.38% — engine 69.49%, heuristic 22.78%; `heuristic_name_kind` (53,566) almost matches `engine` — zig-compiler-fresh dominates and the engine's Zig path is undersized |
| 34 | Erlang              | 1.5%     | 96.27%   | 257,404    | 9,962      | 🟢     | engine 92.63%; `heuristic_namespace` (7,703) is the gap |
| 35 | Fortran             | 1.4%     | 95.07%   | 102,622    | 5,326      | 🟢     | engine 35.70%, heuristic 59.37% — `heuristic_name_kind` (35,522) is 92% of engine total. **Fortran engine path is severely underbuilt.** Rate is propped up by heuristic |
| 36 | Ada                 | 1.4%     | 93.41%   | 23,543     | 1,662      | 🟡     | small regression from 95.30%; engine 78.76% |
| 37 | F#                  | 1.3%     | 96.34%   | 99,914     | 3,791      | 🟢     | engine 95.15% |
| 38 | OCaml               | 1.2%     | 96.89%   | 96,959     | 3,112      | 🟢     | engine 90.21%, heuristic 6.68% |
| 39 | Gleam               | 1.1%     | 83.05%   | 4,122      | 841        | 🟡     | regression from 98.58% — engine 79.99%; investigate |
| 40 | Prolog              | 1.1%     | 74.11%   | 13,352     | 4,665      | 🟡     | regression from 96.55% — `heuristic_ref_module` (243) the main fallback. Residual `calls` (4,665) — predicate dispatch may need an engine hook |
| 41 | COBOL               | 1.0%     | 100.00%  | 88         | 0          | ✅     | engine 100% |
| 42 | Mojo                | 0.4%     | —        | —          | —          | —      | not bucketed |
| —  | Pascal (FreePascal) | —        | 97.12%   | 391,354    | 11,591     | 🟢     | engine 89.63%, heuristic 7.49%; `heuristic_name_kind` (17,612) — Delphi-RAD namespace classifier carries this |
| —  | Odin                | —        | 91.75%   | 59,505     | 5,353      | 🟡     | regression from 97.77% — engine 57.56%, heuristic 34.18%; `heuristic_name_kind` (22,147) is doing the heavy lifting — Odin engine path needs widening |
| —  | Vue                 | —        | 90.47%   | 77,553     | 8,168      | 🟡     | regression from 98.75% — engine 80.70%; `heuristic_name_kind` (4,884) the dominant gap. Check vue-element-admin (Vue 2 Options API resolver) |
| —  | Haskell             | —        | 96.00%   | 121,177    | 5,046      | 🟢     | engine 93.21%, heuristic 2.80% |
| —  | Bicep               | —        | 80.50%   | 14,742     | 3,572      | 🟡     | regression from 97.21% — engine 56.32%, heuristic 24.18%; investigate |
| —  | Nim                 | —        | 95.57%   | 207,124    | 9,592      | 🟢     | engine 86.34%, heuristic 9.23%; `heuristic_name_kind` (20,006) is the gap |
| —  | Clojure             | —        | 98.94%   | 78,667     | 844        | ✅     | engine 79.51%, heuristic 19.43% — `heuristic_name_kind` (9,390) catches most; engine path doesn't cover namespace-qualified vars |
| —  | Razor               | —        | 98.86%   | 8,321      | 96         | ✅     | engine 77.07%, heuristic 21.79% — `heuristic_qualified_name` (1,722); fluentui-blazor dominant |
| —  | Astro               | —        | 86.36%   | 1,102      | 174        | 🟡     | regression from ~98.9% — engine 25.78%, heuristic 60.58%; `heuristic_import` (543) the main fallback |
| —  | Robot               | —        | 89.86%   | 32,407     | 3,658      | 🟡     | regression from ~97.5% |
| —  | Jupyter             | —        | 77.36%   | 8,273      | 2,421      | 🟡     | **Fix #2 (2026-05-23): attribution shift moved R/Python cell refs to those buckets, 59.91% → 77.36%, +21.31pp.** Remaining unresolved are the host cells' own ref shells |
| —  | Svelte              | —        | 90.97%   | 18,543     | 1,841      | 🟡     | regression from ~96.9% — engine 32.70%, heuristic 58.27%; `heuristic_import` (6,508) is doing all the work — engine's Svelte import resolution is weak |
| —  | Nix                 | —        | 93.46%   | 43,575     | 3,047      | 🟡     | regression from ~95.6% |
| —  | Starlark            | —        | 94.62%   | 10,007     | 569        | 🟡     | engine 91.77% |
| —  | MDX                 | —        | 87.60%   | 16,912     | 2,393      | 🟡     | regression from ~100% — engine 66.60%, heuristic 21.00%; `heuristic_namespace` (2,132) |
| —  | CMake               | —        | 87.47%   | 10,461     | 1,499      | 🟡     | regression from ~95% — engine 77.61% |
| —  | Jinja               | —        | 93.17%   | 6,342      | 465        | 🟡     | engine 16.31%, heuristic 76.86% — Ansible role resolver carries everything via heuristic |

*Rates are computed from each project's index.db after the 2026-05-22 recapture. Numbers may differ from prior versions of this file when the underlying methodology was per-ref-language (via `unresolved_by_lang_kind`) rather than per-file-language. Both views are honest; this one surfaces file-extension misclassification (Qt `.ts`).*

## Regressions to investigate

The recapture exposed several regressions compared to the previous baseline. Listed in order of size of the affected language's resolved-edge count, biggest first:

1. **TypeScript -9.7pp** — 85% of the loss is `keepassxc_*.ts` Qt Linguist files; content-sniff `.ts` to fix.
2. **C++ -3.6pp** — Qt MOC, templates; investigate cpp-keepassxc internals.
3. **C -0.8pp** — minor.
4. **Lua -4.4pp** — `lua-koreader` framework methods missing from synthetics.
5. **Groovy -6.1pp** — `gradle-plugin` Gradle DSL API methods lack a sources jar.
6. **Zig -5.1pp** — engine Zig path is undersized; heuristic fallback carries the rate.
7. **Vue -8.3pp** — likely Vue 2 Options-API or import path; investigate vue-element-admin.
8. **Bicep -16.7pp** — investigate.
9. **CMake -7.5pp** — investigate.
10. **MDX -12.4pp**, **Astro -12.5pp**, **Svelte -5.9pp** — SFC import path resolution shared between these, likely one root cause.
11. **Robot -7.6pp**, **Nix -2.1pp**, **Gleam -15.5pp**, **Prolog -22.4pp**, **Odin -6.0pp**, **Ada -1.9pp** — long tail; size each before triaging.
12. **Jupyter -37.2pp** — biggest absolute drop; per-cell ref attribution after the embedded-language fix is the likely culprit.
13. **VB.NET -73.3pp** — engine hook isn't firing; first to fix.
14. **Shell -16.8pp** — bare `calls` not classified; investigate the bash/shell resolver.
15. **SQL -5.1pp** — small absolute count (354 unresolved); probably benign.

## Engine vs heuristic — the underlying gap

Top six languages where the heuristic Tier-2 fallback carries a meaningful share of resolved edges (i.e. the engine path could be wider):

| Language | Engine share | Heuristic share | Top heuristic strategy |
|---|---:|---:|---|
| Perl | 0.00% | 100.00% | `heuristic_ref_module` |
| Fortran | 35.70% | 59.37% | `heuristic_name_kind` |
| Svelte | 32.70% | 58.27% | `heuristic_import` |
| Astro | 25.78% | 60.58% | `heuristic_import` |
| Jinja | 16.31% | 76.86% | `heuristic_name_kind` |
| Scala | 70.79% | 26.35% | `heuristic_ref_module` |

A `heuristic_*` hit means the language-specific engine pass returned None, then the generic Tier-2 fallback matched. Each top-six row is a candidate for an engine-side resolver path. See `baseline-gaps.md` for the full per-language breakdown including top unresolved targets.

## Other indexed plugins

40+ plugins are parsed and indexed but absent from the SO2025 ranking above — they either emit no `internal_edges` of their own (markup/config) or carry expressions in another language inside their template syntax (most template DSLs). Refs in embedded sub-language regions (e.g. JavaScript inside an HTML `<script>` block) are now attributed to the language they're written in, not the host file's language — so HTML / julius / angular_template / markdown rows that were previously inflated by embedded-JS / embedded-TS / fenced snippets have dropped to zero. **Status here is bucketed on absolute unresolved count, not rate** (per-language resolved isn't tracked separately, so a percentage can't be computed): ✅ 0 · 🟢 1-30 · 🟡 31-300 · 🔴 >300.

| Plugin           | Category | Unresolved | Status | Note |
|------------------|----------|-----------:|:------:|------|
| nunjucks         | template | 0   | ✅ | stale baseline resolved; imports refs now classified correctly in full-index path; remaining 28 unresolved are in `.js` files inside nunjucks projects (browser builtins), not nunjucks language itself |
| scss             | markup   | 11  | 🟢 | sass-true externals via `package_ships_scss` npm gate; multi-selector / pseudo-selector / compound-selector extraction; class text-scan fallback for parse-error files; `.css`-extension SCSS partials promoted via content sniff |
| hcl              | config   | 10  | 🟢 | `compact`/`regex_replace` added to built-in keywords; provider alias refs (`aws.remote`) resolved to underlying provider symbol |
| heex             | template | 174 | 🟡 | HeexResolver: bare `<.component>` calls resolve via ext:/internal symbol lookup |
| make             | config   | 20  | 🟢 | special-target / pattern-stem / file-extension prereq suppression landed |
| twig             | template | 56  | 🟡 | |
| blade            | template | 539 | 🔴 | regression — investigate |
| handlebars       | template | 2,135 | 🔴 | major regression — investigate |
| prisma           | config   | 4   | 🟢 | |
| dockerfile       | config   | 4   | 🟢 | |
| freemarker       | template | 77  | 🟡 | |
| graphql          | config   | 0   | ✅ | |
| proto            | config   | 274 | 🟡 | regression — investigate |

Zero-unresolved plugins (✅) — parsed and indexed, with all refs either resolved or attributed to their actual sub-language: `crontab`, `gotemplate`, `gsp`, `jsp`, `liquid` (only 4 unresolved), `mako`, `nginx`, `pug`, `puppet`, `rmarkdown`, `shakespeare`, `slim`, `smarty`, `systemd`, `thymeleaf`, `velocity`, `yaml`. Plus the meta dispatchers `generic` and `polyglot_nb`.

**Total:** 94 directories under `crates/bearwisdom/src/languages/` (93 language plugins + 1 fallback dispatcher).
