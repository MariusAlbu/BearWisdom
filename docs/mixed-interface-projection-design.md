# Mixed interface inheritance and receiver-specific members

1. This is generic engine work in `program_interface_heritage`, configured views and member consumers; no language/API-name hook is introduced.
2. A canonical mixed base is not a nominal head. Recursively prove every nominal and structural branch before publishing the child.
3. Preserve nominal base applications for ancestry, but preserve the complete intersection separately as the child's base type; never silently drop a structural operand.
4. Intersections combine property values and effective optional/readonly modifiers; separate extends clauses still require compatible inherited members. Keep assignment surfaces distinct from declaration identity.
5. Reuse source-owned member facts, including complete callable signatures and row-independent source/signature origins. Do not reconstruct navigation by spelling.
6. Canonical key TypeIds map to MemberNameIds only at the existing source ingestion boundary. Structural additions without an attested origin remain explicit barriers until structural-origin capture is supplied.
7. Store proved effective member information by receiver-owner and member-declaration IDs, independently of shared physical-row TypeInfo. Generic environments belong to the receiver after projection.
8. Route bound calls, callback patterns and member yields through that receiver-specific information; retain authoritative missing/ambiguous/foreign-context results.
9. Start with compiler-backed legal and rejected inheritance cases, required/readonly/generic refinements, callable preservation, same-parent different children, portable/cold inputs and provider changes.
10. Re-evaluate the unchanged real Query Core population and both retained regressions. Mixed projection is not completion of initializer inference, full type semantics or the multilingual/product gates.

## Implemented semantics

```ts
interface Base<T> { readonly value?: T; read<U extends T>(x: U, next?: U): U }
type RequiredValue<T> = Base<T> & { value: {} };
interface Child<T extends object> extends RequiredValue<T> {}
// ✅ [generic] Mixed proof retains both operands and the original member source/signature IDs.
// ✅ [generic] Child.value is required and mutable; the parent and sibling keep their own modifiers.
// ✅ [generic] Child<T> member reads and indexed type queries use the child-owned generic environment.
// ✅ [generic] The method keeps constraints, defaults, optional/rest flags and navigation origins.
type Added = Base<string> & { extra: number };
// ⚠️ [generic] A newly introduced structural key still needs source-origin capture before heritage admission.
```

`program_mixed_heritage.rs` splits complete canonical intersections into nominal applications and explicit structural refinements. Nominal applications use the existing source-bound constraint/default and inherited-surface proofs. Only property/index groups with sufficient evidence are intersected; unmodified callable groups retain their full source signatures and all alternatives. Separate extends clauses still require compatible members. Conflicting required discriminants reject the whole base; an explicitly never-valued property does not by itself collapse its owner.

`View` stores receiver/member-to-slot indices and effective surfaces separately from physical declaration `TypeInfo`. The surfaces preserve row-independent `SourceInstanceId`/`SignatureId` origins and supply the nominal key/index inventory after admission. Bidirectional key-TypeId/member-name-ID maps are populated at source ingestion, not decoded during resolution. Bound calls, callback patterns, field projection and chain yields read the receiver-specific information. Failed or ambiguous navigation does not invent a physical row.

The binding epoch is 54; source extraction remains schema 71. The existing public consumer compilation caveat for AlphaT/Lynx remains unresolved; no sibling lockfiles were changed.

## Verification and remaining boundaries

The first compiler-backed mixed generic inheritance test failed before implementation. TypeScript 5.9.3 independently verifies 26 heritage cases: eight legal admissions, sixteen diagnostic-backed rejections and two legal unsupported cases. The original 41 assignment, 11 obligation and 25 merge compiler controls remain unchanged and pass. Source tests cover sibling isolation, exact generic return substitution, provider retarget/edit/deletion without consumer recapture, filtered portable caches, poisoned names/signatures, cold reload, rowless origins, query/read consistency and budget exhaustion without poisoned negative memo entries.

Final selected library verification after the member-consumer changes: 2,066 passed, zero failures, 31 ignored, 5,623 filtered; 28.19 seconds. This is the selected TS/JS/framework, core-type, lexical, resolution, flow, contracts/cache and oracle scope, not a workspace-wide claim.

The first real-project report (`2026-09-08-query-core-mixed-heritage-program-report.json`, SHA-256 `6ead186181985437ddd8864014e9fa44770a16037573ade5f3f3ec6aab43bdc2`) improved 546 to 548 correct of 690 labels. Exact comparison identified only the retained `queryObserver.ts` calls at bytes 10706 and 15174: both now match compiler targets in `types.ts` at lines 341 and 415, respectively. Every other labelled occurrence was unchanged fresh and cold. This interim report precedes the final modifier/budget/query-consistency changes; elapsed 44,883 ms is not controlled performance evidence.

Still open: source origins for new/remapped structural keys, complete callable-property refinements and structural call/construct syntax, union/distributive inference and exact numeric/string key equivalence. Repeated-property query dependencies are not yet fed back into private value-query staging; initializer-derived fields remain the next retained cascade. Full IDE APIs, representative multilingual 99%/99.9% evidence, AI benchmarks and cross-service flows are not completed by this change.

## Final configured measurement

After all production changes, `2026-09-08-query-core-mixed-heritage-verified-program-report.json` matches the interim report in its entirety after removing only `elapsed_ms`. Every original expected label is unchanged. Both fresh and cold contain 548 correct, 40 declaration-kind-only disagreements and 102 unresolved: 79.4202898551% recall and 93.1972789116% strict precision. Only the two retained inherited regressions changed from unresolved to correct; zero source-input gaps or snapshot differences. The 99% gate remains false.

Final report SHA-256: `650fd96f4a10d320043c485446d50ac74824589f9d20ac17a2d2322ca5d96f76`; elapsed 40,818 ms, not benchmark evidence. The original 187-source / 24-selected-file / 843-call / 690-label compiler manifest is unchanged. Seven selected integrations passed (`resolution_corpus`, `resolution_corpus_js`, `resolution_corpus_rust`, `per_file_manifest`, `per_package_context`). Native audits found zero file-budget violations in 317 changed production sources and zero added ID-discipline patterns in 118 scoped production sources; `git diff --check` passed.

The final legacy report (`2026-09-08-query-core-mixed-heritage-legacy-report.json`) is identical to the preceding nominal/conditional legacy report after removing only `elapsed_ms`: 557 correct, 40 kind-only disagreements, 93 unresolved, fresh/cold equal, gate false. SHA-256 `14dee68cb51d257f637955cee1ab0852512abb99b64f678d76f0570876be7c6e`; elapsed 49,085 ms. Both oracle processes completed; the final process check found no Cargo, rustc or project_oracle process.
