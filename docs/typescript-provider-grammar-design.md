# TypeScript provider grammar repair

1. [evidence] The pinned configured Query Core program has 55 incomplete providers; actual parse errors and unsupported ambient module capture are separate causes.
2. [boundary] Repair three source forms rejected by the pinned TypeScript grammar: bare global blocks, keyof readonly arrays, and generic import-type member applications.
3. [boundary] Extend the pinned upstream grammar structurally for TypeScript and TSX; do not rewrite user source or recover declarations from error text.
4. [identity] Preserve existing named nodes and byte coordinates so lexical/module ingestion can allocate source-owned IDs without spelling-based recovery.
5. [generic] Keep program completeness and unsupported-global barriers unchanged until module/augmentation identity is actually represented.
6. [delivery] Add a local grammar crate with checked-in generated parsers, upstream notices, pinned npm generation inputs and a reproducible generation command.
7. [integration] Route workspace TypeScript grammar consumers, including highlighting, through the same local dependency; preserve the existing Rust constants.
8. [snapshot] Invalidate extraction and binding caches because changed CST shapes can change declaration ownership and binding allocation.
9. [verification] First reproduce each error through the existing production grammar; then test both dialects, AST ownership, valid controls, invalid syntax and incremental parsing.
10. [measurement] Reparse all pinned compiler providers and rerun unchanged configured/legacy occurrence reports; parser success alone cannot count as resolved calls or completion of F1.

## Implemented (2026-09-07)

`crates/tree-sitter-typescript-local` extends the pinned upstream TypeScript
0.23.2 / JavaScript 0.23.1 grammars with CLI 0.25.10, ABI 14. The shared Cargo
dependency now selects this crate for both indexing and highlighting. Its public
language/node-type/query constants retain the upstream interface. Cargo builds
the checked-in generated C and does not invoke npm, download grammars or use the
TypeScript compiler. Extraction schema 53 and binding epoch 33 reject old CST
identity/cache recipes.

Bare global blocks retain `ambient_declaration` nodes, including source offsets;
explicit `declare global` does not gain an extra wrapper. `global` remains a
contextual identifier for ordinary calls, properties and labels. Generic import
types retain a `generic_type` with a structural member head and argument list;
import heads also participate in indexed-access and array syntax. Prefix type
operators bind beneath postfix operations but above unions/intersections.

```ts
declare module 'provider' {
  global { interface Catalog { read(): string; } }
  // ✅ [grammar boundary] A real augmentation node, no recovered `global` expression.
  // ❌ [generic] Program/module identity capture is still incomplete for this provider.
}
type Keys = keyof readonly string[] | number;
// ✅ [grammar boundary] Union(keyof(readonly(array(string))), number).
type Stream<T> = import('provider').Stream<T>;
// ✅ [grammar boundary] Generic arguments belong to the import-qualified type.
// ⚠️ [generic] Clean parsing alone does not bind its module/export/type IDs.
```

## Verification

- Three production grammar regressions failed before the replacement: bare
  global augmentation, mapped `keyof readonly`, and generic import type.
- Seven local grammar tests pass for both dialects: queries, AST ownership,
  source anchors, contextual identifiers, invalid syntax, dialect distinctions,
  and incremental/fresh parsing. Additional test development caught the ordinary
  `global()` regression and an incorrect operator tree before integration.
- The pinned TypeScript 5.9.3 developer verifier independently checks 14 syntax
  and AST-ownership cases. These are not compiler-provided symbol target labels.
- The read-only pinned-source gate parses all 187 supplied Query Core sources
  without ERROR/missing nodes and verifies input hashes before and after.
- 1,910 selected production tests pass, 26 ignored, spanning TS/JS/embedded
  languages, lexical/flow/engine/cache/core and occurrence oracles.
- Seven integration tests pass across TS/JS/Rust resolution corpora and
  per-file/per-package contexts. The grammar registry and highlighting tests
  also pass (2 + 9).
- Regeneration reproduces all 20 generated/vendored files byte-for-byte by
  SHA256. Native audits check 280 changed production files and 94 ID-discipline
  files with no violations; whitespace checks pass.

## Unchanged real-project occurrence evidence

The compiler manifest, original reports and target labels are untouched. New
reports use the version-two configured manifest in both execution modes:

- `2026-09-07-query-core-provider-grammar-program-report.json`, SHA256
  `477f744edffd2aec1e3fd951d0b51736484b7b91aa5f1c6ee37a12b0ef2e4a0d`:
  0 correct, 0 wrong, 690 unresolved; 54 incomplete provider captures instead
  of 55. The well-known-symbol library is no longer a source-capture barrier.
- `2026-09-07-query-core-provider-grammar-legacy-report.json`, SHA256
  `50842fc2e5bd4bc7d2fff98585b11d204951396834778e433fc4c5af761f9e73`:
  557 correct, 40 declaration-kind-only disagreements, 93 unresolved.

Both modes have zero fresh/cold differences. Counts and every expected/actual
occurrence verdict, plus kind-disagreement lists, deep-equal the previous
large-source reports. The legacy manifest revision differs because this run
uses version two rather than the old version-one legacy manifest; do not claim
the full report objects or revisions are identical. Neither report is eligible
for the 99% correctness gate, and no real-project recall gain is claimed.

## Next semantic work

All 54 remaining providers contain string-named ambient external modules. Their
names must become module identities at ingestion, not global identifier
declarations. Imports currently install at `ScopeId(0)` and module capture scans
only file roots; these must support source-owned module scopes before their
global contributions can be trusted. Nested augmentations now have proper CST
nodes and still trigger the conservative incomplete-provider barrier. Preserve
that barrier until module exports/imports, augmentation provenance and program
isolation are represented; then address real stdlib merge compatibility and
broader source type recipes. No resolver fallback or provider exclusion was added.
