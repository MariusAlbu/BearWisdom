# Configured ambient identity — diagnosis and implementation contract

## Evidence, not a resolution fix

The pinned Query Core oracle remains 557 correct / 40 kind-only disagreements / 93 unresolved, with identical fresh/cold results. `project_trace` preserves its 690 compiler labels and selects the 93 baseline unresolved sites. The first two trace artifacts each contain 362 diagnostic records and zero changed oracle observations.

`queryCache.ts` shows `getAll()` yielding a typed array and `forEach` resolving. Its callback signature is captured, and the callback parameter has a valid source span. Yet the receiver environment binds `GenericParamId(576)` while the callback parameter carries `GenericParamId(814)` in the captured run. The same mismatch appears at 18 traced callback call sites. IDs are run-local; these numbers describe the retained artifact, not a stable symbol address.

```ts
// lib.es2015.core.d.ts and lib.es5.d.ts contribute to one compiler global Array.
interface Array<T> { /* one physical declaration */ }
interface Array<T> { forEach(callback: (value: T) => void): void; }
queries.forEach(query => query.onFocus());
// ❌ [generic] An unbound Array head can select one physical owner, then the
// legacy member path supplies another owner's method. The two T IDs differ.
// ✅ [generic] lambda_seed declines the still-open callback parameter.

interface Promise<T> { /* instance surface */ }
interface PromiseConstructor { reject<T>(reason?: unknown): Promise<T>; }
declare var Promise: PromiseConstructor;
Promise.reject(error);
// ❌ [generic] The root trace selects the instance interface, not the value's
// declared constructor type. The correct declarations are already supplied.
```

These are not permission to map generic parameters by spelling or merge every same-named indexed type. `TypeForm::Array` currently creates a legacy string head; `head_decl::head_symbol_id` recovers a declaration by name; `lookup_member_on_bounded` permits a name-shaped receiver to retry legacy member lookup. `module_input::scoped_declarations` correctly fences separate lexical declarations, but does not yet represent shared configured global scopes. Fixing only callback substitution would preserve the underlying identity error.

## Implementation contract (next work)

1. [generic] Represent a configured compilation/program identity separately from a workspace and a physical file; sharing an index is not evidence of sharing globals.
2. [boundary] Capture source membership and configuration fingerprints from ecosystem inputs; compiler use remains independent oracle validation, not the runtime resolver.
3. [profile data] Attest script/global declarations, external-module isolation and explicit global augmentations from syntax; never infer ambient status from a `.d.ts` suffix alone.
4. [generic] Intern global export names once into program-owned type/value namespace bindings; preserve local/import shadowing and unresolved or ambiguous providers.
5. [generic] Give legally merged declarations one logical owner identity while retaining every physical member/declaration as a navigation target.
6. [generic] Canonicalize generic parameter IDs by attested declaration-group correspondence before materializing signatures; incompatible arity/kinds/constraints/defaults must not be zipped speculatively.
7. [profile data] Bind language-provided type constructors such as array syntax to that program's attested declaration; no resolver builtin lists or format-and-re-resolve bridge.
8. [generic] Bind ambient value reads to their value declaration/type IDs, keeping instance and constructor surfaces distinct; calls and contextual writes consume only those IDs.
9. [snapshot] Persist configured source instances, merge evidence and dependency fingerprints; edits, membership changes, deletion and cold reload must reconstruct the same semantic snapshot without cross-program leakage.
10. [verification] Close independently labelled split-interface and value/type cascades; retain same-file controls, module-export negatives, overlapping-program isolation, provider edits/deletion and poisoned-display tests before measuring the real cohort again.

## Scope cautions

- One physical source may participate in multiple programs with different global augmentations. A workspace-wide `physical row -> canonical row` map alone cannot model that distinction.
- The 18 traced callback sites are upstream evidence, not 18 guaranteed repaired labelled calls. Some downstream expressions also need callback-return inference or other type facts.
- Promise/static calls are a separate proven value-binding failure; their broader relationship to missing configured global identity is an architectural inference until the new path closes the cascade.
- The diagnostic trace collector uses legacy line filters, can include adjacent references, and does not phase-tag fresh/cold records. Exact correctness remains the source-addressed oracle's responsibility.

