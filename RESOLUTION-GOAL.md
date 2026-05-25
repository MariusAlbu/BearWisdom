# Resolution goal — CLOSED 2026-05-22, per-language widening 2026-05-23+

> **Architecture:** `heuristic.rs` deleted, resolver is single-tier engine-only at `confidence = 1.0`. Closed 2026-05-22 at corpus 95.55%.
>
> **Per-language widening track** (opened 2026-05-23): one item from `baseline-gaps.md` per session — fix, recapture affected projects, refresh `baseline-all.json` + `baseline-gaps.{json,md}` + `baseline-by-so2025.md` + this doc.

## Per-fix tracking

| Fix | Date | Language | Before | After | Δ |
|---|---|---|---:|---:|---:|
| #1 Rust prelude in resolver | 2026-05-23 | rust | 90.36% | 94.36% | +4.00pp |
| #2 Attribute code-fence refs to `origin_language` | 2026-05-23 | markdown | 71.38% | **81.60%** | +10.22pp |
|  |  | jupyter | 56.05% | **77.36%** | +21.31pp |
|  |  | rmarkdown | 65.65% | 65.14% | −0.51pp |
| #3 A–H batch (see below) | 2026-05-23 | rust | 94.36% | 95.33% | +0.97pp |
| #4 Phase 1 — candidate ranking in DefaultResolver | 2026-05-24 | rust | 95.33% | 95.82% | +0.49pp |
|  |  | corpus | 95.51% | 95.55% | +0.04pp |
| #5 Phases 2–5 — generic resolver + walker abstractions | 2026-05-24 | corpus | 95.55% | **95.64%** | +0.09pp |
|  |  | typescript | 96.05% | **96.38%** | +0.33pp |
|  |  | java | 95.81% | **95.93%** | +0.12pp |
|  |  | python | 94.82% | **95.05%** | +0.23pp ✅ crosses 95% |
|  |  | scala | 94.96% | **95.01%** | +0.05pp ✅ crosses 95% |
|  |  | go | 92.26% | **92.87%** | +0.61pp |
|  |  | kotlin | 96.21% | **96.36%** | +0.15pp |
|  |  | nim | 93.51% | **93.75%** | +0.24pp |
|  |  | javascript | 92.35% | **92.55%** | +0.20pp |

**Cumulative rust: 90.36% → 95.82% (+5.46pp)** — past the 95% target with margin.

### Fix #5 — Phases 2–5: generic resolver + walker abstractions

Five abstraction tracks, all engine-side, applied in one batch:

**Phase 2 (resolver enhancements)**
- **2a Transitive re-export closure** — `SymbolIndex::build` now runs the alias-synthesis pass as a fixed-point loop (cap 5 hops). One-hop `pub use X as Y` cases were already handled by fix #3-C; multi-hop named-alias chains now land. Generic across rust, ts, python, js.
- **2b Inheritance-walk fallback** — new `find_member_via_inheritance` helper in `engine/chain_walker.rs`. Wired into `walk_rust_lang_chain` Phase 3. When `members_of(T)` doesn't yield the segment, climb `parent_class_qname(T)` up to 8 levels and retry. Generic helper any per-language chain walker can adopt.
- **2c Static wildcard import follow-through** — new `resolve_via_wildcard_import` strategy in `DefaultResolver`. When the caller's file has wildcard imports in scope, resolve bare names against the wildcard's namespace via the `qname_directly_under` predicate. Closes Java `import static X.*;`, Rust `use foo::*`, Python `from X import *`, C++ `using namespace std;` patterns.
- **2d Bare-name chain-miss recording in DefaultResolver** — generalised the rust-specific recording. When every strategy in `resolve_all` fails on a non-trivial bare-name ref, record an empty-current-type chain miss so the externals stage's `locate_via_symbol_index` gets a chance to demand-pull the file. Every language now triggers demand-pull on unresolved bare names, not just Rust.

