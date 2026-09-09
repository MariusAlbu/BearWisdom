# Compiler-labelled real-project evaluation

1. [evidence] Enumerate every call expression in a declared compiler-program source cohort before running BearWisdom; never select sites by engine success.
2. [boundary] Begin with TypeScript/JavaScript compiler symbol-declaration navigation; record unsupported call/declaration forms and declaration sets explicitly.
3. [identity] Store numeric manifest file IDs and exact UTF-8 source coordinates; names and engine row IDs cannot supply ground truth.
4. [snapshot] Pin compiler version, configuration/read inputs and source hashes; reject changed inputs before and after engine evaluation.
5. [evidence] Retain compiler diagnostics and missing-target reasons; missing compiler symbols are not automatically negative labels.
6. [generic] Feed the manifest's complete source/contract supply through the production parser, Compilation and occurrence log using a disposable in-memory database.
7. [generic] Reuse the existing independent oracle evaluator and declaration-ID-to-source adapter; compare fresh and cold snapshots per site.
8. [report] Separate correct/wrong/unresolved/not-extracted labels, unlabelled compiler sites, extractor-only sites and compiler-program health.
9. [delivery] Provide a reproducible read-only project capture command and a scoped evaluation runner; generated artifacts never overwrite existing files implicitly.
10. [gate] This first real-project baseline is diagnostic/development evidence; held-out splits, broader languages/reference kinds and representative 99% gates remain explicit work.

## Implemented contract

`resolution_oracle/capture_typescript_project.mjs` uses pinned TypeScript 5.9.3 with the project's actual tsconfig and compiler source program. It enumerates all `CallExpression` sites under the selected prefix, independently of extraction or engine success. Aliases resolve through compiler symbols. A single supported declaration gives a source-coordinate target; missing symbols, overload/declaration sets and unsupported syntax remain explicit unlabelled observations, never negative labels.

`resolution_oracle::project::evaluate_manifest` verifies source/configuration hashes and UTF-8 coordinates, parses the compiler-supplied sources with production parsers, and runs fresh and DB-restored Compilation snapshots using an in-memory database. It writes neither the source project nor an existing index. The example runner creates a new report only. Declaration-kind-only disagreements are reported separately but remain strict mismatches. Both capture and evaluation reject changed recorded inputs.

## Reproduce the Query Core development cohort

Run from the BearWisdom root; choose new output filenames if these already exist:

```powershell
node crates/bearwisdom/src/resolution_oracle/capture_typescript_project.mjs F:/Work/Projects/AlphaT/node_modules/typescript/lib/typescript.js F:/Work/Projects/TestProjects/react-tanstack-query packages/query-core/tsconfig.prod.json packages/query-core/src resolution-documents/2026-09-07-query-core-compiler-manifest.json
cargo run --offline -p bearwisdom --example project_oracle -- resolution-documents/2026-09-07-query-core-compiler-manifest.json resolution-documents/2026-09-07-query-core-enclosing-id-report.json
```

The frozen manifest has 24 selected files, 187 supplied compiler files and zero compiler diagnostics. All 843 compiler calls were enumerated before engine evaluation: 690 labelled targets and 153 unlabelled sites (85 multiple declarations, 52 unsupported declaration forms, 15 unsupported call forms, one missing compiler symbol). All 690 labelled sites were extracted; four unlabelled compiler sites were not. There are three extractor-only sites and 832 duplicate extractor emissions, counted before occurrence deduplication.

The original `2026-09-07-query-core-oracle-report.json` remains unchanged. It recorded fresh 552 correct / 45 strict mismatches / 93 unresolved versus cold 243 / 40 / 407, with 314 per-site differences. Of the 45 fresh mismatches, 40 share the expected file/line/column but differ in declaration kind; five select child overrides for `super` calls. These are distinct failure categories, not 45 wrong navigation locations.