## Reproduction results

`ambient_fixtures.json` contains five independent cases, verified by TypeScript 5.9.3 for all 15 source call labels. Four legal cases have zero compiler diagnostics. The negative control has the expected missing-property diagnostic 2339 and contextual implicit-parameter diagnostic 7006. These diagnostic checks are optional explicit fixture data in `verify_modules.mjs`; older cohorts retain their existing contract.

The retained `ambient_observed_baseline.json` records 12/14 correct positives, two unresolved positives and one false-positive negative; fresh and cold match exactly. The regression test permits a changed historical observation only when it becomes compiler-correct. It is a baseline gate, not a claim that these failures are solved.

```ts
// ✅ [generic] Same-file merged Catalog<T>: first().touch() and callback touch().
// ❌ [generic] Split-file Catalog<T>: callback item.touch() loses owner T identity.
// ✅ [generic] Single-file Deferred value/type surfaces: reject().then().
// ⚠️ [generic] Split-file Deferred: reject() works, then() loses the other declaration's member.
// ❌ [generic] Unrelated exported Catalog.forEach leaks into the global Catalog receiver.
```

The final real-project owner trace confirms receiver owner 14025 is `ext:oracle:6/lib.es2015.core.d.ts`, while selected `forEach` owner 15495 is `ext:oracle:59/lib.es5.d.ts`. This establishes the physical-owner mismatch directly, not just from generic ID numbers.

Matching baseline unresolved source sites to same-file/same-selector-line diagnostic records confirms 18 labelled calls die at the same Promise root owner 14222. Their offsets are: hydration 2511/2880/8625; infiniteQueryBehavior 1481; mutation 4480/7142/7369/7738/7988; mutationObserver 4939/5210/5507/5784; queryClient 8183/8194/9577; retryer 4127; utils 11521. Some chained forms can require additional inference after the root is repaired. The separate 18 callback call sites with the cross-owner T mismatch are upstream observations, not an additive disjoint count of unresolved labels.

Next implementation must establish configured global identity and access before attempting to equate the two generic IDs. The exported-module false positive is a mandatory non-interference gate for that work.

## First implementation layer

The first layer wires explicit `ProjectContext` program descriptors into the existing module-input capture, rebuild and persistence lifecycle. Source syntax records root declarations separately from explicit global augmentations. A frozen program graph owns snapshot-local ProgramId, SourceInstanceId, global-name and binding-group IDs; runtime queries accept those IDs only. It retains physical declaration rows instead of rewriting workspace-wide canonical owners. Tests must prove overlapping memberships, module isolation, type/value separation, incomplete providers, stale handles, config replacement and cold reconstruction. FileLookup selection, program-specific TypeInfo materialization and ecosystem configuration discovery remain necessary follow-on integration; this layer alone is not a repaired call-resolution claim.

Implemented in `indexer/programs.rs`, `lexical_globals.rs`, and engine `program_input.rs` / `program_graph.rs`, integrated through `ProjectContext`, `ModuleInput` and `ModuleGraph`. Binding epoch is 30; extraction schema is 50. No runtime compiler dependency was introduced.

```ts
// ✅ [profile data] Empty export {} and side-effect imports attest module isolation.
// ✅ [generic] Shared physical Catalog<T> has distinct logical groups in programs A/B.
// ✅ [generic] Generic declaration-name correspondence is interned once; group checks use IDs.
// ✅ [generic] Deferred's type and value declarations occupy separate global binding IDs.
// ✅ [generic] Missing/stale providers, stale handles and conflicting configuration abstain.
// ✅ [generic] Declaring a provider as a module removes its global contribution after reload.
// ⚠️ [generic] Overlapping members, index/call signatures, inherited merge surfaces and
// constraints/defaults require further compatibility evidence; they remain incomplete.
// ⚠️ [profile data] Unsupported nested augmentations fence environment completeness.
// ❌ [generic] FileLookup has not yet selected these program environments or materialized
// program-owned TypeInfos; the original callback/value-root failures remain open.
```

