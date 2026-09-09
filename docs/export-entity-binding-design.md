# Export-assignment entities and scoped aliases

1. [evidence] Fourteen pinned Node providers have 28 incomplete literal units; import-equals and export assignments are the upstream missing representation.
2. [profile data] Capture import-require, internal import aliases and export-assignment CST forms through explicit profile fields; no package/API spelling lists.
3. [generic] Give namespace declarations lexical BindingIds before aliases/selectors are captured, retaining legal declaration facets and rejecting conflicting owners.
4. [generic] Persist export assignments separately from named/default exports and preserve alias bases as source-owned BindingIds plus selector recipes.
5. [generic] Add numeric module-entity targets with declaration/overload and namespace facets; selecting a facet must not discard the identity of the other facet.
6. [generic] Reuse graph cycle/ambiguity/completeness accounting for assignment forwarding and internal aliases; missing assignments are not default-export fallbacks.
7. [generic] Keep callable roots and namespace selectors separate in file facts; use bound IDs for factory returns, class types and merged namespace members.
8. [snapshot] Version source recipes and verify overlapping providers, edits, deletion and filtered/new-arena cold caches; do not reuse old BindingIds.
9. [oracle] Pin compiler options and resolved-signature targets for callable merged entities, with negatives and same-name isolation; preserve existing oracle modes.
10. [gate] Re-run unchanged real compiler supply and report capture/merge/type barriers separately; neither a clean parse nor passing authored cases establishes 99%.

## Implemented and verified — 2026-09-07

- [profile data] TypeScript module forms describe import-require, internal aliases and assignment tokens. No package or API spelling lists were added.
- [generic] Namespace value/type bindings are installed in the enclosing source module's lexical body, not the namespace's declaration scope. Internal aliases are all declared before their RHS paths bind, retaining forward shadowing and numeric owner edges.
- [generic] Export assignments persist separately from named/default exports. Module targets preserve declaration or overload alternatives alongside namespace identity; path traversal and callable roots consume the appropriate facet. Missing competitors, cycles and ambiguous declaration facets remain barriers.
- [generic] Type-space declaration groups are consulted before the value-space ambiguous-slot guard. The broader tests caught and repaired a regression that otherwise rejected three existing compiler-labelled interface-merge cohorts.
- [snapshot] Binding epoch 36 and external extraction schema 56 invalidate older ID allocation/recipes. Tests cover callable and namespace facets through actual provider edits, late arrival, deletion, overlapping program selection, filtered portable source caches, arena restoration and poisoned display/signature/selector strings.
- [evidence] `export_entity_fixtures.json`: 11 cases, 17 positive call targets and four diagnostic-backed negatives independently verified with TypeScript 5.9.3. Coverage includes callable factories, forwarding assignments, generic class/namespace return cascades, exported/internal/forward aliases, missing providers/members, cycles and default-export separation. Existing 79-label module and 16-label ambient module verifiers still agree with the compiler.
- [verification] 1,942 selected core/profile tests, seven integrations and five project-trace adapter tests passed. After adding overload-facet assertions and the ignored provenance diagnostic, the compiled focused seven-test run and both read-only real-source diagnostics passed. Native budget/ID audit: 286 production files, 97 ID-scoped files, zero violations; `git diff --check` passes. No public API changes, source-project writes, commits or cleanup.

## Real-source results and remaining failures

The unchanged pinned manifest still supplies 187 files and labels 690 of 843 compiler calls in 24 selected source files. Every source/configuration hash is verified before and after evaluation. All 28 incomplete literal units in 14 Node providers are now represented: the source-module diagnostic and configured report both show zero capture gaps.

| Evaluation | Correct | Kind-only disagreements | Unresolved | Fresh/cold changes |
|---|---:|---:|---:|---:|
| Configured, previous | 0 | 0 | 690 | 0 |
| Configured, export entities | 354 | 40 | 296 | 0 |
| Legacy, unchanged | 557 | 40 | 93 | 0 |

Configured recall is 51.3043% and strict precision 89.8477%; neither is near the required gate. All 40 strict mismatches have the correct declaration coordinates but disagree on syntactic kind. The entire legacy report is structurally identical to the previous report except elapsed time. The configured path retains 354 legacy-correct sites, leaves another 203 legacy-correct sites unresolved, and retains the same 93 legacy misses; it does not improve by reviving legacy spelling fallbacks.

Reports (created exclusively, previous evidence untouched):

- `resolution-documents/2026-09-07-query-core-export-entity-program-report.json`: SHA-256 `fbcc0d0f30b6f686bc6cfbb0b7db70df478e0761bd7bb4f5f79983f32619d249`.
- `resolution-documents/2026-09-07-query-core-export-entity-legacy-report.json`: SHA-256 `0cc4adf10abcc95d7293d93815ed7664e28e004dd2efe28d1d235115cd14c93a`.
- `resolution-documents/2026-09-07-query-core-export-entity-program-traces.json`: SHA-256 `9d6e78d7713f3391bfccfbed216ed49ca7cc9d5dfc3813567ef55011fd7b38f6`; 296 requested unresolved occurrences, 1,126 raw trace records, zero baseline or snapshot changes. Its full report equals the ordinary configured report except elapsed time. Raw trace-record totals are not occurrence counts.

The retained `diagnose_pinned_program_global_and_signature_barriers` test verifies the next upstream failures against actual source:

```ts
Promise.resolve(value) // ❌ [generic] PromiseConstructor group is Incomplete; computed/construct/overload member compatibility is not represented.
class FocusManager extends Subscribable<Listener> {} // ❌ [generic] input.bases contains Import(13), but the configured TypeInfo has base_type_id=None.
protected listeners = new Set<TListener>() // ❌ [generic] configured field TypeInfo has no field_type_id; initializer-owned typing is not materialized.
```

Array, ReadonlyArray, Set, Map, Promise and PromiseConstructor type groups all remain Incomplete with their full supplied declaration parts retained. This is no longer a parser/source-module-supply failure. Next work must represent legal member/merge evidence and source-owned signature/base/initializer recipes, not remove barriers or accept groups by library name. Repeated namespace/entity merging, complete assigned-module interop and further type-space/expression forms remain open. F0–F3 parents remain open.
