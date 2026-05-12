# BearWisdom resolution by language

Per-language resolution rates from `baseline-all.json`, ordered by Stack Overflow 2025 Developer Survey usage.

**Status:** ✅ ≥98% · 🟢 95–97.99% · 🟡 70–94.99% · 🔴 <70% · — not bucketed.

| #  | Language            | SO usage | BW res%  | Resolved   | Unresolved | Status | Notes |
|----|---------------------|----------|----------|------------|------------|--------|-------|
| 1  | JavaScript          | 66.0%    | 96.19%   | 420,600    | 16,676     | 🟢     |       |
| 2  | HTML/CSS            | 61.9%    | —        | —          | —          | —      | markup, no code graph |
| 3  | SQL                 | 58.6%    | 99.71%   | 104,387    | 301        | ✅     |       |
| 4  | Python              | 57.9%    | 96.37%   | 74,181     | 2,794      | 🟢     |       |
| 5  | Bash/Shell          | 48.7%    | 96.52%   | 57,871     | 2,086      | 🟢     |       |
| 6  | TypeScript          | 43.6%    | 97.93%   | 1,065,348  | 22,464     | 🟢     |       |
| 7  | Java                | 29.4%    | 95.61%   | 346,879    | 15,939     | 🟢     |       |
| 8  | C#                  | 27.8%    | 100.00%  | 1,008,377  | 48         | ✅     |       |
| 9  | C++                 | 23.5%    | 98.14%   | 117,730    | 2,189      | ✅     | dedicated C++ TestProjects; cross-corpus aggregate 90.29% includes bundled C++ in non-C++ projects |
| 10 | PowerShell          | 23.2%    | 99.90%   | 19,228     | 20         | ✅     |       |
| 11 | C                   | 22.0%    | 98.77%   | 2,218,510  | 27,725     | ✅     |       |
| 12 | PHP                 | 18.9%    | 98.56%   | 270,287    | 3,947      | ✅     |       |
| 13 | Go                  | 16.4%    | 98.92%   | 172,579    | 1,886      | ✅     |       |
| 14 | Rust                | 14.8%    | 97.82%   | 203,120    | 4,527      | 🟢     |       |
| 15 | Kotlin              | 10.8%    | 95.69%   | 324,141    | 14,602     | 🟢     |       |
| 16 | Lua                 | 9.2%     | 97.93%   | 585,997    | 12,101     | 🟢     |       |
| 17 | Assembly            | 7.1%     | —        | —          | —          | —      | tree-sitter coverage exists, no plugin |
| 18 | Ruby                | 6.4%     | 99.84%   | 127,801    | 199        | ✅     |       |
| 19 | Dart                | 5.9%     | 96.86%   | 235,700    | 7,644      | 🟢     |       |
| 20 | Swift               | 5.4%     | 97.82%   | 57,170     | 1,272      | 🟢     |       |
| 21 | R                   | 4.9%     | 100.00%  | 9,481      | 0          | ✅     |       |
| 22 | Groovy              | 4.8%     | 96.08%   | 100,340    | 4,098      | 🟢     | type inference for chained instance calls landed (`endsWith`, `matching`, GradleRunner fluent API); gradle-plugin 82.09% (Gradle API methods need sources jars) |
| 23 | VB.NET              | 4.4%     | 97.22%   | 455        | 13         | ✅     | sparse OSS — large VB.NET projects rarely on GitHub |
| 24 | VBA                 | 4.2%     | 99.39%   | 92,928     | 573        | ✅     | aggregate dominated by rubberduck 99.5%; stdvba 92.2%, vbaweb 86.7% remain on Office Object Model gap |
| 25 | MATLAB              | 3.9%     | 67.55%   | 10,284     | 4,938      | 🔴     | walker wired but install-gated (no MathWorks license on dev box) |
| 26 | Perl                | 3.8%     | 96.27%   | 126,837    | 4,919      | 🟢     | corpus expanded with perl5/Moose/Catalyst/Mojo/Dancer2 |
| 27 | GDScript            | 3.3%     | 96.87%   | 13,635     | 440        | 🟢     | mod.rs keyword-wiring bug fix unblocked ~500 false unresolved; godot_api walker pulls extension_api.json |
| 28 | Elixir              | 2.7%     | 98.04%   | 173,540    | 3,470      | ✅     |       |
| 29 | Scala               | 2.6%     | 96.01%   | 173,827    | 7,215      | 🟢     |       |
| 30 | Delphi              | 2.5%     | —        | —          | —          | —      | Object Pascal-adjacent; not bucketed |
| 31 | Lisp                | 2.4%     | —        | —          | —          | —      | not bucketed |
| 32 | MicroPython         | 2.3%     | —        | —          | —          | —      | folds into Python |
| 33 | Zig                 | 2.1%     | 97.38%   | 2,462,498  | 65,711     | 🟢     |       |
| 34 | Erlang              | 1.5%     | 95.46%   | 294,588    | 14,000     | 🟢     | spec false-positive suppression + ERTS C BIF arity-strip; cowboy 98.3%, emqx 97.2%, rabbitmq 93.5% (residual project-internal cross-file refs) |
| 35 | Fortran             | 1.4%     | 95.44%   | 104,975    | 5,011      | 🟢     | fypp preprocessor subprocess + SHA-256 cache landed; stdlib 93.4 → 95.34; fpm 97.2, json 95.5 |
| 36 | Ada                 | 1.4%     | 95.30%   | 27,876     | 1,374      | 🟢     |       |
| 37 | F#                  | 1.3%     | 95.67%   | 136,164    | 6,169      | 🟢     | Paket-project empty-PackageReference fallback fixed; saturn 47→79, ionide 75→94 |
| 38 | OCaml               | 1.2%     | 95.09%   | 101,834    | 5,256      | 🟢     | multi-`.opam` union, `local_open` ctx propagation, attribute suppression, `file_stem_matches` ext: prefix fix |
| 39 | Gleam               | 1.1%     | 98.58%   | 24,364     | 351        | ✅     |       |
| 40 | Prolog              | 1.1%     | 96.55%   | 97,436     | 3,478      | 🟢     |       |
| 41 | COBOL               | 1.0%     | 100.00%  | 5,719      | 0          | ✅     |       |
| 42 | Mojo                | 0.4%     | —        | —          | —          | —      | not bucketed |
| —  | Pascal (FreePascal) | —        | 96.54%   | 450,032    | 16,120     | 🟢     | UTF-8 lossy fallback recovers Windows-1252 files; Delphi-VCL namespace classifier (`Vcl.*`, `Winapi.*`, `FireDAC.*`) reclassifies Delphi-RAD externals; heidisql 99.1 ✓, doublecmd 97.6 ✓, castle-fresh 95.7 ✓ |
| —  | Odin                | —        | 97.77%   | 107,348    | 2,453      | 🟢     |       |
| —  | Vue                 | —        | 98.75%   | 57,764     | 731        | ✅     |       |
| —  | Haskell             | —        | 95.46%   | 118,625    | 5,644      | 🟢     |       |
| —  | Bicep               | —        | 97.21%   | 124,011    | 3,565      | 🟢     |       |
| —  | Nim                 | —        | 95.71%   | 205,180    | 9,200      | ✅     | when-block proc extraction (2-space indent), pragma-annotated type extraction, enum member extraction, pkgcache fallback for build-failed packages, package-level + stdlib-any resolver passes; compiler 96.0%, nimbus 96.2%, libp2p 96.1%, nimble 96.9%, arraymancer 95.8%, nitter 88.8%, pixie 92.8% |
| —  | Clojure             | —        | ~98.6%   | ~43,000    | 611        | ✅     | rate from 3 dominant projects (babashka/datascript/ring) |
| —  | Razor               | —        | ~100%    | ~10,500    | 55         | ✅     | dotnet-fluentui-blazor at 100% |
| —  | Astro               | —        | ~98.9%   | ~6,200     | 69         | ✅     | astro-awesome-privacy dominant |
| —  | Robot               | —        | ~97.5%   | ~36,500    | 936        | 🟢     | robot-framework / robot-cookbook |
| —  | Jupyter             | —        | ~97.1%   | ~47,900    | 1,430      | 🟢     | jupyter-ml-for-beginners; cell-level extraction |
| —  | Svelte              | —        | ~96.9%   | ~36,100    | 1,155      | 🟢     | svelte-realworld / svelte-shadcn |
| —  | Nix                 | —        | ~95.6%   | ~66,500    | 3,059      | 🟢     | dream2nix's flake outputs lower the average; home-manager solid |
| —  | Starlark            | —        | ~95.4%   | ~5,200     | 249        | 🟢     | bazel-skylib / rules-python |
| —  | MDX                 | —        | ~100%    | ~1,000     | 0          | ✅     | astro-starlight MDX-specific refs all resolved; SFC default-import name fallback (`.astro`/`.svelte`) + Fragment suppression closed 999 refs |
| —  | CMake               | —        | ~95.0%   | —          | ~41        | 🟢     | cmake-cpm cmake-only 96.6%; CPM_ prefix suppressed as builtin, `_SOURCE_DIR`/`_BINARY_DIR` suffix classified as fetched-external, CPM-using files' unresolved `Calls` targets classified as cpm-package external, nested-var-ref artifact names containing `}` suppressed at extraction |
| —  | Jinja               | —        | 95.04%   | 10,189     | 531        | 🟢     | Ansible role resolver: `requirements.yml` manifest reader, role-variable symbols emitted from `roles/<role>/defaults`/`vars`, `group_vars/<group>.yml`, `host_vars/*.yml`; `infer_external_namespace` routes declared-external-role-prefixed refs to `external_refs`. matrix-ansible 85.0 → **98.4%** ✅. kubespray 90.5% unchanged — residual splits into (a) Ansible runtime magic vars (`hostvars`, `inventory_hostname`, `lookup`, `group_names`, ~50 refs) needing an `ansible-runtime` ambient ecosystem, and (b) project-specific vars (`node_pod_cidr`, `kubeadm_token`, ~380 refs) defined under the directory-form `inventory/group_vars/all/<topic>.yml` layout the current extractor doesn't yet walk |

