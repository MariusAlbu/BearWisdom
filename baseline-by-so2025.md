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
| —  | Nim                 | —        | 83.73%   | 172,371    | 33,485     | 🟡     | extractor density fixed (31× edge growth), stdlib pre-pull, implicit `system` import; stew/chronos/results/bearssl/httputils installed but rate didn't move on libp2p/nimbus — needs `eth`/`chronicles`/`nimcrypto`/`metrics`/`secp256k1` which fail to install (nimble pins 32-bit Nim that clashes with 64-bit gcc) |
| —  | Clojure             | —        | ~98.6%   | —          | 611        | ✅     | rate from 3 dominant projects (babashka/datascript/ring) |
| —  | Razor               | —        | ~100%    | —          | 55         | ✅     | dotnet-fluentui-blazor at 100% |
| —  | Astro               | —        | ~98.9%   | —          | 69         | ✅     | astro-awesome-privacy dominant |
| —  | Robot               | —        | ~97.5%   | —          | 936        | 🟢     | robot-framework / robot-cookbook |
| —  | Jupyter             | —        | ~97.1%   | —          | 1,430      | 🟢     | jupyter-ml-for-beginners; cell-level extraction |
| —  | Svelte              | —        | ~96.9%   | —          | 1,155      | 🟢     | svelte-realworld / svelte-shadcn |
| —  | Nix                 | —        | ~95.6%   | —          | 3,059      | 🟢     | dream2nix's flake outputs lower the average; home-manager solid |
| —  | Starlark            | —        | ~95.4%   | —          | 249        | 🟢     | bazel-skylib / rules-python |
| —  | MDX                 | —        | ~87.9%   | —          | 1,808      | 🟡     | astro-starlight; React + Markdown polyglot needs JSX expression resolution |
| —  | CMake               | —        | ~80.2%   | —          | 1,775      | 🟡     | function calls into project-internal CMakeLists with include() chains |
| —  | Jinja               | —        | 65.45%   | 3,353      | 1,770      | 🔴     | template macros + Ansible variable namespace not resolved |

*Rates marked `~` are weighted averages from projects where the language is ≥30% of files (per-language resolved-edge counts aren't tracked separately in `baseline-all.json`).*

**93 language plugins total.** Not in the table above: ~40 templating engines (blade, ejs, erb, eex, heex, freemarker, gotemplate, gsp, haml, handlebars, hcl, jsp, liquid, mako, nunjucks, pug, shakespeare, slim, smarty, templ, thymeleaf, twig, velocity), schema/config languages (graphql, prisma, proto, dockerfile, systemd, crontab, make, nginx, puppet, hcl), markup (markdown, json, yaml, xml, toml, scss, css, html), and embedded sub-languages (angular, angular_template, julius). Most have either trivial unresolved counts (≤30) or no `internal_edges` graph at all (markup/config). They're indexed and parsed but don't move resolution-rate metrics meaningfully.