**Phase 3 (generated-code adapters)**
- New `Ecosystem` impls for build-time generators, each activated by its config file (no hardcoded symbol tables — the generated source itself is the symbol surface):
  - `PrismaClientEcosystem` — reads `schema.prisma`'s `generator client { output = "..." }` block, walks `node_modules/.prisma/client/` as TS externals. Closes `PrismaClient` / `findUnique` / `findMany`.
  - `ProtocGeneratedEcosystem` — when any `*.proto` is present, probes conventional output dirs across languages (rust target, gradle build/generated, go internal/genproto, etc.). Walks them with the per-file language extractor.
  - `OpenApiGeneratedEcosystem` — when `openapi.{yaml,json}` / `swagger.{yaml,json}` exists, probes generated/ / src/generated / openapi-generated / build/generated/openapi.

**Phase 4 (archive walking)**
- New `jar_walker` module with an in-tree minimal `.class` bytecode parser (constant pool + access flags + this_class + super_class + fields + methods). No external dep — straight zip + parser.
- `MavenClassesEcosystem` (transitive on Maven) parses every `.jar` in `~/.m2/repository`, `~/.gradle/caches/modules-2/files-2.1`, project-local `lib/` / `libs/` / `vendor/` — bounded at 300 jars per project and 32 MB per jar — emitting public/protected types + members as `ParsedFile` entries via `parse_metadata_only`. Closes transitive .class-only deps that don't have `-sources.jar` downloaded.

**Phase 5 (missing manifest readers)**
- New manifest readers + `ManifestKind` variants:
  - **Vcpkg** — `vcpkg.json` `dependencies` array (string or object form). Closes C/C++ `find_package`-style deps.
  - **PipRequirements** — `requirements.txt` / `requirements-*.txt` / `requirements/*.txt`. Parses pip specs including extras, environment markers, editable installs (`-e ...#egg=name`), and PEP 503-normalises names.
  - **Rebar** — `rebar.config` / `rebar3.config` Erlang term parser for `{deps, [...]}` block. Handles `{name, version}`, `{name, {git, ...}}`, and bare atom forms.

### Fix #4 — Phase 1 candidate ranking

Pure engine change in `DefaultResolver::resolve_via_ranked_candidates`. When
`by_name(target)` returns ≥2 kind-compatible candidates and the strict
single-candidate strategies above all bail, rank by deterministic signals
already on the lookup / file_ctx / ref_ctx:

- Same workspace package as caller (+1000)
- Caller imports the candidate's package (+500)
- Caller's import module-path matches candidate's qname prefix (+300)
- Ambient path (+200)
- Shared directory prefix with caller (+10 per segment)
- Public visibility (+50), private (−200), neutral otherwise
- Small depth penalty for deeply-nested externals

The top candidate must beat the runner-up by `RANK_MARGIN = 100`. Below the
margin → return None, stay honestly unresolved. No tables, no per-language
plumbing, no hardcoded names.

Corpus impact (+0.04pp) was smaller than projected because most cross-language
bucket-A targets I'd estimated (bicep `description`, JUnit `assertTrue`)
turned out to be bucket D (no external symbol in index at all) — they need
manifest/walker work, not resolver ranking. The strategy correctly fired
where it had signal (Rust externals across stdlib/cargo/workspace boundaries)
and correctly stayed out where candidates tied.

### Fix #3 batch — A through H

