# Source-owned call arguments

1. [generic] Capture semantic call operands once per CST selector, independently of display/connector `ExtractedRef.call_args` and duplicated graph references.
2. [profile data] Reuse configured call/path/member/wrapper syntax and explicit grouping-node data; no language-name branch in resolution.
3. [generic] Keep a three-state lookup contract: unmigrated source, attested operands (including zero arguments), or authoritative missing/ambiguous source evidence.
4. [generic] Store source-addressed argument syntax in the namespace input and bind every identifier leaf to its own lexical BindingId after declarations exist.
5. [generic] Expose selector-addressed operands through the per-file lookup without copying the table for each call or moving the active inference cursor.
6. [generic] Route bare calls, initializer pretyping, qualified/namespace calls and terminal/intermediate member calls through the same operand selector.
7. [generic] Missing captured evidence must not fall back to display identifiers, signature text or a previous inferred return; legacy-only sources keep their existing adapter.
8. [generic] Preserve connector/display payloads and recapture semantic operands from source after contract filtering and cache hydration; bump binding/extraction epochs.
9. [evidence] Add compiler-labelled bare/member/associated/imported direct and local cascades plus negative controls, then poison display payloads to test non-interference.
10. [boundary] This removes an argument identity bridge; it does not establish full expression typing, borrow-valued locals, closure ownership, overload correctness or the representative 99% gate.

## Implemented contract — 2026-09-07

`NamespaceData.call_arguments` owns source syntax by the exact callable selector.
The table is borrowed by `FileLookup`, not copied per call. Identifier leaves are
bound once from their exact CST spans into lexical BindingIds; the former second
pass over reference/segment display operands is removed for this captured path.
Rust supplies grouping/borrow syntax as profile data; selection and transport are
generic engine code, with no language-name branch.

`FlowCacheLookup::source_call_arguments` distinguishes:

- `None`: unmigrated source; the existing adapter remains available.
- `Some(Ok(args))`: authoritative source operands, including an empty list.
- `Some(Err(()))`: captured source without usable unique call evidence; no retry
  through display operands, spelling-based overload selection or an old return.

Bare calls, initializer pretyping, namespace calls, selected methods, qualified
trait calls and intermediate/terminal member calls consume that contract.
Captured calls require canonical signature IDs for argument substitution; a
missing signature cannot fall back to the string-generic adapter. Binding epoch
24 and extraction schema 44 invalidate older cached semantics. Source syntax is
recaptured after portable cache hydration and contract filtering; filtered body
references remain filtered, and a removed caller cannot attest a borrow region.

```rust
let x = keep(&p); x.touch();       // ✅ [generic] source argument IDs preserve the local-return cascade
let y = api::keep(&p); y.touch();  // ✅ [generic] module/declaration IDs plus the same operand contract
receiver.method(&p);              // ✅ [generic] exact output TypeId retains the ordinary argument region
factory()(&p);                    // ⚠️ [generic] colliding first-token selectors stay unknown; richer expression identity is pending
let borrowed = &p;                // ⚠️ [generic] borrow-valued local initialization is outside this completed slice
```

## Evidence

- Eight new pinned-rustc target-labelled cases: 28 positive targets and one
  diagnostic-backed negative. Before the migration: 22/28 correct, six unresolved;
  after: 28/28 correct, zero wrong, one correct negative, fresh/cold equal.
- All 24 argument/qualified/borrow cases retain identical full oracle reports
  after clearing or poisoning both reference and segment display operands, fresh
  and cold. Independent labels and historical snapshots are untouched.
- Exact return-TypeId assertions cover bare, inherent member, associated and
  namespace calls: outputs equal the actual argument's source-owned reference,
  including its caller/byte region identity, despite poisoned display arguments.
- Accessor tests distinguish empty, unknown and unmigrated states and reject
  stale yields when operands or canonical signatures are unavailable.
- Provider signature edits retarget both renamed-import and namespace local
  cascades from module a to module b; filtered providers, deletion and fresh-arena
  cold reload preserve exact IDs. Portable-cache tests independently rebuild
  semantic operands without resurrecting filtered body references.
- Targeted core/profile suite: 2,146 passed, 25 ignored. Pinned rustc 1.94.0
  verifies 69 cases / 157 positive targets / 14 negatives; 12 adapter tests pass.
- Native file-budget/string-identity audit: 247 changed production files,
  zero violations; diff whitespace check passes.

This is authored diagnostic evidence, not representative 99% proof. Remaining
work includes borrow-valued locals, field/dereference/value expressions, closure
ownership, implicit-Self qualified calls, region/coercion/outlives obligations,
advanced trait constraints and broader language migration.

## Integration and downstream verification

All 12 integration tests pass: incremental (5), per-file manifests (2), package
context (2), and the TypeScript, JavaScript and Rust resolution corpora (3).

The additive public lookup contract was followed by serial `cargo check --locked
--offline --lib` checks using BearWisdom's target directory for both consumers.
AlphaT stops at dependency selection (`der ^0.8.0`, yanked, via `ureq 3.3.0` and
`ort 2.0.0-rc.12`); Lynx stops because its lockfile needs updating. Neither reaches
application compilation. Their lockfiles are untouched; compatibility is not
claimed from these blocked checks.
