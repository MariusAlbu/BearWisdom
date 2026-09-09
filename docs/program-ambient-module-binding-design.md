# Configured ambient module binding

1. [evidence] Source-owned module IDs are present, but configured imports still consult the workspace graph and nested global contributions are not captured.
2. [generic] Capture global augmentation declarations with their source-unit owner and validate the owning module chain before admitting program-level contributions.
3. [generic] Build each configured module graph from exactly its source-hashed input instances; absent/stale providers and overlapping program selection remain authoritative barriers.
4. [generic] Intern ambient provider names once while allocating program-owned module groups; all import/export traversal thereafter follows numeric module, binding and declaration IDs.
5. [generic] Reuse the shared module graph for explicit imports, namespace selectors, re-exports and provider unions; retain duplicate-declaration ambiguity unless legal merging is independently attested.
6. [generic] Install source-local import results in the program view before materializing signatures, using program-canonical declaration identities rather than workspace signature fallback.
7. [profile data] Module/augmentation syntax and export policy remain profile-attested ingestion data; no TypeScript-specific branch is added to semantic traversal.
8. [barriers] Do not admit unsupported source forms or invent provider targets; source completeness, module export completeness and legal global contribution evidence remain explicit.
9. [snapshot] Version changed source/global ownership recipes and verify filtered/cache, cold, source edits/deletion, provider arrival and program isolation.
10. [verification] Start with independently compiler-labelled ambient factory/generic/namespace/global cascades and negatives, then repeat the unchanged real-program occurrence census.

## Implemented and verified (2026-09-07)

The generic source capture now records each global contribution's `SourceModuleId`
and validates lexical containment separately from nearest module ancestry.
Function/block/namespace-contained augmentations cannot impersonate root or
ambient-module contributions. The configured program validates these owners,
including empty augmentation units, before admitting any global environment.

`ModuleGraph::for_program` selects source-hashed inputs belonging to one program,
allocates numeric provider groups, and reuses the shared import/export walker.
Ambient parts combine disjoint export surfaces; competing declaration IDs remain
ambiguous. Physical-module augmentations redirect the resolved target `ModuleId`,
not the raw relative specifier: augmenting `a/model.ts` cannot change `b/model.ts`
just because both consumers write `./model`. Missing augmentation targets cannot
manufacture providers. These graphs are rebuilt from persisted source evidence;
workspace graphs do not automatically acquire configured ambient providers.

Program views construct their graph before signature materialization. Import,
namespace and overload queries use that graph, and imported return/callback
recipes use program-owned declaration/generic/type IDs. String-query methods
retain no workspace fallback. Binding epoch is 35; extraction schema is 55.

```ts
// ✅ [generic] configured provider → import BindingId → exact declaration.
import { make } from "provider";
make().touch(); // ✅ [generic] program-owned return TypeId → member declaration.

// ✅ [profile data] capture a distinct source unit and its contribution owner.
declare module "provider" {
  global { interface Catalog<T> { each(cb: (item: T) => void): void; } }
}
// ✅ [generic] configured global group → shared GenericParamIds → callback yield.

// ⚠️ [generic] export-assignment identity is not represented yet.
import Provider = require("provider"); // authoritative incomplete capture.
export = Provider; // not a named/default-export approximation.
```

### Evidence

The new `resolution_oracle/ambient_module_fixtures.json` contains seven cases,
14 positive call targets and two diagnostic-backed negatives. TypeScript 5.9.3
independently verifies all 16 labels with `verify_modules.mjs`. Engine tests
require exact targets both fresh and in a new arena loaded from the database,
and unchanged reports with poisoned signature/qualified-name/selector display.
These are authored diagnostic tests, not a representative 99% gate.

Source/program/module tests also cover overlapping programs selecting different
providers for the same physical consumer, missing/competing providers,
configuration and source fingerprints, late arrival, actual provider source
edits, deletion, and filtered portable caches. A cached nested global member's
return retains the ambient module's local nominal owner after display poisoning.

Verification: 136 selected source/module/program/oracle tests passed; three
additional actual-source/cache/containment tests passed; the broader selected
core/language/oracle suite passed 1,936 tests (27 ignored). The read-only provider
diagnostic passed on all 187 pinned source files. Native file-budget and
ID-discipline audits passed (285 production files, 97 resolver/type-checker files).
Seven integration tests passed across `resolution_corpus`, `resolution_corpus_js`,
`resolution_corpus_rust`, `per_file_manifest` and `per_package_context`.
`git diff --check` passed. Existing unrelated compiler warnings were not changed.

### Real-project result and next work

The unchanged version-two Query Core manifest has SHA256
`44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`:
187 supplied files, 24 selected files, 843 compiler calls, 690 labelled targets.
No compiler supply or labels were dropped.

- `2026-09-07-query-core-program-ambient-binding-program-report.json`, SHA256
  `06f69a907e49a62341be1ef43e2bb4b8eccb3073f8d41d2e2fa0385d92c35393`:
  incomplete global-provider captures decreased from 54 to 14; zero missing
  captures. Still zero correct, zero wrong and 690 unresolved, fresh = cold.
- `2026-09-07-query-core-program-ambient-binding-legacy-report.json`, SHA256
  `430574673d8c62438d9904abb8c3ad552ef80c0e83367339b0f501a938f4b897`:
  unchanged 557 correct, 40 declaration-kind-only disagreements, 93 unresolved;
  80.7246% correct recall, 93.2998% strict precision, fresh = cold.

Deep comparison with the source-module reports finds every field unchanged
except elapsed time and the configured source-gap list. Neither report is gate
eligible. The improvement is 40 repaired capture barriers, not a real-project
recall gain.

The source-owned diagnostic isolates 28 incomplete literal units in 14 Node
providers; all have valid lexical containment and their file-root module capture
is complete. Every affected file contains import-equals syntax; export assignments
also occur in ten of them. Examples include EventEmitter import-equals in cluster,
domain, http2 and inspector, and `export =` plus provider aliases in path/constants.
The next generic representation must carry export-assignment entities and scoped
import-equals/internal aliases through IDs, including callable/namespace facets,
without turning them into default exports or loosening completeness checks.

Actual stdlib merge compatibility (repeated index/call/overload surfaces), richer
type recipes, implicit declaration-file ambient context, and automatic configured
program discovery remain open. F0 representative evidence, F2 semantic/query
completion and F3 product benchmarks/flows remain open as well.
