# Occurrence correctness oracle (F0)

This is an independent **labelled-fixture correctness instrument**, not a new
resolver and not evidence that BearWisdom has reached 99% on real projects.
Ground truth lives in manually authored source markers and explicit numeric
reference-to-declaration pairs in
[corpus_tests.rs](../crates/bearwisdom/src/resolution_oracle/corpus_tests.rs).
The engine never supplies the expected target.

## Identity and evidence

- A fixture manifest assigns numeric file IDs, unrelated to SQLite allocation.
- Reference identity is (fixture file ID, zero-based UTF-8 byte offset, edge kind).
- Declaration identity is (fixture file ID, zero-based line/byte column, symbol kind).
- A SHA-256 revision pins source contents, file order/paths, language, call cohort
  and independently authored labels. Changed fixtures require explicit rebaselining.
- Names, qualified names, confidence and strategies do not determine equality.
  Strings are decoded only at source-marker, manifest and database boundaries.
- These source addresses are a separate, revision-local oracle identity domain.
  They are not compiler BindingIds/SymbolIds or cross-edit persistent identities.

The production resolution log now also persists the extractor's byte offset.
Existing rows acquire a nullable column: NULL means unavailable evidence; zero
remains a valid offset. Opening a writable legacy index migrates the schema but
does not invent positions. **Reindex old files before oracle evaluation.**
Existing snapshot/IDE APIs are unchanged; their legacy string keys and graph-edge
deduplication are not fixed by this addition.

The adapter receives unique extracted source sites from the *same source snapshot*
as the index, captured before resolution. Without this input, absence from the log
cannot distinguish failed extraction from a resolver early exit. The fixture
runner uses the production parser, writer, Compilation, inference prelude and
resolver, with one shared TypeArena and no external dependency discovery.

Duplicate extractor emissions of one source site are collapsed for fixture
scoring; the separate raw occurrence census still counts them. Multiple surviving
log outcomes at one site, co-located target declarations, missing manifest targets
and legacy byte offsets are errors, not name-matching opportunities. Nested
references sharing the same start/kind need richer span/occurrence identity before
joining this cohort. Synthetic/merged targets likewise need explicit oracle
identity support rather than relaxed coordinate matching.

## Scoring

Each labelled reference is exactly one of:

| Verdict | Meaning |
|---|---|
| correct | Bound to the independently labelled declaration |
| incorrect | Wrong target, false-positive binding on a negative, or dangling target |
| correct_unbound | Explicit negative extracted and recorded unresolved/drained |
| unresolved | Positive reference extracted and recorded unresolved/drained |
| not_extracted | Labelled source site absent from extraction input |
| missing_resolution | Extracted site with no recorded resolution outcome |

Binding precision = correct / (correct + incorrect).
Correct-binding recall = correct / expected-bound labels.
Extraction coverage = extracted labelled sites / all labelled sites.
Empty denominators produce null. Negatives never inflate positive binding recall;
unlabelled observations are reported separately and never treated as correct.

The source-level snapshot comparator detects per-site retargeting even if totals
are unchanged. Improvements elsewhere cannot cancel regressions. The fixture
gate additionally blocks wrong-to-wrong retargets pending review.

## Initial fixture observations — 2026-09-06

These are **11 call sites in four tiny, manually labelled fixtures**, not
representative language baselines or a compiler-validated/held-out corpus.

| Fixture | Positive labels | Correct | Wrong | Correct negatives | Precision | Correct-binding recall |
|---|---:|---:|---:|---:|---:|---:|
| TypeScript sibling parameters | 4 | 2 | 2 | 0 | 50% | 50% |
| JavaScript same-line calls + missing name | 2 | 2 | 0 | 1 | 100% | 100% |
| Python same-line calls | 2 | 2 | 0 | 0 | 100% | 100% |
| C# same-line calls | 2 | 2 | 0 | 0 | 100% | 100% |

All 11 labelled sites were extracted and received outcomes. The TS fixture has
100% binding coverage despite only 50% correct bindings:

    class Alpha { save(): void {} }
    class Beta  { save(): void {} }
    function first(value: Alpha) {
      value.save(); // ❌ [generic] FileLookup: bound to Beta.save, not Alpha.save.
      value.save(); // ❌ Same upstream binding collision affects both calls.
    }
    function second(value: Beta) {
      value.save(); // ✅ [generic] Target happens to match the file-flat cache.
      value.save(); // ✅ Same declaration.
    }

At the initial snapshot, the declared-local seeding loop in engine/pipeline.rs
fed a name-keyed FileLookup cache and set_cursor was a no-op. A later sibling's
parameter overwrote an earlier sibling's type. Adding TypeIds to the values did
not provide lexical identity for the keys. The first F1 implementation now fixes
this TS fixture: all four calls bind correctly, without changing its labels or
historical snapshot. See [scoped-bindings.md](scoped-bindings.md) for scope and
remaining legacy paths.

[corpus_snapshot.json](../crates/bearwisdom/src/resolution_oracle/corpus_snapshot.json)
records observed behavior and pinned revisions, **not ground truth**. Known wrong
bindings remain scored incorrect. Tests permit correction or honest abstention,
while protecting previously correct sites. Do not change independent labels to
make the snapshot pass.