Source declaration correlation reuses the lexical ingestion convention: whole-node coordinates for named declarations, identifier coordinates for variable patterns. It never uses display names to locate a physical row. Source correlation builds its coordinate index once, and group compatibility work scales with declaration count times distinct kinds rather than all declaration pairs.

`ProjectContext.programs = None` leaves configuration unavailable and allows a cold load to restore persisted configuration. `Some([])` explicitly clears it; a fresh nonempty configuration wins over stored input. Callers must supply actual source membership, content hashes, configuration fingerprints and module-detection evidence. Merely indexing a dependency or sharing a `.d.ts` suffix is not sufficient. No ecosystem currently populates this new seam automatically.

The new Query Core report `resolution-documents/2026-09-07-query-core-program-input-report.json` retains the baseline's complete 690 fresh/cold occurrence objects and counts (557/40/93, zero snapshot differences). Independent ambient compiler verification still validates all 15 labels. This establishes non-regression, not improved recall or completed ambient binding. Consumer checks were attempted with locked/offline dependency resolution: AlphaT stops on yanked `der 0.8.0`, Lynx requires a lockfile update; neither reached compilation and neither lockfile was changed.

Final verification after the nested-augmentation completeness guard: 1,103 selected engine/lexical/cache/oracle tests passed, nine ignored; 19 new tests cover this layer. `2026-09-07-query-core-program-input-final-report.json` again matches every baseline fresh/cold occurrence and count by deep object equality. Native file-budget audit checked 270 changed production files with no violations; the ID audit checked 87 resolver/type-checker files with no added forbidden string-identity patterns. These are verification scopes, not a full workspace or downstream-consumer compilation claim.

## Program-specific nominal type integration

1. [generic] Keep physical declaration rows as navigation addresses; add a separate opaque nominal-context ID to bound nominal types.
2. [generic] Intern bound nominals by `(context ID, declaration ID)` only; display spelling must not participate in type lookup or interning equality.
3. [generic] Route direct `intern(Type::Decl)` through the same numeric index as `decl`, including concurrent calls and snapshot reconstruction.
4. [snapshot] Rebuild every nominal index on arena restore; remint serialized context handles consistently within each load so old/foreign handles cannot alias a live program.
5. [generic] Each rebuilt configured program owns its nominal context; overlapping programs never share it even when their physical source rows coincide.
6. [generic] Expose the selected nominal context on the lookup contract; legacy workspace lookup accepts only unconfigured nominal types.
7. [generic] Check context before stripping a receiver to its physical owner, looking up a member, expanding an alias or reading owner generic parameters.
8. [generic] Preserve context in structural substitution and compare it in inference/applicability; neither physical-row equality nor display-name equality proves cross-context sameness.
9. [verification] Add failing-before tests for direct declaration interning and stale restore caches, then scoped identity, concurrent display poisoning, serialization, foreign-context and inference barriers.
10. [integration] Program-specific signature/member views consume this identity contract; this step does not alone claim the ambient call fixtures or real-project recall improved.

### Nominal-context implementation and verification

`Type::Decl` now carries an optional opaque `NominalContextId` separately from its physical row. `TypeArena::decl`, `decl_in`, direct `intern(Type::Decl)` and `lookup` share a numeric `(context, row)` index. Declaration payloads no longer enter the structural hash map whose derived keys include display text. A single write-lock recheck prevents competing display payloads from splitting nominal identity.

Two tests failed before the change: modifying only a bound declaration's display caused `lookup` to miss, and restoring an arena left a stale `decl_by_symbol` entry pointing at the previous arena's slot. Both now pass. Snapshot restore clears/rebuilds every nominal index and consistently remints serialized contexts; arena merge preserves context separation while remapping navigation rows. Old pre-context snapshots still load as unconfigured types. Content-only parse caches refuse to turn a configured nominal into a name-only head; the source binding recipes must reconstruct that identity.

