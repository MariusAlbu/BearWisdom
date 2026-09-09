1. [generic] Separate explicit module roots from unqualified lexical roots; self/super paths must not retry a missing member as an external crate.
2. [generic] Represent explicit paths as module-ID selections so inline and out-of-line paths use the same access checks.
3. [profile data] Decode visibility syntax and crate/self/super keywords at ingestion using the existing namespace Forms.
4. [generic] Capture named restrictions as declaration-only module paths, distinct from ordinary import/export paths.
5. [generic] Lower their source names once to export-name IDs and retain configured root/parent evidence.
6. [generic] Traverse only direct module declarations and their binding IDs; imported module aliases cannot establish an access scope.
7. [generic] Validate that the resulting scope is an ancestor of the declaring module; missing, cyclic, ambiguous and non-ancestor scopes remain unknown.
8. [generic] Preserve the ordinary public/private access and re-export ceiling mechanisms, finite work and cold-reload fences; advance the binding epoch.
9. [generic] Add failing tests for explicit-root external namesakes and named restrictions, then test sibling/descendant/outside access, aliases, source layouts and reload.
10. [generic] Independently validate accepted/rejected Rust fixtures; keep member privacy, module instances, edition-specific legacy paths and the full 99% gate open.

References: https://doc.rust-lang.org/reference/visibility-and-privacy.html and https://doc.rust-lang.org/reference/paths.html

## Verification (2026-09-07)

Both new regression groups failed before implementation: explicit `self::api` selected dependency declaration ID 3 instead of abstaining; `pub(in crate::outer)` lost the legal constructor ID 7. Both now pass, including downstream members and database reload.

The shared graph interns declaration-only scope paths, resolves them once during rebuild through direct declaration/binding/module IDs, verifies structural parent edges, and checks that the final module is an ancestor of the declaring module. Import aliases, non-ancestor targets, missing/duplicate declarations and unbounded binding cycles cannot establish a visibility scope. Normal selector traversal does not resolve these scope-only recipes. Binding epoch is 9.

1,937 selected core tests passed, with 18 ignored. Both ignored rustc checks were run explicitly: 19 source fixtures total (eight visibility fixtures plus four dependency-path and seven named-restriction fixtures) agree on compiler acceptance/rejection. These checks validate fixture legality, not compiler-provided occurrence targets. The Windows-native budget/ID audit checked 112 changed production files with zero violations; diff whitespace check passed.

Full member/field privacy, configured module instances, legacy edition-specific paths, independent Rust occurrence labels and representative 99% recall/precision measurements remain open.

All 12 selected integration tests also passed: incremental indexing (5), per-file manifest context (2), per-package context (2), and TS/JS/Rust resolution corpora (1 each). Existing compiler warnings remain; no formatting/lint or sibling public-API changes were made.