The enclosing-ID restoration report records 552 / 45 / 93 in both snapshots, zero per-site differences and unchanged fresh observations. It restores 309 correct cold targets and five previously wrong cold targets, without claiming those five are fixed. See `enclosing-snapshot-identity-design.md` for the generic cause and regression tests.

## Explicit limitations

- Development cohort only; not held-out or representative across languages. `gate_eligible` is always false.
- Call-symbol navigation is not overload dispatch, runtime implementation selection or complete reference coverage.
- Recorded read-input hashes are not a complete filesystem resolution fingerprint: absent-file probes, directory inventories and changed compiler root selection are not fully captured.
- Compiler source supply/configuration is recorded, but complete engine configuration parity, package discovery and demand-materialization coverage are not established.
- Absolute physical paths make this captured manifest machine-specific; relocation needs explicit re-capture or a reviewed path-mapping contract.
- Debug runtime is diagnostic timing, not a release-performance or AI-efficiency benchmark.

## Configured-program comparison contract

1. [evidence] Preserve the existing manifest and legacy evaluation mode; never relabel a target to accommodate the new binder.
2. [boundary] Version-two compiler captures record each supplied source's configured module isolation, including module detection without import/export syntax.
3. [boundary] This compiler-derived configuration is oracle input only, not runtime project discovery or a compiler-backed production resolver.
4. [generic] Add an explicit evaluation mode that converts the pinned source membership, hashes and scope evidence into ProjectContext program descriptors.
5. [generic] Reject configured evaluation of older manifests lacking source-scope evidence instead of guessing from file extensions or names.
6. [snapshot] Fingerprint the complete pinned oracle input and reconstruct the same configured program from persisted recipes on cold load.
7. [evidence] Keep unsupported supplied languages visible and make the program incomplete rather than silently dropping providers.
8. [report] Record the selected binding mode and retain every strict occurrence verdict, unsupported reason and snapshot difference.
9. [verification] Test forced module detection and declaration-file globals in the compiler adapter; test input validation and actual fresh/cold global cascades in Rust.
10. [measurement] Capture a new real-project manifest, verify its call labels equal the old population, and report configured regressions honestly before extending semantics.

### Configured-program evidence (2026-09-07)

The capture now emits version two with `source_scope` for every compiler source; `ts.isExternalModule` observes actual configured module detection. The forced-module test checks a source without imports/exports alongside global and exported declaration files. Version-one manifests remain valid for legacy evaluation, but configured evaluation rejects them rather than guessing missing scope evidence. The production resolver does not call TypeScript.

`project_oracle <manifest> <new-report> --configured-program` selects the explicit program; omitting the flag preserves legacy evaluation. Reports identify the binding mode and list source-ingestion barriers separately from call verdicts. Unknown or unparsed source providers keep the environment incomplete. Reports still never qualify as a held-out 99% gate.

