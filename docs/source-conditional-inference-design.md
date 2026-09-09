# Source-owned conditional inference

1. The traced QueryObserverOptions barrier is the QueryKey generic constraint: its Register conditional contains uncaptured infer declarations. No library-name hook or relaxed constraint is permitted.
2. Add a generic TypeOperator::Infer operand carrying a source-signature GenericParamId; it denotes a pattern binder, never an evaluated value or an unknown-type wildcard.
3. Describe infer syntax in TypeForm profile data. A generic source-capture module owns binder scopes and recipes; no language-name branch enters resolution.
4. Infer bindings are visible in the enclosing conditional's extends pattern and true branch, not its check or false branch. Nested conditionals retain distinct owners; unsupported duplicate/complex binder forms remain explicit.
5. Preserve infer signature spans, constraints and row-independent owners through normal durable source recipes, configured programs and portable/cold arenas.
6. Conditional evaluation returns proved match, proved non-match or unknown. The existing conservative boolean assignment failure must never select the false branch.
7. Use source-attested nominal key inventories to prove required-property presence/absence; evaluate selected property values lazily and collect inferred types by GenericParamId.
8. Evaluate only the selected branch after substitution, with ownership/cycle/depth/work barriers. Union distribution, variance, overload inference and unproved pattern shapes must abstain until their own semantics are implemented.
9. Add independent pinned-compiler cases for both branches, inferred values/constraints, shadowing, missing versus optional properties, invalid source and legal unsupported cases; test source identity, portable/cold and provider edits.
10. Rerun the exact real-project trace and occurrence oracle. Conditional evaluation does not itself prove mixed structural heritage, initializer inference, complete conditional semantics or the full 99% goal.

## Implemented and independently checked

`lexical_infer_types.rs` supplies separate extends/true branch environments with source-signature owners. False/check branches retain their outer bindings. Nested infer scopes remain distinct. The TypeScript profile registers `infer_type` as data; resolution contains no language or library-name hook. `TypeOperator::Infer` survives operand mapping and portable persistence, and generic substitution protects pattern binders rather than rewriting their declaration identities.

`program_conditional_eval.rs` makes branch selection an `Option<bool>` proof: `None` does not select the false branch. Supported source-object patterns use nominal key inventories, required-property absence/presence, selected values, scalar constraints and ID-bound inferred results. Only the selected branch is evaluated. Foreign/unowned/cyclic/unsupported evidence is fenced. Source Type::Function currently omits optional/rest/generic axes, so it is not accepted as callable conditional-identity evidence; an independently checked required-versus-optional function case protects this boundary.

Pinned TypeScript 5.9.3 verifies sixteen assignment cases (fourteen positive, two diagnostic-backed negative) and eleven obligation/barrier cases (three diagnostic-invalid, eight legal unsupported). The signature-owner cohort now contains five cases and twenty-six compiler-labelled uses, including seven new infer references. These checks follow the compiler's [conditional inference and branch scoping](https://www.typescriptlang.org/docs/handbook/2/conditional-types.html), not a custom spelling-based interpretation.

Failing-before evidence: the engine could not prove the empty-interface false branch; compiler-labelled infer markers exposed comments being treated as declaration names. The latter was repaired by selecting non-extra syntax children, not changing the compiler labels. The focused nominal/conditional cohorts also run through filtered portable caches, fresh/cold configured views and poisoned display data. Provider retarget/edit/delete tests preserve both inferred values and nominal navigation origins without recapturing the consumer.

## Real upstream trace

Before this change, the original pinned Query Core inputs produced `QueryKey -> None`, `keyof QueryOptions<...> -> None`, and `WithRequired<QueryOptions<...>, 'queryKey'> -> None` in the private proof view. The same read-only trace now produces `ReadonlyArray<unknown>`, the sixteen source-owned QueryOptions keys, and the complete `{ queryKey: {} } & QueryOptions<...>` intersection. The manifest input hashes were checked before and after both traces.

