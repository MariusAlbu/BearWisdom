1. [generic] Keep member-call type arguments and parameter signatures as source-addressed type recipes at ingestion.
2. [profile data] Use the existing TS/JS call/type syntax tables; capture member selector byte addresses separately from expression roots.
3. [generic] Materialize recipes through BindingId -> declaration/GenericParamId -> TypeId, never through display names.
4. [generic] Store ordered parameter TypeIds on the exact callee TypeInfo and persist them with the canonical arena snapshot.
5. [generic] Substitute receiver, explicit-call and inferred argument bindings by GenericParamId, preserving owner identity and explicit priority.
6. [generic] Infer only matching ID-addressed structural shapes; leave mismatches and unknown arguments open, without a nominal-name fallback.
7. [generic] Seed callback declarations by their existing source spans from the substituted canonical parameter types.
8. [generic] Keep legacy signatures isolated for unmigrated inputs; imported/qualified heads and inheritance composition remain explicit follow-up work.
9. [generic] Pin sibling-local type collisions, member-chain argument addresses, generic-owner shadowing, callback signatures and reload preservation in tests.
10. [generic] Validate independent TypeScript labels, resolver/incremental suites and source/ID ratchets before checking off this foundation slice.
