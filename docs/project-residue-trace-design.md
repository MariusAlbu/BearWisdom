# Compiler-cohort residue tracing

1. [evidence] Diagnose the fixed real-project compiler cohort; do not change labels or source inputs.
2. [boundary] Add a standalone example, using the public project oracle and existing trace collector.
3. [identity] Validate the baseline revision and every expected source-addressed target against the manifest.
4. [selection] Select baseline unresolved occurrences, not a newly successful engine-dependent population.
5. [boundary] Convert UTF-8 selector offsets into diagnostic file/line filters; no spelling is used for selection.
6. [generic] Run the existing fresh/cold oracle unchanged, against an in-memory database.
7. [report] Preserve requested occurrence IDs, raw traces, the new oracle report and exact baseline changes together.
8. [limitation] Legacy trace filters accept adjacent line conventions and refs may start before their selector; traces are diagnostic, not occurrence truth.
9. [safety] Verify source hashes and reject existing output files; a scope guard disables tracing on failure.
10. [verification] Test wrong revisions/labels, duplicate or omitted labels and UTF-8 addresses; require unchanged targets during diagnostic-only runs.

## Delivered diagnostic path

`cargo run --offline -p bearwisdom --example project_trace -- <manifest.json> <baseline.json> <new-diagnostic.json>` verifies the source/configuration supply, baseline revision and full compiler target population before selecting unresolved sites. Its four example tests cover the validation and UTF-8 boundaries. Output is created exclusively and includes the original baseline SHA-256, requested source sites, raw traces, new fresh/cold report and exact per-site baseline changes.

Three retained Query Core diagnostic artifacts (`2026-09-07-query-core-residue-traces.json`, `2026-09-07-query-core-callback-traces.json`, `2026-09-07-query-core-owner-traces.json`) each contain 93 requested sites and 362 raw records. All preserve the 690 baseline reference outcomes, including the unresolved and kind-only failures, with zero fresh/cold changes. Callback instrumentation records the materialized signature, source parameter span, remaining generic ID and receiver/member owner paths.

The trace macro previously evaluated diagnostic operands before the inactive flag could reject the message. `disabled_trace_macro_does_not_evaluate_diagnostic_operands` reproduced one operand evaluation with tracing disabled. The flag now guards construction of `format_args!` at the macro call site; active collection still evaluates the operand once and captures its output. This avoids new tracing overhead when off and removes the same overhead from existing macro calls. Diagnostic values computed outside the macro remain outside this guarantee; no runtime-speed percentage is claimed.

## Verification closeout

- `cargo test --offline -p bearwisdom --lib -- indexer::resolve::engine resolution_oracle::`: 1,035 passed, nine ignored, 6,455 filtered; includes the ambient per-site baseline gate and all five trace tests.
- `cargo test --offline -p bearwisdom --example project_trace`: four passed.
- TypeScript 5.9.3 verifier: original module cohort 79 labels, base-receiver TS 25, JS nine, ambient 15; all targets agree. Ambient diagnostic codes also agree.
- `2026-09-07-query-core-trace-disabled-report.json` records the ordinary untraced path. Deep object equality confirms all 690 complete occurrence records and counts equal the original bound-base baseline and the traced report in both fresh/cold modes. JSON property order differs between nested diagnostic and direct report serialization and is not semantic evidence.
- Existing-output invocation rejects the write and preserves the report SHA-256.
- Native PowerShell equivalent of file-budget/ID gates: 266 changed production source files checked, zero violations. `git diff --check` passes.
- No public function signature/type-layout changes, source-project writes, sibling lockfile changes, commits or cleanup in this diagnostic pass. Existing warnings remain; no workspace-wide or downstream compatibility claim.

## Configured-program diagnostic continuation

1. [evidence] The export-entity configured baseline now has 296 unresolved calls; legacy remains a separate 93-call residue.
2. [boundary] Read the baseline's binding mode before selecting the evaluator; never silently trace a different semantic environment.
3. [compatibility] Reports predating the mode field retain the documented legacy behavior.
4. [safety] Reject unknown mode spellings instead of falling back to a permissive environment.
5. [identity] Preserve manifest revisions, exact source labels and source/configuration hash verification.
6. [generic] Reuse evaluate_manifest_with_mode; no resolver changes or spelling-based target queries.
7. [evidence] Keep the same requested occurrences, source-line trace filters and baseline comparisons.
8. [tests] Add adapter cases for configured, explicit legacy, absent and unknown modes.
9. [verification] Compare diagnostic and ordinary reports structurally, excluding only elapsed time.
10. [next] Trace receiver/signature provenance before proposing stdlib merge or type-recipe changes.

Verified: five adapter tests passed, including absent/explicit legacy, configured and rejected unknown modes. `2026-09-07-query-core-export-entity-program-traces.json` contains 296 requested unresolved sites and 1,126 raw records, zero baseline changes and zero fresh/cold changes. Deep structural comparison of its complete report with the ordinary configured export-entity report passed after excluding elapsed time. The diagnostic selects the baseline's configured environment and does not silently rerun legacy semantics.