- **A** `EdgeKind::Implements` + `Inherits` added to rust prelude resolver arm — `impl Default for Foo` now resolves Default; 600 unresolved cleared.
- **B** `engine_generic_param` strategy in `DefaultResolver` — bare TypeRef matching an enclosing scope's declared generic parameter resolves to that scope. Cross-language (rust/ts/java/kotlin/c#/scala/cpp all benefit).
- **C** Re-export alias synthesis in `SymbolIndex::build` — `pub use X as Y` registers a virtual `Y` entry pointing at `X`'s file. Closes `TantivyDocument` / `CompactDoc`-style cases. Generic across rust/ts/python/etc.
- **D** Rust `impl X` block extraction — synthetic Namespace marker now uses `<impl X@L>` instead of `X`, so it no longer collides with the implementing type in `by_name` / `types_by_name`.
- **E** Cargo manifest parsing — extended to recognise `[target.'cfg(*)'.dependencies]`, sub-table form `[dependencies.foo]`, plus `workspace.dev-dependencies` / `workspace.build-dependencies`; added `test` and `proc_macro` to `STDLIB_CRATES`.
- **F** `demand_pre_pull` for `rust-stdlib` ecosystem — eagerly walks `macros.rs` / `macros/mod.rs` / `fmt/macros.rs` per stdlib crate so `vec!` / prelude macros land in the index without waiting for chain-miss demand.
- **G** tree-sitter `const trait` workaround — rewrites `const trait` → `      trait` and `[const]` → `       ` in the source before parsing (byte-offset preserving). Recovers `PartialEq` / `Eq` / `PartialOrd` / `Ord` / `AsRef` / `AsMut` from `core/src/cmp.rs` + `convert.rs` that tree-sitter-rust 0.24 dropped.
- **H** Variable type-inference for builder-pattern receivers — `let x = obj.method()` now emits a chain-bearing TypeRef so build-time chain inference can hop receiver → return type and bind `x`'s field_type.

Verification: 5,839 / 5,839 lib tests green.

## Current corpus state (post phases 2–5)

| Metric | Value |
|---|---:|
| Corpus rate | **95.64%** |
| Engine edges | 16,045,040 |
| Heuristic edges | **0** |
| Honestly unresolved | 730,803 |
| Languages with rate ≥ 95% (target met) | **26** (python + scala cross in) |
| Languages at ≥ 90% but < 95% | 9 |
| Languages < 90% | 13 |
| Projects re-indexed | 250 / 257 (7 ghost — source missing) |

### Per-language rates (post-closeout)

Languages meeting the ≥95% target:

