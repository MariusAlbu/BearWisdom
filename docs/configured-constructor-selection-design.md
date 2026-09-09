1. Preserve each constructor's canonical owner, source instance, signature span and optional navigation row.
2. Keep implicit constructors without a fabricated source signature; inherited explicit constructors retain the base origin.
3. Share source-group ordering and literal-specialization rules with method overload dispatch.
4. Use compiler-attested source order for merged providers, never physical rows or collection iteration order.
5. Share subtype-before-assignable selection and stop when earlier evidence is incomplete.
6. Without order, select only a unique proved candidate; equal results may establish a type without establishing a winner.
7. Publish constructor evidence with the source initializer result, invalidating it with failed, cyclic or exhausted evaluation.
8. Rebuild the evidence from portable source inputs in each configured view and cold snapshot.
9. Compare independent TypeScript labels for overloads, inheritance, defaults, access, merged order and downstream calls.
10. Verify poisoned metadata, provider/configuration edits, deletion, stale inputs and the retained difference<T> cascade before closing its roadmap parents.

Constructor candidates now preserve canonical owners and source signature origins.
Methods and constructors share both ordering facts and dispatch phases. A missing
navigation row does not erase the source signature; an implicit constructor has
no fabricated origin. The source view publishes constructor evidence only with
its validated initializer result and rebuilds that evidence during cold loading.

Merged inherited constructors need the order of the providers declaring their
bases, even when all constructor signatures reside in one other source. Missing
or revoked order therefore permits only a unique applicable candidate or a
consensus result without a selected signature. Ordered selection stops at the
first applicable candidate; incomplete earlier arguments block selection, while
an unsupported later rest parameter does not invalidate an earlier winner.

The pinned TypeScript 5.9.3 constructor-selection verifier covers 23 fixtures:
20 exact result types, 18 source signature origins, two implicit origins and 21
downstream call origins. Three invalid access/abstract cases have compiler
diagnostics. Two valid cases remain explicit engine abstentions: an earlier
variadic-array-rest competitor and contravariant constructor-interface heritage.
The latter is rejected by existing inheritance compatibility checks before
dispatch; this change does not weaken that proof. General variadics, callable
properties, Promise cascades and caller-sensitive constructor access remain open.

The real retained source probe uses the independently captured compiler binding
order and selects the iterable Set constructor at source file 9, byte 6641.
It also verifies the filter and has source declarations and reproduces the
constructor origin after a cold reload. Compiler signature end offsets include
the trailing semicolon, whereas captured member spans exclude it.

The derived constructor-order manifest adds only the independently attested
source order to the frozen compiler manifest. Its SHA-256 is
038db174914092dd9d2296d457690c1db8ee2c4b68aff95cfca3359607878d3b.
The original manifest and supplemental compiler labels remain byte-identical.

Verification covers 2,150 selected library tests and seven integration tests.
The broader run passed 2,149 tests and found one historical legal-abstention
expectation that this feature now supports. The independent initializer verifier
confirmed its Left result; the entire constructor result/barrier matrix then
passed with the updated expectation. The 54-case initializer verifier checks
39 exact types/signatures, and the shared 47-case method-overload verifier checks
41 exact selected signatures and result types. No production code changed after
the broader run. A native Windows audit checked 347 changed production files
with zero file-budget violations, and `git diff --check` passed.

The final frozen report is
`resolution-documents/2026-09-09-query-core-constructor-selection-program-report.json`
(SHA-256 791b76e06f8e61486184c84b8b44b0e3db86583882b5fa9fea7b46301f24d9ca):
581 correct calls, 40 kind disagreements, 69 unresolved, zero snapshot changes.
Its entire contents equal the preceding local-initializer report except elapsed
time. The ordered report is
`resolution-documents/2026-09-09-query-core-constructor-order-program-report.json`
(SHA-256 ea9f494a62e06adec22916fb9fab187e8b4fd1295971b249a1abbc93e6f4ef87).
All 690 fresh and cold observations match the frozen run; only elapsed time and
the expected manifest SHA-256 revision differ. The reports remain ineligible
for the full correctness gate.

The final legacy report is
`resolution-documents/2026-09-09-query-core-constructor-selection-legacy-report.json`
(SHA-256 70f12c6130e71481a9d366c40872fce8f7751225874fdda93251afc0872da366).
It exactly matches the preceding legacy report except elapsed time: 557 correct,
40 kind disagreements, 93 unresolved and zero snapshot changes. This completes
the retained difference<T> constructor/filter/has cascade, including its generic
augmentation and compiler-intrinsic dependencies. Broader roadmap work stays open.