## Run and extend

    cargo test -p bearwisdom --lib -- resolution_oracle:: query::oracle_evidence:: db::resolution_schema:: --test-threads=4 --nocapture

Place declaration markers at the declaration node start and call markers at the
call expression start. Markers are removed before parsing, including in Python.
Author expected marker-ID pairs first. Review language semantics independently.
Add explicit negative/shadowing cases as well as positive ones; preserve fixture
revisions and review changed snapshots per occurrence.

Next: expand scope, closure, declaration-order, import and module-identity
fixtures while implementing scoped bindings. Representative per-language
baselines, compiler-backed label validation, incremental semantic equivalence,
and the independent 99%/99.9% gate remain open. No real-project recapture, LLM
speed/token benchmark, or resolution-rate improvement is claimed here.

## Compiler-checked scope cohort — F1, 2026-09-06

[scope_tests.rs](../crates/bearwisdom/src/resolution_oracle/scope_tests.rs) adds
66 TS fixtures with 134 labelled calls: 125 positive bindings and nine negatives.
They run through the same production fixture adapter. All labels match the
installed TypeScript 5.9.3 compiler's declaration targets, checked independently
by [verify_typescript.mjs](../crates/bearwisdom/src/resolution_oracle/verify_typescript.mjs).
The verifier compares declaration byte addresses, not symbol names, and never
rewrites fixtures or snapshots.

    node crates/bearwisdom/src/resolution_oracle/verify_typescript.mjs <path-to-installed-typescript-module>

The optional argument defaults to module resolution for `typescript`. No install
is performed. This is a small compiler-checked navigation cohort, including
diagnostic-bearing source, not a general language-service differential harness.
All 134 labels pass the engine oracle; this remains far too small to infer a
project-wide accuracy rate.

### Called-identifier anchors (call-selectors-v2)

The scope cohort now places reference markers at the called identifier:
`value./*@ref:11*/save()`. Production evidence persists `source_selector_byte`
alongside the unchanged expression-start `source_byte`. This separates calls
like `start.go(...).go(...)` that share an expression start but not a selector.
Its duplicate guard now retains both TS/JS occurrences by numeric positions.

`fixture_support::run_selectors` and `query::oracle_evidence::selector_observations`
use the new anchor domain. All scope-cohort marker moves are an explicit
identity-schema change; expected declaration targets are unchanged and rechecked
against the compiler. The original four-language `calls-v1` fixture snapshots and
strict expression-start adapter are untouched.

Both adapters reject ambiguous locations. Missing selector evidence stays NULL
on legacy rows and requires reindexing. Selectors are not full expression spans
and do not yet disambiguate every indirect/dynamic form. Nothing is resolved or
matched by target spelling to hide missing evidence.

### Bare-reference identity follow-up

The added cases independently label local parameter/variable declarations,
including same-line siblings, closure capture, untyped parameters, block/TDZ
shadowing, callable return propagation and destructured function bindings.
These are declaration-reference labels, not runtime-dispatch labels.
`ref_resolutions` records the lexical declaration, while a separately captured
callable ID can preserve the existing graph edge to a known implementation.

### Named declaration and generic-root follow-up

Nine further fixtures add 18 labels for block function shadowing, hoisted returns,
sibling local constructors/static members, callable-parameter roots, namespace
shadow rejection, constructor callback type arguments, explicit generic function
returns and wrapper return propagation. The compiler check still compares exact
source declaration addresses; no labels were learned from engine outcomes.

The wrapper fixture exposed duplicate extraction under both the enclosing and
nested function. Fixing ownership in the TS call visitor makes the evidence
unambiguous; the oracle still rejects ambiguous logs rather than choosing by name.
Bare constructor references now anchor `source_byte` at the `new` expression and
their selector at the constructor identifier. Existing constructor evidence must
be reindexed; historical four-language call snapshots remain untouched.

### Private named-expression follow-up

Eight additional fixtures add 18 labels for private recursion, parameter shadowing,
outer variable navigation, callable returns, class self/static/constructor members,
generic return and constructor callback cascades, and two negative scope escapes.
All labels are independently compiler-checked source addresses. No engine target
was substituted for an expected declaration when a case initially failed.

The fixture writer now checks that lexical-only visibility rows exactly match
parsed BindingId-to-symbol-slot identities. Separate storage/engine tests verify
visibility after cold reload, same-qname row and return-metadata preservation,
survivor visibility replacement and deletion. Those checks do not turn this
small navigation cohort into a general incremental semantic differential harness.

### Type-space follow-up

Ten more fixtures contribute 19 compiler-checked labels for same-named local
type annotations and explicit arguments, private class self-types, independent
value/type namespaces, field/return cascades, callable/applied type shapes,
interface/alias chains, canonical generic substitution and an unconstrained
parameter shadowing an outer nominal type. The negative remains a negative;
it is not counted as a successful positive binding.

Separate tests exercise canonical metadata reload with the saved TypeArena,
parameter owner identity, default/alias preservation, fresh-parse precedence,
deleted-row exclusion and missing-row abstention. The original four-language
fixture snapshots remain unchanged. Full semantic incremental equivalence and
representative multilingual language-service comparisons remain open.

### Member-call and contextual-signature follow-up

