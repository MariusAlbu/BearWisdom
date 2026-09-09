1. Reuse the generic configured structural evaluator for source-bound member receiver aliases.
2. Keep grammar, alias spellings and member names at the existing ingestion boundary; no profile or hook changes.
3. Expose an optional evaluated receiver through the lookup contract, with authoritative configured failures.
4. Delegate configured file lookups to their selected source view and preserve legacy expansion elsewhere.
5. Evaluate the complete original alias application so defaults, constraints and source parameter IDs stay checked.
6. Preserve source declaration and signature origins when the chosen conditional branch becomes a receiver.
7. Keep unknown operands, unsupported distribution, recursive aliases and foreign contexts as barriers.
8. Verify the retained compiler-intrinsic read-to-touch cascade before adding branch, constraint and state controls.
9. Pin independent TypeScript diagnostics, exact declaration/signature labels and downstream result types.
10. Rerun targeted resolver tests, integration suites and the frozen fresh/cold benchmark before closing the task.

```ts
interface Payload { touch(): number }
interface Box<T> { read(): T }
type Choose<T extends string = "yes"> = T extends "yes" ? Box<Payload> : never;
declare const selected: Choose;
selected.read().touch(); // resolves: configured Eval checks the alias, then source IDs select both methods [generic]
type Invalid = Choose<number>; // rejected receiver: the original alias constraint is checked before substitution [generic]
type Loop<T> = T extends string ? Loop<[T]> : Loop<T>; // recursive-alias guard prevents a receiver proof [generic]
```

`FlowCacheLookup::evaluated_receiver` distinguishes an unconfigured lookup from a failed configured proof. `member_applicability::expand` delegates the complete receiver to the selected view's existing structural evaluator, preserving source generic IDs, default/constraint evidence, context checks and bounded recursion. This also covers aliases reached through properties, call results and optional receivers. Legacy expansion keeps its existing path. No grammar, profile or hook changes were needed; the binding epoch is 73 and extractor schema remains 83.

The pinned TypeScript 5.9.3 verifier covers 24 cases in script and module scopes: 88 exact declarations, selected signatures and result labels, including retained unsupported cases and five diagnostic negatives. Fourteen supported cases produce 56 exact engine call checks per fresh/cold pass. Portable parse caches, poisoned symbol/chain display names, provider switches, provider/consumer edits, deletion, stale source and foreign context are covered. Union/any distribution, an unbound checked parameter, unknown comparisons and callable conditional operands remain conservative barriers. The existing intrinsic fixture now passes all eight compiler call checks, closing the four formerly retained misses.

Validation: 2,157 selected library tests passed, with zero failures and 32 ignored. All seven integration tests passed across the TS/JS/Rust resolution corpora, per-file manifest and per-package context suites. Production file-budget and string-identity audits found zero violations across 352 changed production files; whitespace checks passed.

The immutable [configured report](../resolution-documents/2026-09-09-query-core-conditional-receivers-program-report.json) retains 585 correct, 40 declaration-kind disagreements and 65 unresolved labels out of 690. The [legacy report](../resolution-documents/2026-09-09-query-core-conditional-receivers-legacy-report.json) retains 557 correct, 40 kind disagreements and 93 unresolved. Both fresh/cold views agree exactly. Deep comparison with the object-initializer checkpoint finds all 690 observations unchanged in each mode and whole reports equal except elapsed time (49,717 ms configured; 48,179 ms legacy). The [comparison artifact](../resolution-documents/2026-09-09-query-core-conditional-receivers-comparison.json) records input hashes. The frozen manifest is unchanged; this task closes fixture-backed behavior without claiming an additional real-cohort recall gain.
