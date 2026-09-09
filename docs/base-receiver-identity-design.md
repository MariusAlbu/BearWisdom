# Source-bound base receivers

1. [evidence] Add independently compiler-checked superclass call fixtures before changing the engine; retain the real-project five wrong targets.
2. [profile data] Distinguish enclosing and direct-base receiver syntax during extraction; resolution never compares receiver spelling.
3. [generic] Preserve the semantic receiver kind through cached chains and reject base-root misses before namespace/global retry paths.
4. [ingestion] Capture class base heads as value-space BindingIds and their generic arguments as existing source type recipes; unsupported heads are explicit unknowns.
5. [identity] Lower owner slots to declaration IDs and persist source base recipes in the existing fingerprinted module input, including classes without bases.
6. [generic] Rebind direct base types through the owning module's value binding and generic TypeIds after imports/exports are materialized, never legacy inheritance-name seeds.
7. [generic] Root base access at the source method's enclosing declaration ID and its bound base TypeId; preserve parent generic applications for member-return cascades.
8. [tests] Cover overrides, inherited members, generics, renamed/namespace imports, namesakes, missing providers and changed/deleted providers, fresh/cold and portable caches.
9. [refactor] Keep receiver syntax in language data; extract the existing JS chain builder from oversized common code and reuse existing module/type metadata infrastructure.
10. [gate] Re-evaluate the unchanged Query Core manifest to a new report, preserve snapshot parity and retain remaining kind disagreements/unresolved calls; broader F0–F3 work stays open.

## Generic inheritance propagation

The compiler-labelled grandparent cascade still fails after source-bound edge capture: `substitution::hop_env_by_id` keys its environment by parameter spelling and calls `rebind_class_params`, which cannot replace captured `Type::Generic { param }` identities. Its preliminary qualified-name equality can also skip unrelated physical owners.

The shared fix is a declaration-ID inheritance walk composing `GenericParamId -> TypeId` environments. Both member yields and call-parameter inference consume the same environment. Source-attested class receivers must not retry the legacy string climb when an ID path is missing, cyclic or inconsistent. Keep legacy behavior only for input not yet migrated, and test exact IDs, identical display names, generic shadowing, cycles and conflicting paths separately from the compiler-labelled end-to-end cascade.

The direct parsed-input test also proved that the final legacy inheritance rebuild overwrote an already-bound `GenericParamId(U)` edge argument with `Class("U")`. Every rebuild must finish by applying source-bound owner edges, and the legacy resolved-inheritance prepass must not append competing name-derived parents to those owners.

## Real-project regression investigation

The first new Query Core report, `resolution-documents/2026-09-07-query-core-base-receiver-report.json`, is retained as a failing intermediate observation: fresh/cold both 546 correct, 40 incorrect, 104 unresolved, zero snapshot changes. Four old wrong-location targets became correct, one became unresolved, and ten previously correct calls regressed. All eleven newly unresolved sites require the same abstract base declaration, `Removable`. The source lexical profile lists ordinary class declarations but omits `abstract_class_declaration` from named declarations and generic type scopes. The stricter base binding exposed that missing value/type export input; restoring a name fallback would conceal it. An independently compiler-labelled abstract generic base fixture covers inherited and super calls before the profile correction.

## Accepted real-project result

`resolution-documents/2026-09-07-query-core-bound-base-report.json` uses the unchanged manifest (SHA256 `51f6db2ade740b26fdf300033cfb6316fbaa19d300f57a6428187f4799496db9`). Fresh and cold reports are identical: 557 correct, 40 incorrect, 93 unresolved out of 690 independently labelled calls; 80.7246% strict correct-binding recall, 93.2998% strict precision, 100% labelled extraction. All 40 remaining mismatches are declaration-kind-only disagreements, not silently relabelled successes. The 153 unlabelled compiler calls remain outside that labelled denominator. This is a development cohort, not held-out or general 99% evidence.

Exact per-site comparison with the enclosing-ID baseline proves only five changes, all incorrect-to-correct. The other 685 labelled outcomes and every expected target are unchanged; no previously correct target regressed. Corrected source bytes are file 169: 1876, 2180, 2541, 3503; file 176: 6059, mapping to the original compiler-provided parent declarations. The intermediate abstract-base regression is retained in its separate report rather than overwritten. Runtime 29,521 ms is debug evaluation timing, not a comparative performance benchmark.

Reproduction (the output path must not already exist):

```powershell
cargo run --offline -p bearwisdom --example project_oracle -- resolution-documents/2026-09-07-query-core-compiler-manifest.json resolution-documents/<new-report-name>.json
```

## Verification and limits

- TypeScript 5.9.3 independently validates 13 authored TS cases / 25 call labels and seven JS cases / nine labels. The combined 30 positive and four negative labels pass fresh, cold, and after deliberately poisoning legacy inheritance names and base-receiver display names. These are diagnostic fixtures, not representative language coverage.
- The direct parsed-source test asserts captured parent/ancestor GenericParamIds before checking the composed member return. A lookup double panics on string lookups and exercises identical display names, missing owners, cycles, generic arity gaps, and agreeing/conflicting diamond applications.
- Production-path tests cover late provider supply, unchanged-caller retargeting after a barrel edit, provider deletion, source-hash rejection, filtered declaration slots, portable cache hydration and fresh-arena snapshot reload.
- The abstract-class fixture failed before the profile correction and passes after adding its value/type declaration and generic scope forms. Binding epoch 29 and extractor schema 49 invalidate earlier source-ID allocations and cached receiver kinds.
- Selected core, lexical, module, flow, contract, extraction, canonical-form, oracle and type-system verification: 2,232 passed, 25 ignored. Five real-project compiler-adapter tests and the existing 79-label TS module verifier also pass.
- Twelve integrations pass across incremental indexing, per-file manifests, per-package context, and TS/JS/Rust resolution corpora. Native PowerShell audits checked 264 changed production files with zero budget or string-identity-pattern violations; `git -c core.safecrlf=false diff --check` passes.
- Downstream `cargo check --locked --offline --lib --target-dir F:/Work/Projects/BearWisdom/target` remains unverified: AlphaT stops before compilation on yanked `der ^0.8.0` through `ureq`/`ort`; Lynx requires a lockfile update. Sibling lockfiles were not changed, and these failures are not reported as successful API compatibility checks.

This does not complete constructor delegation via bare `super()`, mixin/factory/class-value base expressions, all generic defaults/constraints, static-versus-instance/access diagnostics, interface inheritance migration, complete language coverage, or the independent 99% gate. Existing legacy paths outside the migrated source-bound class inputs remain explicit F1/F2 work.
