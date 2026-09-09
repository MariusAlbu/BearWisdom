1. [generic] Extend the shared TypeExpr representation with explicit source-namespace Uses, separate from local BindingIds.
2. [profile data] Describe composite type nodes, declaration type fields and generic parameter syntax in language data.
3. [generic] Capture annotations, field types, returns and alias RHS recipes from exact declaration/source addresses.
4. [generic] Bind nominal heads through namespace recipes and generic parameters through their declaration owner/index.
5. [generic] Distinguish unconfigured external boundaries from authoritative missing/ambiguous bindings; only the former retains existing legacy payloads.
6. [generic] Materialize and persist the same recipes through TypeIds, including import-dependent nested composite types.
7. [generic] Preserve exact type identities across field/member/return substitution and fresh/cold snapshots.
8. [generic] Extend source-owned initializer recipes to construction/value forms without restoring synthetic TypeRef guessing.
9. [generic] Add failing whole-pipeline positive/negative namesake, generic and initializer cases, then independently validate supported Rust labels.
10. [generic] Keep the complete roadmap and representative 99% gate open until evidence covers the full supported scope.

## Implemented slice — 2026-09-07

`namespace_types.rs` captures supported applications, generic declaration parameters,
field/return/alias recipes, transparent reference/pointer heads, tuples and function
type structure. Rust supplies syntax data. The shared `TypeExpr::Source` explicitly
uses the namespace binding arena, never the local-value binding arena. Locally
attested heads carry no legacy spelling payload. Existing external text is retained
only at an unconfigured boundary; known missing/ambiguous evidence lowers to Unknown.
Persisted source recipes repeat that distinction after reload (binding epoch 7).

Explicit struct initializers own a type recipe. Whole local copies own a source
BindingId and read byte; later writes or shadowing cannot retarget that copy. The old
separate simple-annotation/return capture and materialization path was removed.

```rust
fn f(p: api::Holder<b::Doc>) {
    p.inner.touch(); // ✅ [generic] source head IDs + owner/index GenericParamIds
}
type Wrapped<T> = api::Holder<T>; // ✅ [generic] source generic parameters, not signature parsing
let p = api::Holder::<b::Doc> { inner: b::Doc {} };
let q = p;
q.inner.touch(); // ✅ [generic] explicit construction recipe + position-correct copy
```

Verification: 1,923 selected core tests and 12 integration tests passed. The former
scoped-field known-red probe now has an exact declaration-ID assertion; the legacy
Rust integration gate is 33/33 patterns plus five exact constructor/member targets.
Both existing TypeScript compiler cohorts still verify 213 labels. These are not
independent Rust correctness measurements and do not establish the 99% target.

Open work includes inferred constructor arguments, tuple constructors/projections,
explicit call type arguments, generic impl-owner substitution, const generics,
arrays/slices, source-bound defaults/constraints, full pattern/CFG scopes, non-call
IDE occurrences and independent Rust labels. The Result-unwrap and external
second-hop corpus probes remain red. A cross-file private-module visibility gap
was also reproduced: the public-module positive and missing-module negative pass,
but a sibling source file's otherwise legal `crate::real` access still fails when
`real` is private. This belongs to module access policy, not spelling normalization.
