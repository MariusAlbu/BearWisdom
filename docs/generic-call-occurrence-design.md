1. [evidence] The source-authoritative bare-call barrier exposed a legal explicit-generic factory cascade failing before its member receiver can be typed.
2. [root cause] Rust's legacy call-chain builder has no callable-wrapper case; `make::<T>` becomes a display-text fragment and can lose the inner call or the entire receiver chain.
3. [refactor] Extract the existing chain builder unchanged into a sibling `calls_chain.rs`, shrinking the oversized `calls.rs`; preserve existing cast/index/qualified-chain behavior.
4. [profile data] Supply generic call head and argument fields through the namespace type syntax profile; the extractor consumes those forms rather than adding a semantic language hook.
5. [ingestion] Retain a one-segment structured chain for explicit generic calls and unwrap generic heads structurally when building nested chains. Display argument text stays at extraction boundaries.
6. [generic] Capture explicit call argument type recipes at the exact final selector; materialize them with the existing source TypeBinder into TypeIds, including distinct same-spelling declaration heads.
7. [generic] Attach those TypeIds to lexical and namespace call facts so existing callee-ID generic substitution can consume them without display-text reparsing.
8. [tests] Cover bare, qualified and member generic syntax; zero-argument return substitution, unknown callee non-interference and imported factory initializer cascades must survive fresh/cold snapshots.
9. [evidence] Add independently authored positive/negative source-marker cases and validate them with rustc before engine scoring; preserve unresolved cases and historical false-positive snapshots.
10. [limits] This does not add overload selection, trait dispatch, explicit const-value semantics, full receiver adjustment or representative corpus coverage. Keep the full F0-F3 goal open.

Verified: the retained generic imported-factory direct/local cascade now passes;
the independently rustc-checked zero-argument fixture resolves all six bare,
qualified and member-call occurrences to the expected physical declaration IDs,
including distinct `a::Doc` and `b::Doc` result members. The source recipe and
fresh-arena cold snapshots agree. The full selected core/profile run passes 2,083
tests (25 ignored), with all 12 focused integrations green. The Rust compiler
cohort verifies 47 positive labels and five diagnostic-backed negatives, not
representative language completeness. Existing TypeScript label checks pass.