```rust
// ✅ [generic] Same physical row, two program contexts => distinct TypeIds.
// ✅ [generic] Renamed/poisoned display, same context and row => same TypeId.
// ✅ [snapshot] Old program handles cannot interpret reconstructed nominal types.
// ✅ [generic] Mixed-context children are rejected before member/alias/owner lookup.
// ✅ [generic] Receiver inference compares the full nominal identity, not only its row.
// ✅ [generic] Configured member yields never retry a same-qname signature sibling.
// ✅ [generic] Source-bound type construction preserves private types; member use-site
//              access is checked separately by the existing ID-based access path.
// ❌ [integration] FileLookup program selection and program-specific signature/member
//                 materialization are still open; no ambient-cascade recall claim.
```

Nominal provenance is computed once when each structural type is interned; lookup context checks are O(1), not recursive per-reference scans. `FlowCacheLookup::nominal_context` carries the selected environment, and `contract/lookup_nominal.rs` centralizes nominal construction/checks on `dyn SymbolLookup`. Source type materialization and nominal roots consume this boundary. Program graph rebuilds mint distinct contexts for overlapping programs and refuse context selection for incomplete providers.

The initial broad run exposed 14 Rust alias/receiver regressions caused by adding workspace-global accessibility checks to source-bound type construction. `Compilation` has no requesting module at that phase, so legal private types were erased before their signatures could materialize. Removing that misplaced check restored the original cascades without weakening member use-site access. A dedicated positive-construction/negative-access test now enforces the distinction.

Verification: 18 new tests across nominal core identity, concurrent interning, restore/merge, portable cache barriers, context-aware lookup/yields, inference, annotation-region refinement and program rebuilds. The selected core/engine/lexical/cache/oracle run passes 1,176 tests, nine ignored, with all prior Rust oracle assertions retained. This is a targeted verification result, not workspace-wide or 99% evidence. Binding epoch remains 30 and extraction schema 50; the new optional nominal field is backward-readable and configured signature caches are not yet produced.

The final real-project report `resolution-documents/2026-09-07-query-core-nominal-context-report.json` (SHA256 `8b498ca3b51d03e44c96c9a0313c4d79cf6fc3162543c1664c2463377640cf81`) matches the previous report's complete fresh/cold occurrence objects and snapshot differences by deep equality: 557 correct / 40 kind-only disagreements / 93 unresolved across 690 labels. The runner still does not supply the new program descriptors, so this proves non-regression only. Five integration tests pass across `resolution_corpus_rust`, `per_file_manifest` and `per_package_context`. Native audits report no violations across 274 changed production files and 91 resolver/type-checker files. AlphaT and Lynx consumer checks again stop before compilation on their existing locked/offline dependency problems (yanked `der 0.8.0` and required lockfile update respectively); neither lockfile was changed.

Next work must actually select program/source instances in `FileLookup`, persist source-bound global type/value recipes, allocate program-local `GenericParamId`/TypeInfo/member views before signature materialization, and exercise the original compiler-labelled ambient cohort with explicit program inputs. Nominal context separation is implemented; it is not a substitute for those bindings, configuration discovery, index/call-signature merge evidence or dependent-snapshot invalidation.

## Configured call-binding integration design

1. [verification] Run the original independent ambient labels with explicit, source-hashed program inputs before changing production binding.
2. [profile data] Capture otherwise-unbound type names and array constructors as lexical name IDs at syntax ingestion; legacy display is retained only for unconfigured consumers.
3. [generic] Persist every supported source signature as declaration/global/import/parameter recipes, not workspace nominal TypeIds.
4. [generic] Allocate program-local generic owners before materializing any signature, sharing positions only for graph-attested declaration groups.
5. [generic] Build immutable per-program declaration, signature and member views without mutating workspace canonical identities.
6. [boundary] Select a unique configured source instance per file; overlapping, incomplete or stale configurations are explicit barriers, never unconfigured fallbacks.
7. [generic] Materialize lexical annotations and call arguments against the selected view, then resolve by source occurrence and numeric identities.
8. [generic] Capture unbound value-use anchors once and bind them through the program value domain; type-only globals never supply value roots.
9. [snapshot] Reconstruct views from persisted source recipes after cold load and rebuild them when source/configuration evidence changes.
10. [verification] Require exact callback, return-chain and module-isolation targets, then test namesake/overlap and poisoned-display non-interference before recapturing real-project evidence.