Ten further fixtures add 20 labels for member-call type arguments, same-named
local types in callback signatures, explicit and inferred callback arguments,
explicit-argument priority, repeated generic member hops, and class/method
generic parameters sharing a spelling. The new negative rejects borrowing the
class's concrete type for an unconstrained method parameter of the same name.
The initial six fixtures all failed; one exposed two confidently incorrect
targets. Their expected declaration addresses were independently compiler-checked
before implementation and were not changed to fit the engine.

Parameter lists now persist as canonical TypeIds on the callee's ID-keyed TypeInfo.
Reload tests preserve nested callback types and GenericParamIds, use the canonical
list without a signature string, reject stale metadata over fresh parsed IDs,
and retain explicit legacy absence for older snapshots. This is not a complete
overload, argument-expression or incremental semantic-equivalence proof.

### Source-addressed argument follow-up

Eleven fixtures add 19 compiler-checked labels for class/function arguments,
private callable provenance, parenthesized/awaited arguments, recursive arrays
and ternaries, sibling callback contexts, missing-type non-interference and
higher-order member calls, initializer ownership and nested-call rejection.
The first four fixtures all failed before the change.
Source-addressed argument leaves now contain a SourceSpan without any identifier
string. Separate tests assert exact spans, recursive traversal, serialization,
cursor independence, and position-correct type/callable facts.

Array inference exposed a verifier limitation: `noLib: true` removed TypeScript's
standard array types. The verifier now reads the installed compiler's real
ESNext standard-library declarations; no packages are installed or downloaded.
All existing and new labels still match. An external declaration is represented
with its file identity rather than compared as a fixture-relative byte offset.

The higher-order member fixture exposed wrapper inference overwriting a captured
source return with Unknown when the legacy extractor return slot was absent.
The source-recipe-by-declaration guard fixes both downstream calls in that fixture.
Neither the expected targets nor the original four-language snapshots changed.

The initializer fixtures exposed two global-lookup passes and a missing field
initializer marker. Both now consume the file-local ID environment and the
actual source-owned call, including its arguments. A separate pass-isolation
test caught call attribution to an enclosing function; the join now uses the
captured declaration slot and exact expression/selector addresses. Two negative
labels reject treating calls inside arrays or closures as the whole field value.
The negative fixture initially produced two incorrect bindings, both now absent.

Public consumers were checked with offline, locked Cargo invocations. AlphaT
and Lynx both stopped before compilation because their lockfiles need updates;
neither lockfile was modified. Their compilation is not established by this run.

### Module/export identity follow-up

The separate multi-file cohort in
[module_fixtures.json](../crates/bearwisdom/src/resolution_oracle/module_fixtures.json)
adds 13 cases and 28 compiler-checked labels: 23 positive and five deliberately
unbound sites. Its first eight cases all failed before implementation, including
four cases with incorrect targets; expected source addresses were not changed to
fit engine results. The original four-language snapshots remain unchanged.

Run the independent verifier against an already installed TypeScript module:

    node crates/bearwisdom/src/resolution_oracle/verify_modules.mjs <path-to-installed-typescript-module>

TypeScript 5.9.3 validates all 28 labels, and the existing verifier still validates
66 cases / 134 labels. These are two targeted TypeScript cohorts, not a
representative multilingual corpus or a measured 99% resolution rate.

```ts
import { Model as Left } from "./left";
import { Model as Right } from "./right";
// ✅ [profile data] ES import syntax creates local BindingIds during ingestion.
function use(a: Left, b: Right) {
  a.save(); // ✅ [generic] BindingId → export graph → left's declaration ID.
  b.save(); // ✅ [generic] Same spelling cannot replace right's declaration ID.
}
export { Model as Item } from "./left";
// ✅ [generic] Aliased re-exports retain the original declaration identity.
export * from "./other";
// ✅ [generic] Cycles/diamonds terminate; explicit exports win; default is excluded.
// ⚠️ [generic] Conflicts and missing module evidence do not select a target.
```

Cases also exercise imported class/function arguments, exported aliases and
callback signatures, local value shadows, type-only annotations, named defaults,
exact relative-path selection, `.js`→`.ts` substitution, declaration-file function
and value exports, and wrapper/direct-chain yields from imported overload groups.
Negatives protect private exports, missing paths, namespace export leakage,
default-through-wildcard leakage and broken explicit re-export fallthrough.

Separate engine/storage tests verify late dependency ingestion, cold reload with
a restored TypeArena, deletion, source-fingerprint checks for symbol-less barrels,
fresh-parse precedence, retained package-entry evidence, and changed-barrel
retargeting of unchanged signatures. A failing-first generic return test exposed
templates captured before module rebinding; both the canonical return and its
generic template now follow the new declaration after reload.

External contract filtering previously discarded all lexical metadata after
changing symbol slots; cache payloads omit it too. Declaration-only restoration
now rebuilds source bindings against the final rows for migrated TS/JS profiles.
Imported wrappers consume those bound declaration IDs. Overload tests preserve
every declaration alternative across reload, check combined return types, and
assert that a bare call still abstains from choosing a navigation target.
A 1,000-node shared barrel graph test visits each export key once instead of
enumerating exponentially many paths.

