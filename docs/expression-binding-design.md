# Named expression identity — F1 continuation
1. Own core lexical ingestion/cache, TS/JS syntax data, expression extraction and symbol visibility persistence.
2. [profile data] Describe named function/generator/class expression node kinds separately from declarations.
3. [generic] Allocate a private expression-name scope outside the function's parameter/body scope.
4. [generic] Bind self-name references by NameId/ScopeId/BindingId; parameters and locals may shadow that binding.
5. [generic] Correlate the expression declaration's exact source anchor to a distinct callable/class row.
6. [generic] Preserve the outer variable declaration separately, linking initializer value provenance by numeric slots/IDs.
7. [generic] Seed canonical callable/constructor TypeIds from that provenance without formatting or re-resolving names.
8. [generic] Exclude private expression declarations from unscoped candidate ladders by ID, including after DB reload.
9. [generic] Preserve existing rows/ref slots and honest uncertainty; do not synthesize guessed function signatures.
10. Verify private recursion, parameter shadowing, outer binding calls, class self/static/constructor cascades and non-leakage against compiler-labelled source addresses.
