1. [generic] Add a compiler-checked multi-file oracle cohort whose source labels are authored independently of engine results.
2. [profile data] Capture ES import/export syntax at ingestion, declaring local import BindingIds before annotation and argument-use binding.
3. [generic] Preserve separate value/type export domains, local alias BindingIds, explicit exports and wildcard edges; unsupported forms remain explicit.
4. [generic] Intern module paths/specifiers and export spellings at the compilation ingestion boundary into ModuleIds and ExportNameIds.
5. [generic] Link only exact reachable module files; do not use basename matches or workspace-wide declaration-name candidates.
6. [generic] Resolve explicit/local/re-export/wildcard edges through IDs, preserving missing and ambiguous outcomes and terminating cycles.
7. [generic] Materialize imported declaration IDs into lexical type recipes and file argument/call environments before the reference loop.
8. [generic] Bound imports with absent target evidence must abstain, not fall through to legacy global-name rules; local shadows remain distinct.
9. [generic] Preserve batch/reload identity evidence and test source order, missing exports, same-spelling modules, aliases and generic cascades.
10. [generic] Verify compiler labels, resolver and incremental tests and ID/source budgets; namespace objects, package policies and broader snapshot semantics need explicit evidence before completion claims.

Follow-up implementation decisions:

- [generic] Retain structural import-dependent signature recipes by BindingId and declaration ID, so late batches and changed export edges can rebind unchanged signatures; capture generic return templates only after this rebinding.
- [generic] Store module inputs with source fingerprints and the TypeArena transaction; discard stale/deleted persisted modules and prefer fresh parses. Persist existing package-entry evidence without claiming full package-policy correctness.
- [generic] Restore declaration-binding metadata after external contract row filtering, including cache hits whose payload omits flow; skip unsupported lexical profiles before parsing and leave body inference disabled.
- [profile data] Recognize ambient declaration wrappers and function signatures through the existing TS/JS syntax tables.
- [generic] Preserve source-declared overload groups as declaration-ID sets and callable intersections, union their available returns, and abstain from selecting a navigation target until argument-based overload selection exists.
- [generic] Traverse shared export graphs iteratively with a visited ID set, cache completed export queries, and report incomplete evidence at the traversal budget.
- [generic] Extract the existing optional-TypeId display fallback into the contract's sibling display helper to keep the public lookup module within its source budget; this helper formats output, not semantic identity.
