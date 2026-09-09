# Explicit borrow argument identity

1. [generic] Extend extracted `CallArg` with a source-addressed borrow node and recursive operand, never a stripped identifier spelling.
2. [profile data] Supply borrow node/operand, mutability and raw-pointer discriminator syntax from the Rust namespace profile; no language-name resolver branch.
3. [generic] Reuse common argument recursion with optional borrow syntax, preserving parentheses, nested borrows, comments and argument positions.
4. [generic] Capture each borrow's exact span, mutability and physical enclosing function slot independently of graph-reference attribution; unsupported owners remain absent.
5. [generic] Lower physical caller slots to declaration IDs when constructing the file lookup; source span plus owner ID determines an inference-region identity.
6. [generic] Type borrowed operands through their own BindingIds and construct reference TypeIds only with attested source evidence; do not reuse dot-call regions.
7. [generic] Keep receiver and ordinary-argument borrows distinct through existing callee-ID substitution and trait selection, including returned/local cascades.
8. [generic] Preserve syntax via serialization and rebuild source ownership after contract filtering/cold hydration; invalidate incompatible extraction and binding caches.
9. [evidence] Add independently compiler-labelled fresh/cold shared, mutable, nested, generic, cross-file and negative cases before implementation; test exact region IDs separately.
10. [boundary] Source-addressed regions are identity, not borrow-checker/outlives/coercion proof; raw borrows, missing operands, malformed syntax and unattested owners cannot fabricate reference evidence.

The syntax distinction between shared/mutable references and raw borrows follows the [Rust Reference](https://doc.rust-lang.org/reference/expressions/operator-expr.html#borrow-operators). Temporary scope, place legality and outlives solving remain separate work.

## Implemented contract — 2026-09-07

```rust
<Doc as Keep>::keep(&p).touch();       // ✅ [generic] BorrowAt -> BindingId operand -> reference TypeId -> selected trait return
let x = <Doc as Keep>::keep(&mut p);   // ✅ [generic] physical caller ID + borrow span owns the mutable inference region
x.touch();                           // ✅ [generic] source-bound local retains the returned reference TypeId
<Input as Choose>::choose(&p, &&x);   // ✅ [generic] both nested reference layers retain distinct source regions
<Input as Choose>::other(&p, &x);     // ✅ [generic] ordinary output borrows x's region, not p's
<Doc as Empty>::absent(&p);           // ✅ [generic] missing trait member never selects an inherent namesake
<Doc as Keep>::keep(&raw const p);    // ⚠️ [profile data] raw-pointer syntax is not reference evidence
let borrowed = &p;                   // ⚠️ [generic] general borrow-valued initializer typing remains open
```

`CallArg::BorrowAt` serializes the exact expression span and nested operand; it
contains no operand name or arena-specific type ID. `BorrowSyntax` is generic
syntax machinery configured by Rust profile data. The source binder separately
attests `(SourceSpan, physical function slot, Mutability)`, and `FileLookup` lowers
that slot to a declaration ID. `borrow_argument(span, operand TypeId)` constructs
`Indirect::Reference(Inference { owner, byte })` only for attested spans with
non-unknown operands and a surviving owner. Partial spans, raw operators, missing
owners and unsupported closure owners cannot mint a reference. Dot-call regions
are a different source contract and are not reused.

The existing qualified-call path already consumes CST-stamped segment arguments;
it now gains borrow syntax without a separate resolution ladder. Legacy display
arguments remain on some bare/terminal-call APIs; this change does not claim their
complete migration. Dereference/place expressions, general local borrow inference,
coercions, higher-ranked regions, borrow legality and outlives remain open.

Eight new independently pinned-rustc-validated fixtures contain 20 positive call
targets and one diagnostic-backed negative. Tests additionally assert exact
receiver/ordinary output region equality, distinct owner/site identities, raw and
unknown barriers, provider retargeting/deletion, and fresh-arena cold reload.
Unfiltered portable payloads preserve nested borrow syntax. External contract
filtering intentionally discards body calls; recapture must not resurrect them
or borrow a removed nested function's physical slot.

The selected core/profile regression run passed 2,139 tests (25 ignored); all 12
compiler-adapter tests passed. Binding epoch 23 and extraction schema 43 invalidate
incompatible cached inputs. Native source-budget/ID-pattern audit: 246 changed
production files, zero violations. This is authored diagnostic evidence, not the
representative F0 99% gate or completion of F1–F3.

All 12 selected integration tests passed (incremental, per-file manifests,
per-package context, and TS/JS/Rust resolution corpora). Public consumer checks
were attempted serially with `--locked --offline`: AlphaT stopped before
compilation on its yanked `der ^0.8.0` dependency resolution; Lynx requires a
lockfile update. Their lockfiles were left untouched, so consumer compilation
compatibility is not yet verified.