Namespace imports/objects, import-equals/CommonJS, escaped module literals,
qualified type heads, complete type/value export rules, package exports conditions,
configuration fingerprints, legal declaration merging, and argument-based overload
selection remain open. Source/signature rebinding tests do not establish full
dependent-edge invalidation or incremental/fresh snapshot equivalence. No market,
token-efficiency or whole-corpus performance claim follows from these fixtures.

### Namespace selection follow-up

Six additional multi-file cases contribute 21 labels: 18 positive and three
unbound sites. The module cohort now contains 19 cases / 49 labels (41 positive,
eight unbound), independently checked by TypeScript 5.9.3; the existing 66-case /
134-label cohort remains unchanged. The first targeted namespace baseline had
three incorrect-target failures and one passing shadow case. In particular,
the right-hand module's return was redirected to the left-hand namesake, and
a private namespace member call borrowed a local function.

```ts
import * as left from "./left";
import * as right from "./right";
left.create().save();  // ✅ [generic] Selector address → BindingId → left export ID.
right.create().save(); // ✅ [generic] Right export/return declaration stays distinct.
function use(value: right.Model) {
  value.save();       // ✅ [generic] Qualified type syntax materializes the same ID.
}
export * as nested from "./right";
// ✅ [generic] Namespace re-exports carry ModuleIds, not invented symbol rows.
// ✅ [generic] Nested namespace paths preserve intermediate ambiguity.
```

`lexical_selections` captures source selectors into detached BindingIds; their
spellings are lowered to ExportNameIds at compilation ingestion. Namespace
prefixes then select a declaration through the numeric graph before ordinary
member walking begins. Named imports of namespace re-exports use the same path.
Type-space selection ignores a value-only parameter shadow; value-space selection
does not. No workspace-wide declaration-name recovery was added.

A failing-first generic fixture verifies explicit/inferred return arguments and
callback parameter seeding through the selected callee's GenericParamIds. The
namespace call adapter reads canonical parameter TypeIds directly, without
consulting same-qualified-name overload candidates. Tests also cover qualified
return rebinding across changed namespace re-exports, generic templates, a
restored TypeArena and deletion. Ordinary barrel traversal remains iterative;
nested qualified queries have cycle/depth checks and share a total work budget.
Repeated numeric paths are interned before completed queries are cached.

One proposed negative used a type-only namespace in a value expression.
TypeScript rejected the negative label: its checker still provided the member's
declaration despite the illegal value use. That assumption was corrected before
implementation and replaced by a private-export negative. The engine's existing
type-only value abstention is tested separately, not counted as compiler parity;
diagnostic-aware navigation of invalid code remains an explicit gap.

This slice does not implement namespace objects passed/stored as ordinary values,
computed selectors, declaration namespaces/augmentation/merging, export-equals or
CommonJS, or full project/package configuration semantics. Member lookup beyond
the bound namespace prefix still includes legacy selector-name adapters. These
fixtures do not establish complete string-free resolution, representative 99%
recall/precision, or full incremental dependent-edge equivalence.

### Scoped declaration merging follow-up (2026-09-07)

Six additional compiler-checked cases add 30 labels (26 positive, four unbound).
The module cohort now has 25 cases / 79 labels, alongside the unchanged
66 cases / 134 labels in the earlier TypeScript cohort. TypeScript 5.9.3 validates
the authored source-addressed labels. Every module fixture now runs through both
fresh resolution and cold DB/type-arena restoration. The original five merge
fixtures failed before implementation; the constructor non-interference case
was added while closing the resulting member-lookup precision gap.

```ts
interface Model { save(): void }
interface Model { reload(): void }
function use(x: Model) {
  x.save();   // ✅ [generic] Scoped type BindingId → attested declaration group.
  x.reload(); // ✅ [generic] The other physical member row remains its own target.
}
// ✅ [profile data] Interface/interface and class/interface kind pairs may merge.
// ✅ [generic] Class value bindings remain separate from their type-space groups.
// ✅ [generic] Both declaration orders, named/namespace imports and re-exports work.
// ✅ [generic] Generic returns/callbacks use canonical owner IDs and ordinals.
// ✅ [generic] A sibling lexical Model cannot lend members after an ID lookup miss.
// ⚠️ [generic] Namespace augmentation, conflicting declarations and full heritage
//             binding still require further semantic work.
```