| Language | Rate | Engine edges |
|---|---:|---:|
| C# | 100.00% | 3,254,784 |
| Ruby | 99.86% | 156,179 |
| PHP | 98.40% | 375,787 |
| C | 97.96% | 1,887,895 |
| TypeScript | 96.74% | 2,283,818 |
| F# | 96.74% | 121,445 |
| Dart | 96.91% | 498,111 |
| Kotlin | 96.36% | 403,300 |
| Pascal | 96.00% | 431,328 |
| Swift | 96.07% | 112,088 |
| Haskell | 95.95% | 125,357 |
| Erlang | 95.92% | 256,883 |
| Java | 95.87% | 672,324 |
| R | 95.50% | 94,149 |
| JavaScript | 95.33% | 710,985 |
| **Rust** | **95.82%** | **686,473** (post fix #4 — was 90.36% pre-fix-#1) |

Languages at 90-95% (close to target, one focused Track 3 widening session each):

| Language | Rate | Notes |
|---|---:|---|
| Scala | 94.96% | heuristic deletion left ~1900 edges unresolved; engine path can absorb |
| Elixir | 94.61% | |
| C++ | 94.47% | |
| Python | 94.86% | dunder gap remains |
| Zig | 94.73% | comptime extractor work |
| Fortran | 94.18% | module-procedure-chain extractor |
| Ada | 93.74% | |
| Vue | 93.68% | i18n / store walkers would close remainder |
| Nim | 93.50% | |
| Nix | 92.64% | |
| Go | 92.43% | |

Languages < 90% needing extractor / walker work (future sessions):

| Language | Rate | Reason |
|---|---:|---|
| Odin | 79.51% | engine path under-developed |
| Lua | 87.01% | KOReader externals walker would close ~60% of gap |
| OCaml | 88.91% | |
| Groovy | 81.27% | Gradle DSL externals + bare_name deletion impact |
| Robot | 84.70% | |
| Clojure | 87.48% | |
| Prolog | 74.07% | predicate dispatch engine path |
| Markdown | 81.60% | post-fix-#2 — remaining 8,790 unresolved are link refs that don't match any indexed file |
| Jupyter | 77.36% | post-fix-#2 — remaining `r`/`python` cell refs that the host-language extractor couldn't resolve |
| R (incl. jupyter cells) | **75.61%** | attribution shift from fix #2 absorbed tidyverse calls (`ggplot`/`aes`/`mutate`) from jupyter R cells; .R files alone still resolve >95%. Real R rate requires a tidyverse synthetics walker |

Languages at borderline (≥95% genuinely unreachable, documented):

| Language | Rate | Borderline reason |
|---|---:|---|
| MATLAB | 61.62% | walker install-gated (no MathWorks license on dev box) — ≥95% post-install |
| Jupyter | 56.05% | string-eval (`exec`, `run_cell`) statically untraceable; widening helps but ceiling exists |
| Bicep | 42.96% | ARM-template binding model unique; needs dedicated extractor session |
| Svelte | 22.09% | SvelteKit project structure broke; needs `.svelte-kit/` walker (same pattern as Nuxt) |
| MDX | 57.31% | embedded-component resolution needs JSX-in-MD walker |
| VB.NET | 56.28% | hook fired but extractor emits operator keywords (NameOf, CType) as Calls — extractor work |

## Architecture milestones — all done

- **M1 DefaultResolver tower** ✅ — 13 deterministic strategies in `type_checker/core/default_resolver.rs`, 27 unit tests
- **M1.A Imports early-return removed in 49 hooks** ✅
- **M1.B DefaultResolver fallback in 10 custom resolvers** (Java, Rust, Python, Kotlin, Go, PHP, Ruby, Dart, Swift, TypeScript, Vue, Scala, C#) ✅
- **M1.C `*_bare_name` strategies deleted in 16 hooks** ✅
- **M2 `heuristic_name_kind` deleted** ✅
- **M3 Qt Linguist `.ts` content-sniff** ✅
- **M4 Nuxt auto-imports walker + Vue runtime ambient extension** ✅
- **Track 2.1 VBNet hooks** ✅
- **Track 2.2 Jupyter R-kernel detection** ✅
- **Track 2.3 Shell diagnosis** ✅
- **Track 3.14 Templ + Nunjucks hooks** ✅
- **Track 3.7 Vue (→ Nuxt walker)** ✅
- **Track 3.2 C# heuristic_qualified_name absorption** ✅
- **Track 4.1 final corpus recapture** ✅ (2026-05-22, 250 re-indexed, 7 ghost)
- **Track 4.2 `heuristic.rs` deleted** ✅
- **Track 4.3 goal-doc CLOSED** ✅

## What was *not* completed (future sessions, not blocking architecture)

Per-language widening for languages still below 95% needs focused per-language extractor + resolver work:

- **M4 walkers requiring real installs** (no Linux/WSL test target on this dev box): bash-completion, Pascal OpenSSL/ICU/Castle, KOReader (Lua), AssertJ/kotest Maven sources jars
- **M5 macro synth** (substantial language-specific AST work): Pascal Delphi `[DllImport]`/`[JSImport]` attribute synth, Rust `#[derive(...)]` synth, Haskell Template Haskell synth, Vue `defineProps`/`defineEmits` extractor enhancement
- **Track 3 deep widening** per low-engine-share language: Rust chain-miss interaction, Fortran module-procedure-chain extractor, Zig usingnamespace/comptime, Pascal RTTI, OCaml functor-scoped opens, Bicep ARM-template binding model, Svelte SvelteKit ambient walker (mirror of Nuxt), MDX JSX-in-MD walker
- **Extractor-level fixes**: VBNet operator extraction (NameOf/CType as Calls is wrong)

The architecture is structurally complete and ships engine-only deterministic resolution at `confidence = 1.0`. Further work is incremental per-language improvement — each can run in its own focused session against the new baseline without affecting the architecture.
