# Lexical type heads — F1 continuation
1. [generic] Keep separate value/type namespace entries keyed by ScopeId and NameId, sharing BindingIds for dual declarations.
2. [profile data] Declare type-only names, dual names and type-parameter scopes through syntax data.
3. [generic] Capture annotation and explicit-argument syntax into structural recipes after all lexical declarations exist.
4. [generic] Bind nominal heads to exact type-space BindingIds, including inner/outer shadowing and private expression names.
5. [generic] Materialize bound recipes from exact persisted row slots into Decl TypeIds; missing rows become Unknown, never name fallback.
6. [generic] Preserve composite/functional/applied type structure and canonical generic-parameter identities where captured.
7. [generic] Install binding-owned annotation and occurrence-owned argument TypeIds before reference resolution.
8. [generic] Carry source-bound signature types into declaration-ID metadata; do not format and re-resolve them.
9. [generic] Keep imported/qualified/unsupported syntax explicitly transitional; do not claim those paths are migrated.
10. Verify same-name local classes, type/value separation, explicit-argument cascades, private names and generic shadowing against compiler labels and regression suites.