The grouping policy follows TypeScript's distinct type/value domains and
declaration-group semantics, not same-qualified-name equality.
[TypeScript declaration-merging reference](https://www.typescriptlang.org/docs/handbook/declaration-merging.html).
Compatibility checks here are scoped kind-pair and generic-arity checks, not a
complete implementation of constraint/default/member-conflict diagnostics.
Compiler navigation on invalid declarations remains a separately open policy.

Storage regressions exercise a merged interface becoming two separate lexical
types after an edit, unchanged importer return rebinding, cold restoration and
target-file deletion. Scoped singleton/invalid groups fence legacy name merges;
missing rows cannot attest a partial group. A binder epoch additionally rejects
old module BindingIds even when the source hash matches. Older persisted module
inputs therefore need reindexing to regain current binding metadata.

The ID-bound nominal member walk now stops after an ID lookup miss instead of
trying a same-qname member set or a guessed constructor-interface name. Legacy
name-only receivers, inherited/type-level adapters and selectors are not fully
migrated. No representative 99% correctness or performance claim follows from
these targeted cases.

The strict nominal-ID check exposed missing Rust impl ownership rather than a
reason to restore name fallback. A generic detached-body ingestion binder now
supplies exact local nominal owner slots, with Rust syntax supplied as data:

```rust
Builder::new().build().show(); // ✅ [generic] Parent-ID member tables; exact target fresh/cold.
// ✅ [profile data] Bare local inherent impls, forward declarations and generic wrappers.
// ⚠️ [generic] Import-bearing scopes conservatively abstain until import names are bound.
// ⚠️ [generic] Qualified/imported owners and trait dispatch still need semantic evidence.
AliasDoc::new();              // ❌ [generic] Existing renamed re-export constructor probe remains red.
```

The unchanged Rust integration probe suite reaches 31/32 versus its recorded
30/32 baseline; its overall test still fails. Constructor chaining and nested
SelfProbe improve, but these probes are not an independently compiler-labelled
Rust correctness corpus. See `scoped-merge-design.md` for the bounded contract.

### Detached Rust import names and contract ancestry follow-up (2026-09-07)

The blanket import-bearing-scope abstention described above is superseded by
precise exposed-name capture at ingestion and scoped NameId/BindingId barriers.
Unrelated imports no longer suppress local impl ownership. Function-local impl
methods retain exact nominal/callable ancestry, including forward parent links
through contract filtering and serialized external payloads. External extraction
cache schema 32 prevents reuse of pre-change extraction semantics; existing
indexes still require reindexing to acquire the new metadata.

These are targeted extractor/persistence/engine regressions, not new independent
Rust compiler labels. Rust remains 31/32 with the AliasDoc constructor open;
the independently verified TypeScript cohort remains 213 labels. Imported-target
IDs, namespace domains and crate/module identity are still needed. See
`detached-import-bindings-design.md` for the exact bounds, failing-first evidence
and final verification commands/results.

### Hierarchical module graph prerequisite (2026-09-07)

The shared graph now accepts nested source-module identities and separate
value/type/macro export domains. Tests cover namesake modules, mixed-domain
selectors, ambiguous/uncaptured competitors and fingerprinted graph-input
reloads. TS/JS supplies its wildcard default-name exclusion as syntax-profile
data. Module-binding epoch 3 invalidates older bool-domain recipes.

This is a graph-input extension, not new Rust extraction or compiler-labelled
occurrence coverage. The TS cohorts remain 213 labels, and the existing Rust
AliasDoc failure is not fixed by this prerequisite. See
`hierarchical-module-identity-design.md` for mechanisms and remaining integration.

### Rust scoped module ingestion follow-up (2026-09-07)

Rust inline modules and explicit scoped imports now feed that graph, including
grouped renames, public re-exports, qualified factory calls and exact constructor
return IDs. Fresh/cold and filtered serialized-contract tests verify actual target
IDs; negative cases cover competing imports, missing/private targets, generic
shadowing, cfg barriers and unsupported wildcard-provider shadowing. Binding
epoch 4 supersedes epoch 3; extraction cache schema remains 32.

The unchanged Rust integration checks currently score **30/32**, not 31/32:
ID-preserving returns expose a wrong-owner legacy crate import in nested SelfProbe,
in addition to the still-open AliasDoc constructor. This is not a green integration
result. The next task binds configured crate/module identities instead of erasing
return IDs or adding name fallbacks. See `scoped-module-ingestion-design.md` for
the traced upstream selection, exact limitations and verification. Independent
TypeScript coverage remains 213 labels; no independent Rust labels or global 99%
claim were added.
# Configured Cargo module checkpoint — 2026-09-07

See [configured-module-graph-design.md](configured-module-graph-design.md) for
the precise scope and evidence. Rust's legacy 32-pattern corpus now passes with
new exact declaration-ID assertions for the nested SelfProbe and AliasDoc
constructor/member pairs. The SelfProbe fixture required its missing crate-root
module declaration/re-export; no expected target was weakened. Before that fixture
repair, its old downstream-only assertion passed while the constructor had no edge.
This is additional evidence that pattern hit rates are not binding correctness.

The selected core regression run passed 1,902 tests (16 ignored). Independent TS
labels remain 213 (79 module + 134 scope), verified against TypeScript 5.9.3.
No independent Rust binding-label cohort or representative 99% result is claimed.

### Ordered Rust locals and source call addresses — 2026-09-07

The next F1 slice reproduces and closes the rejected-constructor/local/member
false-positive cascade with an indexed namesake. It also covers ordered shadowing,
simple annotations, bare/imported factories, closure capture and exact row
correlation. Supported call selector addresses now come from CST nodes before
binding capture, not later text-search offset repair. See
[rust-local-binding-design.md](rust-local-binding-design.md) for the failing-first
evidence, static-annotation precedence and precise unsupported boundaries.

The selected core cohort passes **1,914 tests (16 ignored)**. TypeScript 5.9.3
still independently verifies the existing **213 call labels**. Rust generic
type-use and non-call initializer forms remain open; these engineering regressions
do not substitute for a representative compiler-labelled Rust oracle.

### Independent Rust compiler target checkpoint — 2026-09-07

The shared `rust_fixtures.json` now supplies 21 cases, with 47 positive call
targets and five explicitly rejected occurrences. Labels were authored before
engine scoring. `verify_rust.mjs` independently checks positive labels by joining
THIR numeric DefIds to HIR declaration DefIds, physical source spans and kinds;
printed names never participate in that join. Negative labels require the authored
rustc diagnostic code at the marked primary span. A rejected program cannot attest
positive targets. These negatives test valid static binding, not IDE error-recovery
navigation policy or the absence of every possible candidate.

The verifier pins `rustc 1.94.0 (4a4ef493e 2026-03-02)`, first compiles without
bootstrap, then enables crate-scoped `RUSTC_BOOTSTRAP` only in debug-dump child
processes. Temporary fixtures are removed on completion; production resolution has
no rustc dependency. Unsupported compiler formats, expanded macro spans, indirect
calls and non-local target evidence are not silently treated as verified labels.

```powershell
node crates/bearwisdom/src/resolution_oracle/verify_rust.mjs
node --test --test-isolation=none crates/bearwisdom/src/resolution_oracle/rust_compiler_output_tests.mjs crates/bearwisdom/src/resolution_oracle/verify_rust_tests.mjs
cargo test -p bearwisdom --lib resolution_oracle -- --test-threads=1
cargo test -p bearwisdom --lib report_compiler_labelled_rust_cohort -- --ignored --nocapture --test-threads=1
```

The verifier accepts an optional installed rustc executable path; it never installs
a toolchain. Its 11 adapter/adversarial tests include deliberately swapped targets,
wrong diagnostic codes, Unicode byte positions and malformed/ambiguous evidence.

The engine harness includes Cargo configuration in a distinct corpus fingerprint,
preserving earlier unconfigured fingerprints. It scores both fresh and cold
snapshots against unchanged source labels. `rust_snapshot.json` records observed
behavior separately and retains the original wrong binding; a dedicated regression
test requires that wrong binding to remain fixed.

Current diagnostic-cohort result: 41/47 correct positive targets (87.23% recall),
zero wrong targets, all five negatives correctly unbound, and six valid unresolved
calls. All 52 marked sites are extracted, and fresh/cold results agree. The six
misses are three trait/default/qualified-trait calls, two match-payload calls and
one owned-receiver output-region cascade. This small authored cohort is not a
repository-weighted Rust baseline, a held-out corpus, or proof of 99%/99.9%.

The new evidence exposed and closed a source-bound missing-bare-value fallback
into an unrelated same-file function. The fix also required preserving generic
call-chain structure and explicit source-bound type arguments. See
[bare-value-binding-barrier-design.md](bare-value-binding-barrier-design.md) and
[generic-call-occurrence-design.md](generic-call-occurrence-design.md).

## Owned receiver checkpoint — 2026-09-07

The original 21-case diagnostic cohort now binds 42/47 positive targets, with zero
wrong targets and all five negatives correctly unbound. Five positives remain
unresolved: three trait/default/qualified calls and two match-payload calls.
The earlier observed snapshots and all source labels are retained unchanged.

`rust_receiver_fixtures.json` adds six cases / 23 positive targets independently
validated by the pinned rustc adapter before engine scoring. A strict normal test
requires every new target to bind, with fresh/cold equality and no unlabelled
calls. The default Rust verifier and read-only diagnostic include both cohorts.
Combined observed results: 65/70 correct positives, five unresolved, zero incorrect,
five correct negatives, and all 75 marked sites extracted. These authored diagnostic
cases do not establish representative Rust/global recall, precision or a 99% gate.

The adapter suite now has 12 tests. Its separate overridden-trait test verifies
that the static compiler FnDef target is the trait declaration for concrete,
generic and qualified calls; relabelling those targets to the implementation body
must fail. This compiler-contract test is not counted as engine recall. See
[call-site-receiver-design.md](call-site-receiver-design.md) and
[trait-binding-identity-design.md](trait-binding-identity-design.md).

## Trait source-contract checkpoint — 2026-09-07

`rust_trait_fixtures.json` adds 12 cases: 13 independent positive targets and five
diagnostic-backed negative calls. The pinned compiler verified all of them before
engine scoring. Coverage includes overrides, inherent/trait receiver precedence,
cross-file renames, where/supertrait bounds, generic return cascades, anonymous
imports, shadowed trait aliases, missing implementations and unsatisfied obligations.

`rust_trait_snapshot.json` is an explicitly reviewed observation baseline, not target
truth. It retains the discovered wrong mutable-inherent target where Rust selects
the shared-reference trait method. The normal per-site regression gate preserves
negative cases and prohibits new wrong-target retargeting while permitting fixes.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Original Rust | 42/47 | 5 | 0 | 5/5 |
| Owned receivers | 23/23 | 0 | 0 | 0/0 |
| Added traits | 1/13 | 11 | 1 | 5/5 |
| Combined | 66/83 | 16 | 1 | 10/10 |

All 93 labelled sites are extracted, with no unlabelled/missing-resolution entries,
and fresh/cold reports agree. The original 65/70 cohort has not regressed: the larger
denominator intentionally exposes additional gaps. None of these small authored
diagnostic cohorts establishes representative per-language/global 99% correctness.

The source-contract implementation captures and persists ID-based evidence but does
not yet implement ordered trait selection or solve applicability. Verification:
2,104 selected core/profile tests passed (25 ignored), all 12 selected integration
tests passed, and all 12 compiler-adapter tests passed. The pinned compiler verifier
now checks 39 cases, 83 positive targets and ten diagnostic-backed negatives.

## Ordered trait selection checkpoint — 2026-09-07

`rust_trait_selection_fixtures.json` adds six compiler-validated cases: eight positive
targets and one diagnostic-backed negative. They cover implementor Self returns,
satisfied generic impl obligations, owned generic receivers, receiver precedence,
owned-Self output elision and an unsatisfied method-specific where bound.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Original Rust | 44/47 | 3 | 0 | 5/5 |
| Owned receivers | 23/23 | 0 | 0 | 0/0 |
| Traits | 11/13 | 2 | 0 | 5/5 |
| Added selection cases | 8/8 | 0 | 0 | 1/1 |
| Combined | 86/91 | 5 | 0 | 11/11 |

All 102 labelled sites are extracted, without missing-resolution/unlabelled entries;
fresh/cold results agree. The unchanged earlier cohort gained 12 correct positive
targets (66/83 → 78/83), including the corrected inherent/trait precedence wrong target.
Labels and historical snapshots remain unchanged. A new strict per-occurrence gate
protects every corrected site; only five named source sites may remain unresolved,
and none may retarget incorrectly or disappear from extraction.

Remaining sites: `trait_default_and_generic_dispatch` byte 201;
`trait_override_static_declaration_targets` byte 214;
`renamed_cross_file_trait_and_receiver` byte 130 (qualified trait calls);
`match_pattern_payload_provenance` bytes 193 and 231. All are in fixture file 1.
The pinned rustc verifier confirms 45 cases / 91 targets / 11 negatives; all 12 adapter
tests pass. This remains authored diagnostic evidence, not a representative 99% result.

## Qualified trait call checkpoint — 2026-09-07

The exact Self/trait source-ID path closes all three retained qualified-call
misses without changing independent labels or historical snapshots. The strict
historical-site gate now permits only the two match-payload sites unresolved.
Eight new `rust_qualified_fixtures.json` cases verify 18 positive targets and one
diagnostic-backed negative, including return cascades, static functions, generic
arguments, `self` values and explicit trait namesake disambiguation.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Original Rust | 45/47 | 2 | 0 | 5/5 |
| Owned receivers | 23/23 | 0 | 0 | 0/0 |
| Traits | 13/13 | 0 | 0 | 5/5 |
| Ordered selection | 8/8 | 0 | 0 | 1/1 |
| Qualified calls | 18/18 | 0 | 0 | 1/1 |
| Combined | 107/109 | 2 | 0 | 12/12 |

All 121 labelled sites are extracted, without unlabelled or missing-resolution
entries; fresh/cold results agree. The unchanged 45-case cohort improves from
86/91 to 89/91 correct positives. The remaining sites are
`match_pattern_payload_provenance`, fixture file 1, bytes 193 and 231.
The pinned rustc verifier independently confirms 53 cases / 109 positive targets /
12 negatives; all 12 compiler-adapter tests pass. This is authored diagnostic
evidence, not a representative Rust or multi-language 99% result.

## Explicit borrow argument checkpoint — 2026-09-07

`rust_borrow_fixtures.json` adds eight independently compiler-validated cases:
20 positive call targets and one diagnostic-backed negative. Shared/mutable and
nested borrows preserve exact operand identities, receiver-versus-ordinary output
regions, generic/Self parameters and cross-file namesake isolation. The initial
shared-borrow gate reproduced four unresolved sites; both source calls and both
downstream calls now resolve fresh and cold.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Unchanged previous 53 cases | 107/109 | 2 | 0 | 12/12 |
| Added explicit borrows | 20/20 | 0 | 0 | 1/1 |
| Combined 61 cases | 127/129 | 2 | 0 | 13/13 |

All 142 labelled occurrences are extracted, with no unlabelled or missing-resolution
entries; fresh/cold reports agree. The two retained unresolved sites remain
`match_pattern_payload_provenance`, fixture file 1, bytes 193 and 231. No old source
labels or historical snapshots changed. The pinned rustc verifier independently
confirms 129 positive targets and 13 diagnostic-backed negatives across all six
fixture files; all 12 compiler-adapter tests pass. This does not establish a
representative per-language or global 99% result.

## Source-owned call argument checkpoint — 2026-09-07

`rust_argument_fixtures.json` adds eight independently compiler-validated cases
with 28 positive targets and one diagnostic-backed negative. The pre-fix cohort
had six unresolved local-return cascades: bare, inherent-member, associated,
namespace, renamed-import and mutable trait argument paths. Source-owned argument
transport closes all six without consulting the legacy display operands.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Unchanged previous 61 cases | 127/129 | 2 | 0 | 13/13 |
| Added source arguments | 28/28 | 0 | 0 | 1/1 |
| Combined 69 cases | 155/157 | 2 | 0 | 14/14 |

All 171 labelled occurrences are extracted, with no unlabelled or
missing-resolution entries; fresh/cold reports agree. The retained unresolved
sites are still `match_pattern_payload_provenance`, fixture file 1, bytes 193 and
231. All 24 argument/qualified/borrow cases also retain identical reports after
clearing or poisoning the extracted reference and chain-segment display operands.
Separate engine tests assert exact returned TypeIds and source-owned regions,
provider signature retargeting, deletion and filtered/cache/cold behavior.

Pinned rustc independently verifies all seven fixture files: 157 positive targets
and 14 diagnostic-backed negatives. All 12 compiler-adapter tests pass. Source
labels and old observation snapshots are unchanged. This remains an authored
diagnostic cohort, not representative per-language or multi-language 99% evidence.

## Source-owned local value checkpoint — 2026-09-07

`rust_local_value_fixtures.json` adds nine independently compiler-validated cases:
22 positive targets and one diagnostic-backed negative. The pre-fix observation
had 9/22 correct and 13 unresolved positive targets, with no wrong targets.
ID-bound local read/borrow recipes and constrained annotation-region refinement
close all 13 misses, including shared/mutable/nested locals, shadowing, sibling
namesakes, call-produced operands, imported qualified calls and Self values.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Unchanged previous 69 cases | 155/157 | 2 | 0 | 14/14 |
| Added local values | 22/22 | 0 | 0 | 1/1 |
| Combined 78 cases | 177/179 | 2 | 0 | 15/15 |

All 194 labelled occurrences are extracted, with no unlabelled or missing-resolution
entries; fresh/cold reports agree. The two retained misses are still
`match_pattern_payload_provenance`, fixture file 1, bytes 193 and 231. All 33
argument/qualified/borrow/local-value cases retain identical reports with poisoned
stored annotation text and empty/poisoned reference/segment display operands.
Separate engine tests assert exact local reference TypeIds, regions and read
positions; provider signature edits, deletion and filtered/cache/cold paths are
covered without changing independent labels or historical observations.

Pinned rustc verifies all eight fixture files (179 positive targets, 15 negatives);
the 12 compiler-adapter tests pass. This is authored diagnostic evidence, not a
representative per-language or multi-language 99% result.

## Source-owned place expression checkpoint — 2026-09-07

`rust_place_fixtures.json` adds twelve compiler-validated cases: 41 positive
targets and one diagnostic-backed negative. The initial eleven-case cohort had
19/38 correct positives and nineteen unresolved; shared field/tuple/dereference
value recipes close all nineteen. Three direct raw-field controls were added
separately after discovering inconsistent raw-identifier decoding at ingestion.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Unchanged previous 78 cases | 177/179 | 2 | 0 | 15/15 |
| Added place expressions | 41/41 | 0 | 0 | 1/1 |
| Combined 90 cases | 218/220 | 2 | 0 | 16/16 |

All 236 labelled occurrences are extracted, with no unlabelled or missing-resolution
entries; fresh/cold reports agree. The two retained misses remain
`match_pattern_payload_provenance`, file 1, bytes 193 and 231. Forty-five argument,
qualified, borrow, local-value and place cases preserve identical reports under
poisoned annotation/display payloads. Exact TypeId, region, privacy and source
position tests complement actual provider type/visibility edits, field/provider
removal, filtered portable-cache recapture and fresh-arena cold reload.

Pinned rustc verifies all nine fixture files (220 positive targets, 16 negatives);
all twelve compiler-adapter tests pass. The selected core/profile suite passes
2,163 tests and the integration selection passes twelve tests. No independent
labels or historical observations were changed. This remains a diagnostic cohort,
not proof of representative per-language or multi-language 99% correctness.

## Owned enum pattern payload checkpoint — 2026-09-07

`rust_pattern_fixtures.json` adds seven independently compiler-validated cases:
sixteen positive targets and one diagnostic-backed negative. The first new
return/local cascade improved from 0/4 to 4/4. Arm-local binding identity and
declaration-backed payload projection also close both historical
`match_pattern_payload_provenance` misses (file 1, bytes 193 and 231). These sites
now have strict correctness gates; neither independent labels nor old snapshots
were changed.

| Diagnostic cohort | Correct positives | Unresolved positives | Wrong targets | Correct negatives |
|---|---:|---:|---:|---:|
| Previous 90 cases after pattern fix | 220/220 | 0 | 0 | 16/16 |
| Added pattern cases | 16/16 | 0 | 0 | 1/1 |
| Combined 97 cases | 236/236 | 0 | 0 | 17/17 |

All 253 labelled occurrences are extracted with no unlabelled or missing-resolution
entries; fresh/cold results agree. Fifty-two poisoned annotation/display fixture
cases retain identical reports. Exact-ID tests cover generic payload substitution,
guard availability, namesake isolation and unsupported reference/rest/alias-mismatch
barriers. Actual provider payload type/access edits and payload/provider removal
retain correct fresh, filtered-cache and cold-reload behavior.

Pinned rustc verifies all ten fixture files; all twelve compiler-adapter tests,
2,170 selected core/profile tests and twelve integrations pass. A perfect result
on this authored diagnostic cohort does not prove representative language coverage
or the F0 99% gate. Borrowed match ergonomics, full pattern semantics and broader
expression/control-flow coverage remain unfinished.