*Rates marked `~` are weighted averages from projects where the language is ≥30% of files. Resolved counts marked `~` are derived (`unresolved × rate / (1 − rate)`) rounded to the nearest 100 — per-language resolved-edge counts aren't tracked separately in `baseline-all.json`. Razor uses the dominant project's edge total (`dotnet-fluentui-blazor`); MDX uses the SFC-fix delta as a floor (real per-language total is higher).*

## Other indexed plugins

40+ plugins are parsed and indexed but absent from the SO2025 ranking above — they either emit no `internal_edges` of their own (markup/config) or carry expressions in another language inside their template syntax (most template DSLs). Refs in embedded sub-language regions (e.g. JavaScript inside an HTML `<script>` block) are now attributed to the language they're written in, not the host file's language — so HTML / julius / angular_template / markdown rows that were previously inflated by embedded-JS / embedded-TS / fenced snippets have dropped to zero. **Status here is bucketed on absolute unresolved count, not rate** (per-language resolved isn't tracked separately, so a percentage can't be computed): ✅ 0 · 🟢 1–30 · 🟡 31–300 · 🔴 >300.

| Plugin           | Category | Unresolved | Status | Note |
|------------------|----------|-----------:|:------:|------|
| nunjucks         | template | 0   | ✅ | stale baseline resolved; imports refs now classified correctly in full-index path; remaining 28 unresolved are in `.js` files inside nunjucks projects (browser builtins), not nunjucks language itself |
| scss             | markup   | 73  | 🟡 | `.sass` indented-syntax recovery + `@use ... as alias` round-trip landed; residual is `@include alias.mixin()` patterns where the alias targets an unindexed npm package |
| hcl              | config   | 58  | 🟡 | Terraform / Nomad data-model traversal (`var.x`, `module.y.output.z`) |
| heex             | template | 1   | ✅ | HeexResolver: bare `<.component>` calls now resolve via ext:/internal symbol lookup; residual is a JS array method in an embedded `<script>` block |
| make             | config   | 29  | 🟢 | special-target / pattern-stem / file-extension prereq suppression landed; residual is Erlang-define-inside-Makefile parse artifacts |
| twig             | template | 26  | 🟢 | |
| blade            | template | 11  | 🟢 | |
| handlebars       | template | 9   | 🟢 | |
| prisma           | config   | 4   | 🟢 | |
| dockerfile       | config   | 4   | 🟢 | |
| freemarker       | template | 3   | 🟢 | |
| graphql          | config   | 2   | 🟢 | |
| proto            | config   | 2   | 🟢 | |

Zero-unresolved plugins (✅) — parsed and indexed, with all refs either resolved or attributed to their actual sub-language: `angular`, `angular_template`, `crontab`, `eex`, `ejs`, `erb`, `gotemplate`, `gsp`, `haml`, `hare`, `html`, `jsp`, `julius`, `liquid`, `mako`, `markdown`, `nginx`, `pug`, `puppet`, `rmarkdown`, `shakespeare`, `slim`, `smarty`, `systemd`, `templ`, `thymeleaf`, `velocity`, `yaml`. Plus the meta dispatchers `generic` and `polyglot_nb`.

**Total:** 94 directories under `crates/bearwisdom/src/languages/` (93 language plugins + 1 fallback dispatcher).
