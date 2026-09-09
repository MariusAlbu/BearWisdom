# Source-owned type operators

1. Root cause: configured source signatures turn operator syntax into `Unknown` through `Legacy`, preventing meaningful stdlib compatibility checks.
2. Capture `keyof`, readonly, indexed access and conditional operators from CST profile data while source and lexical bindings exist.
3. Represent operands structurally with declaration/generic/type IDs; never decode displayed type text during semantic work.
4. Share a typed operator algebra across lexical recipes, durable program recipes, core TypeIds and portable caches.
5. Preserve conditional distributivity evidence from the source check-type shape; a substituted generic is not enough to reconstruct it later.
6. Traverse every operand during substitution, nominal-context checking and arena remapping; preserve missing/foreign evidence.
7. Keep unsupported operands unresolved and operators deferred; syntax identity does not prove assignability, legal merging or conditional evaluation.
8. Verify exact operand ownership, sibling generic isolation, unchanged-consumer provider edits, filtered portable inputs and cold snapshots.
9. Check independent TypeScript AST/checker evidence plus targeted engine/core/integration regressions; retain real-project per-site baselines.
10. Continue unique-symbol/value-query binding, mapped/infer owners, legal rich merges and initializer cascades; do not mark the parent roadmap task complete.

## Implemented identity path

```ts
interface Ops<T, K extends keyof T> {
  keys: keyof T;                    // ✅ [profile data] CST operator + generic-owner capture
  value: T[K];                      // ✅ [generic] TypeOperator operands carry canonical TypeIds
  frozen: readonly [T, K];          // ✅ [generic] readonly remains distinct from the mutable tuple
  choice: T extends K ? T : K;      // ✅ [generic] all four operands and distribution evidence survive substitution
  // ⚠️ [generic] These are deferred types, not evaluated operator results or merge certificates.
}
```

`TypeOperator<T>` is shared by lexical recipes, configured and import-dependent
module recipes, core types and portable caches. Its operand traversal preserves
nominal context isolation, generic substitution and arena remapping. Source
profile mappings are the only per-language changes; there are no new hooks or
member/API whitelists. Configured recipe lowering does not use display payloads.
Unimplemented operations cannot establish Rust receiver/trait applicability.

Conditional distribution retains `Some(true)` for a directly bound type
parameter, `Some(false)` for attested non-parameter forms, and `None` when alias
or simplification semantics are still needed. It must not be recomputed from an
already-substituted check operand. The independent controls follow the compiler's
conditional root, not a reimplementation of this classification. See the
[TypeScript conditional-type reference](https://www.typescriptlang.org/docs/handbook/2/conditional-types.html)
and [indexed-access reference](https://www.typescriptlang.org/docs/handbook/2/indexed-access-types.html).

## Verification, 2026-09-08

- Failing-before regression: source `keyof T` was `Legacy("keyof T")` (Cargo 41244).
- Initial generic operator mapper exposed six Rust closure lifetime diagnostics;
  the mapper now ties operand borrows to its own borrow rather than requiring
  unnecessarily higher-ranked closures (Cargo 22645/95973, repaired in 89682).
- One shared-fixture assertion distinguished JSON integer `0` from binary64
  `0.0`; corrected the fixture's representation without changing its value or
  compiler target (Cargo 74358). All original semantic assertions remain.
- TypeScript 5.9.3 independently verifies 12 fixtures, 19 operator nodes, 31 exact
  generic targets and zero diagnostics. Rust checks the same fixtures through
  both TypeScript and TSX grammars.
- Cargo 49332: 1,991 selected library tests passed, 29 ignored, zero failures.
- Cargo 70124: all seven selected integration tests passed.
- New configured tests cover filtered/portable source capture, exact bound
  operands, missing heads, poisoned display metadata and fresh-arena cold reload.
- New provider-edit/deletion tests retain the unchanged consumer and retarget
  imported operator operands in both configured and legacy modes.
- Binding epoch 43; external extraction schema 63. New operator source is 64
  lines; core `types.rs` is 683 lines versus 730 at HEAD. Seventeen touched
  production files pass the line budget; `git diff --check` passes.

Public consumer compilation is still unverified: the retained AlphaT/Lynx
locked-build dependency failures require separate lockfile-refresh authority.
No sibling source or lockfile is changed by this step.

Real-project recaptures retain the exact compiler manifest (SHA-256
`44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c`).
Both report JSON objects are identical to the preceding atomic-wrapper reports
after excluding only `elapsed_ms`, including every occurrence and cold result:

- Configured: `resolution-documents/2026-09-08-query-core-source-operator-program-report.json`,
  SHA-256 `baa6dc58c2994a66029332ab72df32dfc78b380d44b07560d26e4b6b58e2a8a2`;
  436 correct, 40 kind-only disagreements, 214 unresolved, zero configured-source
  gaps and zero snapshot changes. Cargo 19573 completed successfully.
- Legacy: `resolution-documents/2026-09-08-query-core-source-operator-legacy-report.json`,
  SHA-256 `446a2e6fc772548b501146a3bf007e53729a1b896efde9be2b9b77f302ab07b7`;
  557 correct, 40 kind-only disagreements, 93 unresolved and zero snapshot
  changes. Standalone current executable 8841 completed successfully.
- Recall remains 63.1884% configured and 80.7246% legacy. Neither gate passes.
  Timings (42,076/54,724 ms) are uncontrolled and not benchmark evidence.

No production code changed after the final library/integration verification.

Remaining: actual operator evaluation (including alias-dependent distribution),
value/type-query and computed unique-symbol identity, mapped/infer/template
source binders, compatibility proofs for rich merges, then initializer-derived
field/value cascades. Existing unknown barriers and the full F0–F3 objective
remain in force; neither type syntax nor these tests establish the 99% gate.