### Source declaration span correction

1. [evidence] The additional independently labelled `declare function make` target begins at `declare`; extraction currently begins eight bytes later.
2. [profile data] Describe declaration-modifier wrappers in lexical syntax data.
3. [generic] Normalize only named declaration spans wrapped by those forms, never variable-pattern anchors.
4. [generic] Correlate the existing physical row using source coordinates and kind before moving its navigation start.
5. [generic] Retain the CST declaration-node coordinate as a slot alias for lexical correlation.
6. [generic] Rebase generic-owner coordinates consistently before merge and type-recipe capture.
7. [generic] Use the same normalized anchor for program-global declaration inputs.
8. [snapshot] Repeated ingestion and portable-cache hydration must preserve the same row and signature ownership.
9. [verification] Keep the compiler's original target unchanged; add normalization idempotence and variable-pattern controls.
10. [scope] Do not use display spelling to find either the wrapper or its physical declaration.

## Configured call-binding implementation

Source-captured `NameId` recipes now represent global type/value uses and call selectors. Every supported source field, return, parameter and alias signature is persisted as a recipe, then materialized after the selected program's generic-owner IDs have been allocated. `program_view` builds isolated canonical declaration, TypeInfo and member views without rewriting workspace rows. `FileLookup` selects the unique source-hashed view; stale, overlapping and incomplete inputs do not fall back to workspace names. Binding epoch is 31 and extraction schema 51.

```ts
// ✅ [generic] Split Catalog<T>: first().touch() and each(item => item.touch()).
// ✅ [generic] Split Deferred instance/constructor surfaces: reject().then().
// ✅ [generic] Module-local type Portal does not shadow the global value Portal.
// ✅ [generic] A module-local value Portal still shadows that global value.
// ✅ [profile data] Array syntax selects the configured global generic owner.
// ✅ [generic] Explicit declare-global contributions share only attested owners.
// ✅ [generic] Method selectors, type heads and signatures ignore poisoned display.
// ✅ [snapshot] Provider edits/deletion, rejected groups and stale parsed sources
//              cannot revive old or unrelated declaration/type identities.
// ⚠️ [generic] The custom Array fixture uses disjoint named methods, not the real
//              stdlib's index/call signatures, overloads and repeated members.
// ❌ [integration] Automatic ecosystem program discovery is still absent.
```

The original 15 independent ambient labels all pass in configured mode; the separate unconfigured observation baseline is retained. Five additional independently TypeScript-validated cases add 12 positive labels, covering array-owned callbacks, a generic ambient function's argument/return cascade, type-versus-value shadowing and explicit augmentation. The new function fixture exposed a declaration-start mismatch (`declare` versus `function`); source-coordinate normalization repaired it without moving the compiler target. A portable-cache test verifies the generic owner survives this normalization.

Verification after the configured selector-ID path: 1,189 selected core/engine/lexical/cache/oracle tests passed, nine ignored. The final focused 12-test run also passed, including poisoned selector display, signatures and type payloads. Five integration tests passed across the Rust corpus and per-file/per-package contexts. These results establish the supported configured slice, not complete TypeScript semantics or a representative 99% result.

Remaining source recipe work includes complex type forms, generic constraints/defaults, base relationships and body-inferred signatures. Remaining environment work includes real stdlib merge compatibility, configured module/package instances, explicit selection for overlapping program memberships and dependency-driven rebuilding. Actual configured real-project measurement is now a separate oracle mode; the old unconfigured result must not be presented as its performance.

## Large-source identity capture contract

