1. [generic] Keep module reachability and item accessibility separate; private entries remain indexed but cannot become public fallback candidates.
2. [profile data] Capture module-item visibility and the requesting source module from language syntax, not qualified-name prefixes.
3. [generic] Persist access scopes as source-module recipes, then lower them to numeric module ancestry before binding.
4. [generic] Carry each path's requesting ModuleId through selector traversal; imports and aliases evaluate their paths in their declaration environment.
5. [generic] Gate private/restricted exports by descendant-of-scope checks and preserve missing/ambiguous/configuration barriers.
6. [generic] Record each declaration/module's own visibility ceiling; a public alias may bypass private ancestors but cannot illegally widen a private terminal item.
7. [generic] Retain the existing public-only traversal for profiles without source access metadata; no runtime language branches or name fallback.
8. [generic] Verify inline/out-of-line parents, sibling/descendant access, public aliases, external denial, restricted scopes and cold reload with exact target IDs.
9. [generic] Recheck the Rust probe fixtures against compiler semantics; the existing Result probe lacks its crate-root alias wiring and is not independent truth.
10. [generic] Keep full member privacy, configured module instances, unsupported visibility syntax and the independently measured 99% gate open until separately verified.

Semantic reference: https://doc.rust-lang.org/reference/visibility-and-privacy.html

Verified on 2026-09-07: 1,931 selected core tests passed (17 ignored); the ignored rustc acceptance/privacy check was run separately and passed for eight fixtures. The original private-module composite return cascade passes fresh and cold, with exact constructor/member row IDs and inaccessible namesakes denied. Access-only rebuilds, retargeting and deletion match a fresh module graph. Binding recipe epoch is now 8.

The compiler check validates acceptance/rejection, not compiler-provided target occurrence labels. Full field/member privacy, named ancestor restrictions, module instances, full incremental dependency invalidation and representative 99% measurement remain open. The legacy Result probe also lacks a crate-root Result alias; do not infer Try output by matching a type spelling.

Integration verification: all 12 selected tests passed (incremental, per-file manifests, per-package context, TS/JS/Rust resolution corpora). TypeScript 5.9.3 independently rechecked 79 module-call labels and 134 scope-call labels. Windows-native budget/ID audit checked 112 changed production files with zero violations; diff whitespace check passed. Existing compiler warnings remain.

Follow-up: [module scope paths](module-scope-paths-design.md) implements explicit-rooted named ancestor restrictions and closes explicit self/super external-name fallback at epoch 9. The earlier named-restriction limitation above describes the epoch-8 snapshot, not the current implementation.
