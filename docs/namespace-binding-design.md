1. [generic] Extend the independent multi-file oracle with namespace calls, qualified types, nested re-exports, return cascades and negative non-interference labels before implementation.
2. [profile data] Describe namespace import nodes and qualified value/type selector fields in the TS/JS lexical syntax tables; no language-name branches in the resolver.
3. [generic] Capture qualified source selections into detached BindingIds, retaining their root import identity and ordered export selectors only at ingestion.
4. [generic] Lower namespace targets and qualified export paths into ModuleId/ExportNameId graph edges; keep type-only, missing and ambiguous outcomes explicit.
5. [generic] Resolve export paths with an iterative worklist and numeric cycle/budget guards, preserving namespace objects as module identities rather than fake declaration rows.
6. [generic] Install selector-addressed namespace member facts in the file environment; a local value shadow must never borrow a namespace's exports.
7. [generic] Enter the ordinary member walker only after an exported declaration ID is known; terminal namespace calls bind that exact declaration and retain typed yields/arguments.
8. [generic] Materialize qualified annotation/return/alias heads from detached BindingIds and reuse retained import-dependent signature recipes for reload/invalidation.
9. [generic] Add sibling tests for graph paths, ingestion scopes, missing/type-only fences and cold reload; extract actual helpers where existing module budgets require it.
10. [generic] Verify compiler labels, targeted engine/integration tests and ID/source gates; namespace-as-value data flow, CommonJS, package policies and full snapshots remain separate work.

Implementation notes:

- Ordinary export/barrel reachability stays iterative. Nested qualified-path queries preserve ambiguity at each intermediate namespace, detect active target cycles and share one bounded work allowance; they cannot restart an unlimited budget on recursion.
- Qualified paths are interned by numeric base target and selector IDs, so repeated source occurrences share completed graph-query results without sharing lexical occurrence identities.
- The namespace call adapter substitutes and seeds canonical signatures by GenericParamId directly; it does not enter the legacy same-qname overload selection helper.
- A proposed type-only value negative failed independent compiler validation: TypeScript navigation still returns a declaration for that invalid use. Keep engine abstention as a separate explicit policy test, not a compiler-equivalence claim.