The next blocker is no longer guessed: `program_interface_heritage.rs` still requires a nominal declaration head and does not admit that mixed intersection. Its replacement must prove every branch and retain effective modifiers, generic constraints, callable evidence and source navigation origins; receiver-specific refinements must not overwrite a parent property's shared physical-row type.

## Remaining scope

Union/any distribution requires the original checked parameter's provenance, not substitution of coincidentally equal TypeIds. Duplicate-infer candidate collection, contravariance/overloads, optional present-value inference, nested outer-dependent infer constraints, recursive/higher-order patterns and complete scalar/nominal relations remain unfinished. The current optional policy is checked with strict TypeScript and exactOptionalPropertyTypes false; configuration completeness is not claimed. Full mixed heritage, repeated-property query staging, initializer-derived fields, multilingual 99% evidence, IDE/AI benchmarks and flows remain open.

Binding epoch is 53; extraction schema is 71. AlphaT/Lynx consumer compilation remains unverified under the previously recorded dependency/lockfile incompatibility; their lockfiles were not changed.

Boolean distribution is explicitly fenced when the checked type came from a distributive/deferred parameter; a concrete `boolean extends true` is independently checked as non-distributive. `undefined extends void` is a supported positive proof. Cross-representation Int/Number and Str/Utf16 comparisons remain unknown instead of manufacturing a negative equality result.

Final selected library verification passed 2,062 tests, with zero failures, 31 ignored and 5,623 filtered (30.93 seconds). The scope includes TS/JS/framework extraction, core types, lexical binding, resolution, flow, contracts/cache and the compiler oracles, not a workspace-wide claim. The original 41 assignment / 11 obligation / 25 merge compiler cohorts were independently reverified without changing their labels.

All seven selected integrations passed after the final production changes: `resolution_corpus`, `resolution_corpus_js`, `resolution_corpus_rust`, `per_file_manifest` and `per_package_context`. The native file-budget audit covered 316 changed production sources with zero violations; the scoped ID-pattern audit covered 117 with zero additions. `git diff --check` passed.

## Final configured occurrence measurement

`resolution-documents/2026-09-08-query-core-nominal-conditional-verified-program-report.json` is identical to the previous structural-evaluation report in its entirety after removing only `elapsed_ms`. All 690 fresh and 690 cold labelled occurrences are unchanged: 546 correct, 40 declaration-kind-only disagreements and 104 unresolved; recall 79.1304347826%, strict precision 93.1740614334%, zero source gaps and snapshot differences, gate false. Both retained QueryObserverOptions regressions remain open. This is an upstream semantic proof improvement, not a measured recall improvement.

Final configured report SHA-256: `3d1e1227deb17c14a3fbbff3c4be37c86ba3cbd5cdd22f13385ea1c2e0959fbc`. Elapsed time was 46,070 ms, not controlled performance evidence. The earlier `nominal-conditional-program-report.json` is retained as an interim run before the final scalar/distribution checks; it also matched every previous report field except its 42,969 ms elapsed time. Compiler manifest SHA-256 remains `44158824a3f91fdeda2d5e5da8a422054e178c76de29857c2f855ffe36a5110c` (187 supplied files, 24 selected, 843 compiler calls, 690 labels, 153 unlabelled, zero compiler diagnostics).

The final legacy report `resolution-documents/2026-09-08-query-core-nominal-conditional-legacy-report.json` likewise matches the preceding structural-evaluation legacy report in every field except elapsed time: 557 correct / 40 kind-only disagreements / 93 unresolved, fresh/cold equal, gate false. Elapsed time 55,451 ms; SHA-256 `207589c4718df83a47b78d6cab8f45eb25850ea0da12e3533da626fdf9a2efe8`. Both final report processes completed, and the final read-only process check found no Cargo, rustc or project_oracle process.
