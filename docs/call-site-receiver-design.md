# Source-addressed receiver inference

1. Scope: F1 receiver-adjustment evidence, a prerequisite of ordered trait/inherent selection; F0–F3 remain open.
2. Root cause: owned nominal receivers have no projected borrow; bound-call substitution replaces their output-region parameter with Unknown and alias applicability cannot continue.
3. Generic ingestion captures method-call selector bytes and their nearest source declaration slot from CST, never by searching identifier spellings.
4. FileLookup lowers these slots to persisted declaration IDs; missing/filtered owners and non-method calls cannot allocate region evidence.
5. Add a distinct Lifetime::Inference { owner, byte } identity: it is neither a declared generic parameter, Unknown equality, nor a solved outlives relation.
6. Generic receiver adjustment uses this ID only for a source-bound reference signature and an exact nominal/application Self match; pointers, unsupported signatures and absent occurrence evidence remain opaque.
7. Preserve existing directly projected reference provenance; owned-value adjustment does not substitute ordinary argument regions or infer static lifetime obligations.
8. Arena persistence/portable merging remap the inference owner alongside declaration IDs; invalidate source-binding/extraction caches for changed occurrence metadata.
9. Verify the retained failing owned cascade, zero/nonzero-argument calls, repeated selectors, distinct callers, aliases, locals, negative controls and fresh/cold snapshots against compiler target labels.
10. Keep trait visibility/implementation/bound identities, candidate ordering, reborrow/outlives solving, UFCS and representative per-language 99% evidence explicitly unfinished.

## Verification — 2026-09-07

The retained owned-receiver probe first failed with only `make`'s declaration ID
instead of the expected `make`/`touch` IDs. It now passes fresh and cold as a normal,
non-ignored regression test. Six additional fixtures in `rust_receiver_fixtures.json`
have 23 independently rustc-validated targets, all bound correctly fresh and cold.

Source capture distinguishes repeated dot-call selectors and nested same-named
owners; UFCS, function-value calls and removed declaration slots do not allocate
implicit receiver evidence. File overlays reuse the same caller/selector identity
independently of cursor movement. Actual provider edits retarget the nominal
receiver and output while preserving call-site identity; deletion removes the
target, including after fresh-arena cold reload.

Whole-arena snapshots preserve inference regions and portable arena merging remaps
their caller declaration IDs. Content-only extraction caches have no row-ID
remapper: they deliberately drop runtime inference regions to Unknown and recover
call-site ownership from source using the new symbol slots. Tests exercise both
this boundary and filtered/cache-restored ownership. These are different cache
contracts, not a spelling-based reconstruction of inference types.

Binding epoch 19 and extraction schema 39 invalidate older source capture.
The selected core/profile suite passed 2,089 tests (24 ignored), and all 12 selected
incremental/package/corpus integrations passed. The compiler adapter passed 12
tests; Rust verified 27 cases / 70 positive targets / five diagnostic negatives.
The TS compiler verifiers remain green at 136 scoped and 79 module labels.
Native file-budget/string-identity audits checked 231 changed production files
without violations; whitespace checks passed. No formatting or lint commands ran.

Public consumer checks used `--locked --offline` and a workspace-local target
directory. AlphaT could not resolve yanked `der 0.8.0` via `ureq`/`ort`; Lynx needed
a lockfile update. Neither reached compilation, so consumer compatibility is not
claimed; their lockfiles were not changed.
