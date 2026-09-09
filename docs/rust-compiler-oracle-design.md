1. [evidence] F0 requires independent call-target labels, not merely rustc acceptance or engine-authored expected row IDs. Preserve the full F0-F3 goal; this instrument enables measured prioritization rather than certifying completeness.
2. [test boundary] Add a shared source-marked Rust cohort with explicit numeric reference/declaration pairs authored before evaluating BearWisdom, including known legal misses and compiler-supported trait/receiver/import cases.
3. [test boundary] A standalone Node verifier invokes the pinned installed rustc only on temporary fixture sources. It neither loads BearWisdom nor changes expected labels or observed snapshots.
4. [compiler evidence] Join THIR direct-call DefIds to HIR declaration-owner DefIds and physical spans. Never join by printed qualified name, method name or BearWisdom output.
5. [compiler evidence] Use a crate-scoped RUSTC_BOOTSTRAP value only in child compiler processes to inspect unstable debug dumps; production builds and parent environment remain unchanged. Pin the full rustc version and fail closed on unexpected format/version.
6. [test boundary] Decode source spans with explicit Unicode-character-to-UTF-8-byte conversion; preserve file identity, declaration-node starts, nested/same-line selectors and duplicate/missing-evidence barriers. No first-match target choice.
7. [generic harness] Extend fixture support with configuration-file inputs and configuration-aware fresh/cold compilation so cross-file Rust fixtures have actual Cargo crate roots. Preserve existing unconfigured corpus fingerprints.
8. [evidence] Record observed outcomes separately from compiler-validated labels. Known legal misses remain unresolved/not-extracted, not negative labels; reject incorrect targets and future per-site regressions.
9. [verification] Test the compiler-output adapter with malformed, ambiguous, Unicode and wrong-owner evidence; deliberately corrupt a label to prove verifier rejection. Run independent rustc validation, fresh/cold engine oracle and existing oracle/module/integration regressions.
10. [remaining gates] This initial semantic cohort is not representative of all real repositories or languages. Representative language baselines, held-out projects, the 99%/99.9% gate, complete F1/F2 semantics and F3 AI/IDE/flow benchmarks remain required.

Sources: https://rustc-dev-guide.rust-lang.org/thir.html ; https://rustc-dev-guide.rust-lang.org/hir.html ; https://doc.rust-lang.org/unstable-book/compiler-environment-variables/RUSTC_BOOTSTRAP.html . Compiler debug formats are unstable and must not become production APIs.

Implemented checkpoint: 21 independently compiler-checked cases, 47 positive
targets and five negative diagnostic labels; 11 adapter/adversarial tests pass.
Fresh/cold engine scoring agrees: 41 correct positives, six unresolved positives,
zero wrong targets and five correctly unbound negatives. The original observed
false positive is retained in the separate snapshot and forbidden by a dedicated
regression. Representative per-language/held-out evidence remains unfinished.

Selected core/profile verification: 2,083 passed, 25 ignored. The read-only Rust
report is an explicitly ignored diagnostic; the snapshot regression gate runs in
the normal oracle suite. Existing TS 5.9.3 checks still verify 136 scope labels
and 79 module labels. See `resolution-oracle.md` for reproducible commands.

All 12 focused incremental/package-context/TS-JS-Rust corpus integration tests
pass. Windows-native audits cover 231 changed production Rust/TypeScript files
with no file-budget or string-identity-pattern growth; whitespace diff check
passes. No formatting/lint commands or sibling consumer lockfile edits were made.
