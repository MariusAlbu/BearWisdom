1. [generic] Separate name interning, owner/member candidate retrieval, and selection; a member spelling is not declaration identity.
2. [generic] Intern member names at structural ingestion and index candidates by (owner declaration ID, member-name ID).
3. [generic] Preserve original member declaration IDs, canonical owner folding, late ingestion and cold database restoration.
4. [generic] Expose ID-only named-member candidates through SymbolLookup and its per-file overlay; unknown names do not trigger global/qname fallback.
5. [generic] Move nominal member traversal into a bounded selector that carries a single name ID across the superclass graph.
6. [generic] Collect candidates at a lookup level and require declaration-ID agreement; distinct declarations are not first-wins matches.
7. [generic] Preserve direct-member hiding and diamond identity deduplication, and block inherited fallback behind conflicting direct declarations.
8. [generic] Keep overload selection, language-specific inheritance precedence and access filtering explicit open requirements, not guessed winners.
9. [generic] Add failing duplicate/sibling-supertype tests, then verify source constructor/return cascades, merged-owner identity, late/cold indexes and negative namesakes.
10. [generic] Check downstream API consumers and existing semantic gates; this step does not claim full removal of legacy string bridges or complete member privacy.

## Implemented and verified (2026-09-07)

`MemberIndex` stores interned member-name IDs and `(owner ID, name ID)` candidate rows. Structural ingestion records entries, canonical owner folding rebuilds the index, and cold database ingestion reconstructs it from containing IDs. The `SymbolLookup::member_index` read-only view exposes borrowed declaration-ID slices, so traversal does not allocate symbol candidate lists or scan unrelated member spellings. Name IDs remain snapshot-local, not durable declaration identities.

`member_selection` keeps Missing/Unique/Ambiguous/Incomplete separate, uses bounded ID traversal and requires candidate-ID agreement at the selected lookup depth. Direct conflicts cannot reveal an inherited candidate; diamonds deduplicate declaration identity. The old `lookup_member_by_id` first-winner regression was observed before implementation. Constructor/factory conflicts now cannot seed fabricated downstream local types, fresh or cold.

The optional-singleton `SymbolSet` conversion now owns construction of borrowed zero/one-element candidate sets; the general lookup contract delegates to it. No formatting or comment trimming was used to meet module budgets.

1,944 selected core tests passed (18 ignored). The two optional Rust compiler checks were explicitly rerun: all 19 existing acceptance/rejection fixtures pass. TypeScript 5.9.3 reverified 213 existing call labels (79 module, 134 scope). Windows-native budget/ID audit: 117 changed production files, no violations; diff whitespace check passed.

Consumer checks were attempted with `--locked --offline` and a target directory inside BearWisdom. AlphaT stopped in dependency resolution at yanked `der 0.8.0` via ureq/ort; Lynx requires a lockfile update. Neither reached code compilation; no consumer lockfile was changed. These failures do not establish consumer API compatibility.

Still open: source-occurrence member IDs at every API boundary, member/field access, real overload selection, language-specific inheritance precedence, outer fallback propagation of ambiguity and legacy unbound/name-based paths. No new overall recall, precision, speed or token-efficiency claim is justified by these targeted tests.

All 12 selected integration tests passed: incremental indexing (5), per-file manifest context (2), per-package context (2), and TS/JS/Rust resolution corpora (1 each).