1. [evidence] The configured Query Core run abstains on all 690 labels; source diagnostics identify missing global capture for the supplied lib.dom.d.ts.
2. [cause] Both flow entrypoints and external contract restoration skip identity capture above the 512 KiB flow-query guard.
3. [generic] Separate the source identity pass from flow-query matching; file size is not permission to discard declaration, binding or type recipes.
4. [generic] Share call stamping, lexical capture and namespace/local identity capture in one helper used by fresh and contract-restoration paths.
5. [safety] Keep the existing size guard around expensive assignment/narrowing/return query matching and CFG construction.
6. [generic] Reuse the available parsed tree; create a shared tree for identity consumers even when query matching is skipped.
7. [snapshot] Restore filtered and portable large-source contracts by source coordinates with CorrelateOnly, without synthesizing body declarations.
8. [verification] Add a failing large-source fresh/tree-reuse/contract regression before moving the guard.
9. [measurement] Re-evaluate the unchanged compiler population and retain all other completeness barriers; missing capture and unsupported merging are distinct causes.
10. [scope] This does not authorize raising the query budget or guessing through unsupported ambient module declarations.

### Large-source implementation and verification

`flow_identity.rs` centralizes source call stamping and lexical/namespace capture. Both flow entrypoints now parse/reuse a tree and run this identity pass before the size guard; assignment/narrowing/return queries and CFG construction remain guarded. External contract restoration uses the same identity helper with CorrelateOnly, without the old size early return. The shared-tree consumer decision includes large flow/identity sources. Binding epoch is 32; extraction schema is 52.

The new size regression failed before the guard moved. It now verifies both fresh-parsing and shared-tree entrypoints preserve global/generic recipes while optional CFG work remains absent; a small-source control actually exercises CFG construction. Two further regressions check filtered/portable physical signature ownership and fresh/cold split-provider callback/return cascades with a provider larger than 512 KiB. Nineteen selected restoration/runner tests pass. The expanded core/engine/lexical/flow/parse/cache/oracle run passes 1,317 tests, nine ignored; no assertions or compiler labels were weakened.

Real-project verification confirms the 1.87 MB DOM declaration file now has a complete global capture. The other 55 incomplete source captures still fence the whole configured environment, so all 690 configured labels remain unresolved with fresh/cold parity. Resolving these barriers, real merge compatibility and the broader source-type recipe surface remains the next implementation work.

Final legacy non-regression report `2026-09-07-query-core-large-source-legacy-report.json` (SHA256 `376a0754d69dd633ba1abf262156aff959808bfbe22fa25d817aab5a398f8915`) matches the earlier nominal-context report's complete fresh/cold occurrence objects and snapshot changes: 557 correct, 40 kind-only disagreements, 93 unresolved. Configured report `2026-09-07-query-core-large-source-program-report.json` has SHA256 `4542af8f9aa927b030277ccfda044f004081cb102efee88bac38fd4dbb04788f`; its source barrier list drops from 56 to 55, with no changed call verdicts. The final five integration tests pass. Native audits cover 280 changed production files and 94 resolver/type-checker files with no violations; whitespace checks pass. Locked/offline downstream checks again stop before compilation: AlphaT on yanked `der 0.8.0`, Lynx on its required lockfile update. Neither consumer lockfile was modified and neither result is a compatibility pass.

## Parser prerequisite closeout (2026-09-07)

The pinned local TypeScript/TSX grammar repairs bare global augmentation nodes,
`keyof readonly` ownership and generic import-type/postfix syntax. All 187 real
compiler-supplied sources now parse without errors. Extraction schema 53 and
binding epoch 33 invalidate the earlier CST recipes. See
`typescript-provider-grammar-design.md` for the implementation and test evidence.

The configured source barrier count is now 54, not 55: the well-known-symbol
library capture is repaired. Node ambient module providers remain incomplete;
their now-visible global augmentations must be represented with source-owned
module scopes and program identities before lifting that barrier. No completeness
check was relaxed. Both real-project modes retain their exact prior per-call
verdicts and zero fresh/cold differences; configured recall remains zero.
