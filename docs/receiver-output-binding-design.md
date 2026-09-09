1. [generic] Capture a distinct source-bound receiver type recipe by method declaration slot; ordinary parameter positions must exclude explicit as well as shorthand receivers.
2. [profile data] Recognize reference receiver syntax and direct Self through ingestion data. Initially support &self, &mut self and self: &Self with explicit/named/placeholder regions; unsupported receiver forms stay fenced.
3. [generic] Reuse InputRegion owner/source-site IDs for shorthand receiver borrows. Output recipes select only that receiver region, never regions inside Self's generic arguments or ordinary arguments.
4. [generic] Persist receiver_type_id beside canonical parameter_type_ids, with a retained Receiver signature slot for provider rebinding and deletion. Filtered external capture must regenerate correct owner slots.
5. [generic] Preserve reference projection evidence separately from the nominal receiver used by member lookup. Evidence records the last direct reference hop, and is cleared by unsupported wrapper/rehead/alternative transitions.
6. [generic] Extend the ID-keyed call environment with attested receiver evidence. Infer callee region IDs by structural receiver correspondence before final Unknown filling, including calls with zero ordinary arguments.
7. [generic] Do not invent borrow regions for owned values or claim reborrow/outlives proof. Missing or incompatible evidence remains Unknown; ordinary arguments cannot supply a distinct receiver parameter.
8. [generic] Extract call-argument orchestration from oversized chain.rs while retaining its compatibility entrypoint. No resolver language-name branching or format-and-re-resolve bridge is added.
9. [generic] Retain exact target-ID cascades plus direct region-ID tests, same-spelled owner isolation, named/placeholder/mutable receivers, argument positions and Unknown barriers; verify fresh/cold/cache/provider edits.
10. [evidence] Reproduce the retained legal receiver probe first, promote only after exact fresh/cold success, check fixture legality with rustc and rerun targeted core/integration suites and public API consumer checks. Independent Rust target labels and the 99% gate remain open.

Reference: https://doc.rust-lang.org/reference/lifetime-elision.html#lifetime-elision.function.receiver-lifetime . This is a bounded reference-provenance slice, not a borrow checker or complete receiver-adjustment implementation.

## Verified slice — 2026-09-07

```rust
fn make(&self, other: &Doc) -> Alias<'_>; // ✅ [profile data] capture selects receiver region only.
fn make(self: &Self, other: &Doc) -> Alias<'_>; // ✅ [generic] ordinary arguments exclude explicit self.
p.make(other).touch(); // ✅ [generic] direct reference evidence -> callee-ID substitution -> exact target.
p.make().touch(); // ✅ [generic] zero-argument calls no longer bypass receiver substitution.
let q = p.make(); q.touch(); // ✅ [generic] the substituted return survives local inference.
p.make(other).inner.touch(); // ✅ [generic] generic receiver/return/field cascade.
// p: C (owned), same calls: ❌ [generic] no captured call-site auto-borrow region yet.
// p: &mut C calling &self: ⚠️ [generic] mutability-changing reborrow remains Unknown.
```

- The former retained legal borrowed-receiver probe now passes exact declaration-ID checks fresh and cold. The new cohort has 16 compiler-accepted reference-receiver cases plus one retained legal owned-receiver failure; rustc 1.94.0 accepts all 17. Seven explicit legality groups passed, 192 fixtures total. These are legality checks, not compiler-provided target labels.
- Selected core/profile regression suite: 2,076 passed, 24 explicitly ignored, 5,287 filtered. The owned-receiver probe was explicitly run and fails `[C.make]` versus required `[C.make, Alias.touch]`; it remains a known missing-positive, not an abstention success.
- Direct call tests assert region IDs themselves, same-display/different-declaration rejection, missing/Unknown evidence, exact mutability, zero ordinary arguments and isolation from an ordinary argument's separate region ID. Projection tests retain the innermost direct reference and drop wrapper-derived evidence.
- Receiver signature tests cover late provider binding, same-named provider retargeting, deletion and fresh-arena cold reload while preserving the unchanged method's region ID. Filtered/portable external caches preserve receiver owner IDs and ordinary argument positions. Extraction schema 37 and binding epoch 17 invalidate older recipes.
- Public metadata adds serde-defaulted `TypeInfo.receiver_type_id`. AlphaT and Lynx compile attempts both stopped before compilation: AlphaT's locked/offline dependency resolution rejects yanked `der 0.8.0` via `ureq`/`ort`; Lynx requires a lockfile update. No `TypeInfo` struct literals were found in the inspected consumer Rust source trees. Both lockfile SHA-256 hashes stayed unchanged; consumer compatibility is not verified.
- TypeScript 5.9.3 independent oracle checks remain green: scope 67 cases / 136 labels, modules 25 cases / 79 labels. Native budget/string-identity audit: 230 changed production Rust/TypeScript source files, no violations; `git diff --check` passes. No formatting or lint commands were run.
- Owned-value auto-borrows, shared/mutable reborrow proofs, full Self-root typing, UFCS call convention/receiver positions, arbitrary receiver types, higher-ranked binders and borrow/outlives/variance solving remain open. No representative 99% correctness, performance, token-efficiency or competitor claim follows from this slice.
- Integration closeout: all 12 tests passed across incremental replacement/deletion/no-op, per-file manifests, per-package context and existing TS/JS/Rust resolution corpora. All Cargo sessions from this slice are terminal.