New manifest `2026-09-07-query-core-configured-compiler-manifest.json` has SHA256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`. Deep comparison confirms its complete calls, targets, diagnostics, compiler options, recorded inputs and original source metadata are identical to the version-one manifest. Only version/scope evidence changed: 119 syntax-scoped and 68 module-isolated sources, 843 calls and 690 labelled targets.

The first configured report abstains on all 690 labels, fresh and cold. This contradicts any claim that passing the authored ambient fixtures establishes real-program readiness. The diagnostic report identifies one missing global capture (`lib.dom.d.ts`, 1,874,901 bytes) and 55 incomplete captures, all in syntax-scoped providers. The latter include `lib.es2015.symbol.wellknown.d.ts` and Node declaration files. They require source-shape diagnosis and proper ambient-module/augmentation support, not namesake fallback or removing providers from the manifest.

After separating large-source identity capture from flow-query budgets, `2026-09-07-query-core-large-source-program-report.json` has zero missing captures and 55 incomplete captures. Every configured call verdict remains unchanged: zero correct, zero wrong, 690 unresolved. Full fresh/cold reports are equal to the pre-fix configured diagnostic report. This repairs an ingestion prerequisite; it does not raise real-project recall while the other program barriers remain.

### Provider grammar repair (2026-09-07)

The pinned local TypeScript/TSX grammar now parses all 187 supplied source files
without syntax errors. The new `2026-09-07-query-core-provider-grammar-program-report.json`
has 54 incomplete captures (ambient external module providers), no missing
captures, and the same 690 unresolved labels. The well-known-symbol library's
`keyof readonly` parser failure is repaired; parsing Node global augmentations
and generic import types does not yet supply their semantic module identities.

The paired `2026-09-07-query-core-provider-grammar-legacy-report.json` remains
557 correct / 40 kind-only disagreements / 93 unresolved. All per-occurrence
expected/actual verdicts and counts match the respective earlier reports, with
zero fresh/cold differences. Its manifest revision changes because the new
legacy run also uses the version-two manifest. See
`typescript-provider-grammar-design.md` for pins, exact hashes and verification.

### Source-local module identity (2026-09-07)

The new `2026-09-07-query-core-module-scope-program-report.json` and paired legacy
report preserve the same pinned population and per-occurrence verdicts after
scoped module/import/export capture and persistence. Configured evaluation still
has 54 incomplete global providers and 690 unresolved labels; legacy remains
557 correct / 40 kind-only disagreements / 93 unresolved. Fresh and cold agree.
Program-owned provider selection is not implemented by source-local unit capture.
See `ambient-module-scope-design.md` for evidence, hashes and remaining integration.

### Configured ambient provider binding (2026-09-07)

`2026-09-07-query-core-program-ambient-binding-program-report.json` reduces the
incomplete global-provider list from 54 to 14 after program-owned provider graphs
and nested global contribution ownership are connected. All 187 supplied files
and the original 843 calls / 690 labels remain. Configured results still abstain
on all 690 labels; the remaining module-capture barriers have not been bypassed.
The paired legacy report remains 557 correct / 40 kind-only disagreements /
93 unresolved, with zero fresh/cold differences. Deep report comparison confirms
only elapsed time and the configured source-gap list changed.

A source-owned diagnostic identifies 28 incomplete literal units in the remaining
14 files, with valid containment and complete file-root module capture. Import-
equals and export-assignment representation is the next prerequisite, followed
by actual stdlib merging and richer source-bound type recipes. Seven new authored
compiler-validated ambient cases pass all 16 labels but do not establish a
real-project correctness gate. See `program-ambient-module-binding-design.md`.

## Export-assignment entities — 2026-09-07

The configured source-capture gap is now zero: all 187 pinned supplied files retain complete source-module evidence, including the last 28 literal units in 14 Node providers. This is representation evidence, not full declaration-merge or type-system coverage.

The new `2026-09-07-query-core-export-entity-program-report.json` measures 354 correct / 40 declaration-kind-only disagreements / 296 unresolved of the unchanged 690 compiler-labelled calls: 51.3043% recall, 89.8477% strict precision, zero fresh/cold changes and no passing 99% gate. This replaces the previous configured all-unresolved result; it is not an improvement over the separate legacy pipeline's recall.

The paired `2026-09-07-query-core-export-entity-legacy-report.json` is structurally identical to the previous legacy report except elapsed time: 557 correct / 40 kind-only / 93 unresolved. Neither old reports nor the compiler manifest were modified. Both evaluators verified all source/configuration hashes.

The trace example now honors the baseline's binding mode. Its configured diagnostic requests all 296 unresolved occurrences, retains 1,126 raw trace records and exactly reproduces the ordinary configured report apart from elapsed time. The additional source-provenance diagnostic confirms incomplete Array/ReadonlyArray/Set/Map/Promise/PromiseConstructor type groups, a captured FocusManager base missing from configured TypeInfo, and absent initializer-derived field typing for Subscribable.listeners. These are the next semantic causes to address. Full implementation, tests, hashes and limitations are in `export-entity-binding-design.md`.
