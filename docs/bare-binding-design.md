# Bare-reference identity — implementation slice
1. Core crate: `bearwisdom`; continue F1, without claiming the full 99% gate.
2. `[generic]` Build lexical scopes before legacy flow-query correlation.
3. `[profile data]` Reuse the existing TS/JS syntax table; no new language hook.
4. `[generic]` Correlate declaration spans to extracted symbol slots; synthesize missing local rows without reordering existing slots.
5. `[generic]` Parent synthesized rows by full source coordinates, never same-line spelling.
6. `[generic]` Route migrated assignment captures through BindingId → symbol slot; retain legacy handling only for unmigrated files.
7. `[generic]` Map BindingId → persisted row ID using strict positional identity; missing/ambiguous rows must abstain.
8. `[generic]` Bind bare local calls before the global ladder; the target is the local declaration, not a speculative runtime callee.
9. `[generic]` Keep callable return TypeIds separate from reference target identity; do not format-and-re-resolve.
10. Verify failing source-labelled cases with TypeScript, then test ingestion, identity maps, resolution, and existing scope/callback cohorts.

Integration finding: destructured call references bind local declarations, while
the existing call graph can retain a separately evidenced implementation target.
`LocalReference` carries both IDs; `ref_resolutions` stores the lexical declaration
and the graph projection uses a known callable ID when available. A call edge to
a parameter/variable without callable provenance remains a dependency, not proof
of a concrete runtime destination; richer flow-edge semantics are still required.

```ts
function first(callback: () => void) { callback(); } // ❌ baseline [generic] wrong declaration / unresolved
function second(callback: () => void) { callback(); } // ❌ baseline [generic] same-line slot conflation
// Intended: both calls bind their distinct parameter SymbolIds, independently of callable inference.
// ⚠️ [generic] function/class/import scopes, complete CFG semantics, overload sets and other languages remain follow-up work.
```
