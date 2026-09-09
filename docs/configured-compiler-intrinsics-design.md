# Configured compiler intrinsic aliases

1. [profile data] Describe the intrinsic keyword, alias-name field, parameter field and supported zero-parameter alias roles in the TypeScript syntax profile.
2. [generic] Capture a role only from a complete, valid alias declaration whose direct body is that keyword; retain its physical source slot.
3. [generic] Keyword aliases keep an unknown ordinary recipe, so unsupported roles and malformed declarations cannot fall back to a namesake.
4. [generic] Persist the role alongside its declaration row in configured type inputs; do not serialize workspace TypeIds.
5. [profile data] Decode effective strictBuiltinIteratorReturn, including strict inheritance and explicit overrides, at the compiler-options ingestion boundary.
6. [generic] Carry optional intrinsic policy in each configured Program; absent or malformed evidence stays unknown and configuration changes rebuild the view.
7. [generic] Materialize the selected alias row's body using its captured role and the selected program's policy, preserving all ordinary alias identities.
8. [generic] Reuse ordinary alias expansion for consumers, generic defaults and iterator heritage; no name/path dispatch enters runtime lookup.
9. [generic] Verify pinned compiler diagnostics and types, script/module/namespace isolation, shifted portable arenas, cold reload, policy/provider changes, edits and deletion.
10. [generic] Bump durable binding/extraction versions, rerun targeted regressions and trace the retained real constructor before closing any broader roadmap parent.

## Evidence and remaining work

TypeScript 5.9.3's `getBuiltinIteratorReturnType` chooses undefined in strict mode and any otherwise. Its `checkTypeAliasDeclaration` admits this intrinsic by declaration name and arity, including namespace-local declarations; an ordinary same-named alias has ordinary semantics. This agrees with the [TypeScript option documentation](https://www.typescriptlang.org/tsconfig/strictBuiltinIteratorReturn.html). The independent verifier pins the local compiler to 5.9.3 and uses its diagnostics and source declaration locations.

Implemented at binding epoch 66 / extractor schema 80. The generic ingestion pass also requires a unique type BindingId slot before supplying an alias body. A compiler-negative duplicate module alias initially exposed an intrinsic body through its physical row; both duplicate bodies now remain unknown. Keyword roles and compiler option spellings occur only in ingestion/profile data. Selected-view evaluation uses declaration IDs and typed policy.

The 20 compiler fixtures run in script and module scopes: 52 exact alias-type labels, eight exact method declaration/signature checks, and five diagnostic-negative cases. They cover strict inheritance/overrides, independence from strictNullChecks, ordinary and generic namesakes, namespace-local intrinsics, keyword shadowing, parenthesized ordinary references, wrong name/arity, duplicate aliases and unsupported string intrinsics.

Engine verification covers shifted portable arenas, poisoned symbol/signature/call text, fresh and cold snapshots, configuration-only strict/non-strict/missing policy changes, provider retargeting, foreign nominal contexts, source edits, stale source hashes and deletion. The constructor-field-to-read-to-touch cascade reaches both compiler-labelled source methods; removing policy revokes the chain. The existing program serde test now verifies old inputs do not invent policy. The selected library regression suite passed 2,138 tests (32 ignored).

At the intrinsic-policy checkpoint, string-transform intrinsics and conditional aliases used as member receivers remained unsupported. The latter limitation retained four exact compiler call labels alongside four supported constructor-cascade checks. The subsequent [conditional receiver implementation](configured-conditional-receivers-design.md) now passes all eight exact call checks; string-transform intrinsics remain open.

The frozen Query Core provenance probe confirms selected stdlib row 14169 (`BuiltinIteratorReturn`) has body `Intrinsic(Undefined)`. Node namespace row 17131 keeps its distinct conditional body. Both IteratorObject providers remain admitted. ArrayIterator row 14170 is still rejected, the readonly-array constructor remains applicable as Set<caller T>, and the iterable candidate/full initializer remains unknown. Recursive/covariant method-return compatibility in `program_interface_heritage.rs` is the next dependency; exact selected constructor order, complete iterable inference and callback negation remain open.

All seven targeted integrations passed: resolution corpora for TypeScript, JavaScript and Rust, plus per-file manifest and per-package context checks. Native Windows file-budget/string-identity audits passed across the dirty worktree (339 production files / 131 scoped resolver and type files); git diff whitespace checks passed.

The public Program field is serde-defaulted. Neither AlphaT's source tree nor Lynx's impact source tree directly constructs Program or CallablePolicy. Required sibling compile attempts used offline/locked mode and workspace-local target directories: AlphaT stops in dependency resolution at yanked der 0.8.0 through ureq/ort; Lynx requires a lockfile update. Neither reaches source compilation, and no sibling files were changed.

## Frozen cohort results

The original manifest remains unchanged: `resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json`, SHA-256 `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c` (187 supplied sources, 24 selected files, 843 compiler calls, 690 labelled targets, zero compiler diagnostics).

- Configured report: `resolution-documents/2026-09-09-query-core-compiler-intrinsics-program-report.json`, SHA-256 `63e3458f69571dec01edb876c75f6f9533c7e3fa74df7646ba9908264e3c6e12`, elapsed 53,813 ms. Fresh/cold: 575 correct, 40 kind-only incorrect, 75 unresolved, 83.3333% recall and 93.4959% strict precision, zero snapshot changes, gate ineligible.
- Legacy report: `resolution-documents/2026-09-09-query-core-compiler-intrinsics-legacy-report.json`, SHA-256 `fd7ca1583483e99338329ae803760c4b9528f7e36248bbe9dc5e12e21b272e63`, elapsed 55,057 ms. Fresh/cold: 557 correct, 40 kind-only incorrect, 93 unresolved, zero snapshot changes, gate ineligible.

Deep equality excluding only elapsed_ms confirms every report field and retained occurrence is unchanged from the September 9 generic-augmentation reports. This is verified infrastructure progress, not a real-cohort recall increase. Existing roadmap lines/order were preserved; only completed intrinsic work and the newly isolated conditional receiver limitation were appended. The full roadmap remains open.
