# Scoped binding foundation — F1, 2026-09-06

This is the first TS/JS migration slice, not a completed compiler binder.
The original independently labelled sibling-parameter fixture improves from
2/4 to 4/4 correct targets. Its labels and historical snapshot are unchanged.
The current TS scope/callback/declaration cohort has 95 compiler-checked labels
(90 positive, five negative) across 45 fixtures; all pass the production oracle.

## Implemented behavior

```typescript
function first(value: Alpha) {
  value.save(); // ✅ [generic] LexicalCache selects this parameter's BindingId → Alpha.save.
}
function second(value: Beta) {
  value.save(); // ✅ [generic] Distinct BindingId; cannot overwrite first's parameter.
}
function shadow(value: Alpha) {
  { let value: Beta; value.save(); } // ✅ [generic] Inner scope selects Beta.save.
  value.save();                     // ✅ [generic] Scope-parent walk restores Alpha.save.
  const cb = (value: Beta) => {
    value.save();                   // ✅ [generic] Annotated callback owns another BindingId.
  };
}
function unknown(value) {
  value.save(); // ✅ [generic] Root shadow barrier abstains; cannot borrow a sibling's type.
}
```

## Identity and ownership

- `indexer/lexical` allocates distinct snapshot/file-local NameId, ScopeId and
  BindingId domains. Scope lookup walks numeric parents; (ScopeId, NameId)
  selects a BindingId. These IDs are not persistent SQLite SymbolIds and must
  not be mixed across parsed files or revisions.
- `lexical_ingest` decodes CST node kinds, fields and token spelling. It uses
  the existing parsed tree, behind the existing flow-size guard. TS/JS syntax
  kinds and the non-union declared-type policy are profile data in
  `languages/typescript/flow.rs`; no new language hook is introduced.
- `FlowMeta.lexical` retains the graph through resolution. Moving FlowMeta to
  its own module preserves the `types::FlowMeta` public path.
- `LexicalCache` stores TypeId facts by (BindingId, function execution ScopeId)
  and source position. Annotations are interned once at cache construction;
  inferred TypeIds are retained without display-format/reparse conversion.
- `FileLookup::for_file` installs that graph. Original reference slots map to
  binding IDs for writes; the resolver visits references in byte order while
  preserving those slots. Destructuring retains a transitional coordinate-based
  extracted-symbol-slot → BindingId bridge.
- Name-only cache writes are rejected in migrated files. Unknown local types
  cannot fall through to an unrelated global or flat extractor annotation in
  member-root/argument typing. Callable facts carry declaration IDs rather
  than qualified-name strings.
- Cache reset clears inferred types, facts, causes and callable targets;
  immutable syntax declarations remain file-owned.
- Bare local calls are prebound during ingestion: source byte → BindingId →
  exact extracted row slot → persisted SymbolId. `SemanticModel` consumes
  that stored relation without consulting target spelling. `SymbolIds::row_id`
  has no qualified-name fallback, including for skipped/missing rows.
- Migrated flow-query captures use scoped binding IDs to correlate symbol slots.
  Missing parameter/local rows are appended at exact declaration coordinates,
  with full-coordinate parent selection; existing rows and reference slots stay
  intact. Constructor property rows are not repurposed as parameter rows.
- Declaration identity and callable provenance remain distinct: occurrence
  evidence records the local declaration; a known callable ID can still supply
  a call-graph edge and return type. Unknown runtime destinations are not proven
  merely because their parameter/variable reference is bound.

This does **not** mean string-based resolution is gone. Read adapters still
accept extracted token text and intern/lookup NameId at that boundary. Type-head
recovery, parts of the resolver ladder and the inference prelude remain legacy.
Their complete identity migration is explicitly unchecked in F1.

## Compiler evidence changed two initial assumptions

```typescript
function order(value: Alpha) {
  {
    value.save(); // ✅ [generic] BindingId is inner value; annotation selects Beta.save.
    let value: Beta; // [profile data] TDZ diagnostics do not erase navigation identity.
  }
}
let value = makeAlpha(); // Assume an explicitly declared Alpha return type.
value = makeBeta();      // Assume structurally compatible Alpha/Beta.
value.save(); // ✅ [profile data] TS preserves this non-union declared Alpha member target.
```

The TDZ and structural-reassignment expectations were checked with TypeScript
5.9.3, not inferred from the engine's output. The verifier is read-only and
compares source addresses of compiler declarations with manual marker labels.

## Boundaries still open

```typescript
consume(value => value.save()); // ✅ [generic] Resolved callee signature → exact param span
                               // → BindingId → contextual TypeId, for supported inline callbacks.
function invoke(callback) {
  callback(); // ✅ [generic] Source-addressed BindingId → parameter SymbolId.
              // ⚠️ [generic] Runtime implementation can still be unknown.
}
if (condition) value = makeBeta();
value.save(); // ⚠️ [generic] Byte-ordered facts are not CFG dominance/branch joins.
```

Other outstanding work:

- Full function/class/import binding, legal redeclaration and hoisting semantics,
  richer pattern forms, indirect callback contexts, overload correctness and other languages.
- Unmigrated `flow_bindings` still correlates extracted symbols by name/line.
  Bare non-call identifiers and declaration kinds outside this slice still need migration.
- Cause/callable facts do not yet have full closure/position-aware propagation.
- Oversized files and languages without lexical metadata retain the legacy
  cache. TSX/Vue embedding and real-project behavior need dedicated cohorts.
- Module/package/export identity, semantic incremental equivalence and
  occurrence-based Find All References remain separate tasks.

Existing TS/JS corpus and incremental tests pass, but are not substitutes for
independent semantic labels. No real-project rate gain, >=99% result, speed/token
advantage or compiler-equivalent navigation claim is made.

## Verification

- 891 focused library tests passed across the engine, lexical/flow ingestion,
  occurrence census, oracle, schema and affected queries.
- Seven integration tests passed: five incremental tests and the TS and JS
  resolution corpora. These corpora include historical limitations.
- TypeScript 5.9.3 independently verified all 29 scope/callback call labels.
- The CLI resolution-gate test passed; a final 41-test cache/oracle/schema rerun passed.
- Native PowerShell equivalents of the file-budget and ID-discipline gates,
  plus `git diff --check`, passed. Downstream IDE/terminal apps were not built.

### Bare-call follow-up verification

Five new cases failed before the change and pass afterwards: same-line sibling
parameters, captured parameters, untyped shadowing, block/TDZ binding, and
parameter-call return propagation. A sixth verifies that a destructured call's
navigation target is the local declaration, not its implementation.

The current 40 labels were independently checked with TypeScript 5.9.3. Historical
11-site fixture labels/snapshots remain unchanged. Old synthetic flow fixtures now
supply real declaration coordinates instead of zeroed name-matched placeholders;
their RHS selection and type-seeding assertions remain intact.

This follow-up passed 869 focused library tests across engine/flow/lexical
ingestion, positional symbol identity and oracle modules. Native source-budget
and ID-discipline checks pass. The earlier verification section records the
preceding callback/census checkpoint, not a second current-run test count.
The final seven-test integration rerun also passed (TS corpus, JS corpus and
five incremental checks). These are regression checks, not proof of full
incremental semantic equivalence.

### Named declaration and generic-root follow-up

```typescript
function f(callback: () => Alpha) {
  { function callback(): Beta { return new Beta(); }
    callback().save(); // ✅ [generic] Block function ID → return TypeId → Beta.save.
  }
}
const channel = new Channel<Alpha>();
channel.each(value => value.save()); // ✅ [generic] Constructor Decl ID retains applied argument TypeIds.
make<Beta>().save(); // ✅ [generic] Explicit return substitution uses the callee's GenericParamIds.
function wrap() { return makeAlpha(); }
wrap().save(); // ✅ [generic] Inferred return is also written to the exact wrapper declaration ID.
```

TS/JS named function, generator-function and class declarations now enter the
lexical graph through syntax-profile data. Their precise declaration anchors
correlate existing rows; missing function/class rows are not synthesized with
invented signatures. Bare constructors and chain roots consume those IDs.
Known lexical roots cannot fall through either namespace-anchoring path.

Root call type arguments are captured from the CST and interned once per file
cache. Constructors carry `Apply { base: Decl, args: TypeIds }`. Explicit generic
function returns use an ingestion-built template containing owner-specific
`GenericParamId`s; substitution preserves structural types and expands supported
defaults without comparing parameter names. Same-spelled foreign parameters and
bound nominal declarations are not rewritten. Templates are rebuilt after parsed
ingestion, DB rehydration and wrapper-return inference.

The integration suite caught wrapper returns written only to the qualified-name
store. Their ID-keyed inference now groups by exact positional owner ID and
requires TypeId agreement. Source-bound callees do not enter the old name ladder;
unmigrated/imported callees and the compatibility name store remain legacy.
Nested named declarations no longer emit duplicate call evidence under the outer
function's owner. Constructor expression and called-identifier anchors are now
separate (`new` versus the constructor identifier); old evidence needs reindexing.

The added nine fixtures provide 18 additional compiler-checked labels, including
hoisting, sibling classes, callable-parameter chains, a negative namespace-shadow
case, explicit generics, constructor callback cascades and wrapper return propagation.
The focused engine/lexical/flow/oracle/TS-call suite passed 911 tests.
The final seven-test integration rerun passed: five incremental checks and both
TS/JS resolution corpora, including all three initially regressed wrapper cases.
Native source-budget/ID-discipline gates checked 52 changed source files and
`git diff --check` passed. The final wider run (engine/lexical/flow/oracle plus
all TS/JS library tests) passed 1,337 tests with 15 existing ignored tests and no
failures. Five JS flow fixtures required real declaration coordinates instead
of zero-offset placeholders; their binding, await and destructuring expectations
were preserved. No ignored tests were added.

At that checkpoint, still open: named expressions, imports, declaration merging, non-call occurrences,
lexically binding annotation/type-argument heads to declarations, general generic
inference and receiver substitution by GenericParamId, full CFG semantics, wrapper
inference across every return form and snapshot invalidation. Existing member and
argument inference still contains name-keyed bridges. This is not a string-free
resolver or evidence of a project-wide 99% rate.

### Private named-expression follow-up

```typescript
const run = function self<T>(): T {
  self<T>(); // ✅ [generic] Private BindingId → expression Function SymbolId.
  throw 0;
};
const value = run<Alpha>(); // ✅ [generic] Outer Variable ID stays distinct from callable provenance.
value.save();              // ✅ [generic] lexical_value retains instantiated return TypeId on assignment.
self();                   // ✅ [generic] Private declaration is excluded from unscoped candidates.
const C = class Channel<T> { each(cb: (value: T) => void): void {} };
new C<Beta>().each(value => value.save()); // ✅ [generic] Constructor TypeId retains Decl and applied arguments.
```

The syntax profile now lists named function/generator/class expression kinds.
Ingestion allocates a private self-name environment outside the parameter/body
environment; parameters and function-scoped `var` bindings may shadow it.
Expression declarations receive their own extracted rows and call ownership.
The outer variable retains its declaration anchor and links to the initializer
through BindingIds and exact symbol slots, never a qualified-name alias.
Anonymous expressions retain their previous extraction behavior.

`lexical_value` shares ID-based call/constructor yield logic between bare calls
and chain roots. The two new generic fixtures initially failed after assignment,
even when a direct chain worked. Retaining initializer provenance and canonical
type arguments closes both cascades; explicit callable annotations retain priority.

`lexical_only_symbols` persists private declaration IDs. Full and survivor writes
replace that file's metadata; foreign-key deletion removes vanished rows. Loaded
private roots and their containment descendants are filtered from FileLookup's
unscoped candidates, while exact symbol/member-ID lookup stays available. The
oracle also verifies parsed-slot/persisted visibility parity on every fixture.
Existing indexes need reindexing: schema creation cannot reconstruct missing
expression declarations or visibility from old rows.

The cold-reload test exposed a separate identity bug: `ingest_from_db` skipped
every row sharing an already-seen qualified name. It now deduplicates structural
rows by SymbolId and protects freshly parsed type metadata by SymbolId, preserving
distinct same-named DB declarations. Tests pin visibility after reload, same-name
non-interference, distinct return metadata and visibility changes on survivor writes.
Legacy qname maps, inheritance reconstruction and generic metadata rehydration
still need broader migration; this is not full semantic snapshot equivalence.

Eight added fixtures contribute 18 compiler-checked labels (16 positive, two
negative). Initial failures included wrong self-name targets as well as unresolved
member chains; negative tests then caught the newly extracted private names leaking
through global ladders. All 76 current labels match TypeScript 5.9.3 and pass the
production oracle. The wider library run passed 1,371 tests, with 15 existing
ignored tests and no failures. Native source-budget/ID gates checked 61 changed
source files; `git diff --check` passed. No new ignore or expected-target changes.
The final integration rerun also passed all seven checks: five incremental tests
and both TS/JS resolution corpora. Downstream IDE/terminal apps were not built.

Remaining boundaries include anonymous expression identity, all expression/return
forms, function-value signature/context propagation, reassignment/closure-aware
callable provenance, annotation/type-argument head binding, imports, non-call
occurrences, full CFG semantics and other language cohorts. The existing bridges
are not removed by giving these expression roots IDs. No real-project accuracy,
speed, token-efficiency or 99% claim follows from this checkpoint.

### Type-space and canonical metadata follow-up

```typescript
function first() {
  class Model { save(): void {} }
  let value: Model;
  value.save(); // ✅ [generic] Type-space BindingId → exact row ID → Decl TypeId.
  type Identity<T> = T;
  let view: Identity<Model>;
  view.save();  // ✅ [generic] Alias declaration ID + GenericParamId template retain Model's identity.
}
function second() {
  class Model { save(): void {} }
  let value: Model;
  value.save(); // ✅ [generic] Distinct declaration, despite identical spelling.
}
class T { save(): void {} }
function generic<T>(value: T) {
  value.save(); // ✅ [generic] Unconstrained generic ID cannot borrow the outer class's member.
}
```

The lexical graph now has separate `(ScopeId, NameId)` value/type maps.
Class declarations share their binding across both spaces; interfaces and aliases
enter only the type space. Generic-parameter scopes and syntax shapes are profile
data. A value-only shadow does not hide a type name; a generic parameter does.

`lexical_type_syntax` captures structural recipes after declaration discovery.
Bound nominal leaves carry BindingIds, not names. Parameter leaves carry an exact
owner slot and ordinal. `lexical_type_ids` materializes those recipes through
persisted positional SymbolIds and canonical parameter IDs before the reference
loop. Missing/ambiguous rows become Unknown, never a name-search opportunity.
Arrays use a profile-supplied wrapper; function, tuple, union, intersection,
optional and applied shapes retain their component identities.

Binding-owned annotations and occurrence-owned root type arguments are installed
in LexicalCache. Source-bound field/return types overwrite only their exact
Compilation ID slots. Supported alias RHS recipes become ID-keyed templates;
the new alias step substitutes GenericParamIds and follows canonical TypeIds
before the legacy name-based alias machinery. This fixed an initially failing
local interface/alias cascade. Conditional/mapped/qualified/imported and other
unmigrated syntax remains explicitly represented by a legacy ingestion payload.

The reload test caught generic owner identity being reconstructed from `T` into
a different GenericParamId. `canonical_type_info_v1` now stores ID-keyed TypeInfo
payloads in the same transaction as the TypeArena snapshot. Reload preserves
captured parameter IDs, defaults, return/field IDs and alias templates. Freshly
parsed IDs win and deleted rows are ignored. Older indexes without the canonical
payload retain the legacy load path and need reindexing for this evidence.
This preserves already-captured defaults; it does not bind all default/constraint
syntax or prove full incremental semantic equivalence.

Ten fixtures add 19 labels, including a negative unconstrained-generic case.
The initial annotation/argument tests failed (four unresolved calls and one wrong
private-class target); all 95 current labels now agree with TypeScript 5.9.3.
Both TS/JS integration corpora and all five incremental tests pass. Native
source-budget/ID gates checked 67 changed source files; `git diff --check` passed.
The final wider library run passed 1,387 tests with 15 existing ignored tests
and no failures. No expected labels or historical snapshots were rewritten.

Still open: member-call type-argument occurrences, source-bound contextual
signature parameters, bounds/default syntax, legal merging, imported/qualified
type heads, all alias forms and downstream name bridges. In particular, emitting
a Decl does not by itself migrate every consumer of that Decl. These fixture
results do not establish a real-project 99% rate or an AI performance advantage.

### Member-call arguments and source-bound contextual signatures

```typescript
class Channel<T> {
  each<T>(callback: (value: T) => void): void {}
}
function use(channel: Channel<Alpha>) {
  channel.each<Beta>(value => value.save());
  // ✅ [generic] Method GenericParamId -> Beta, never the class's Alpha binding.
  channel.each(value => value.save());
  // ✅ [generic] The method parameter is open; no invented Alpha.save target.
}
function local() {
  class Model { save(): void {} }
  function consume(callback: (value: Model) => void): void {}
  consume(value => value.save());
  // ✅ [generic] Callee parameter TypeIds preserve this local Model declaration.
}
```

The existing TS/JS syntax tables drive ingestion [profile data]. Member type
arguments use a separate selector-byte map, so `make<A>().make<B>()` cannot
overwrite a root argument or the other member hop. TS member chain segments
now retain their selector byte addresses. Materialization uses TypeBinder;
the resolver reads those TypeIds rather than reparsing their source text.

Each captured declaration's ordered parameter TypeIds live in its exact TypeInfo
record, including unknown positional entries. `param_patterns` reads this list
first; only unmigrated records without it use the legacy signature parser.
The canonical snapshot carries the list and nested structural/generic IDs.
An old snapshot's absent field is explicitly legacy, not an empty signature.

`bound_call` combines receiver, explicit-call and argument-inferred bindings by
GenericParamId. Explicit arguments win. Own method and class parameters cannot
alias merely because both were spelled T. Inference matches structural shapes
and declaration IDs; conflicting inferences remain open. Callback writes still
use precise parameter spans -> BindingIds. Receiver return substitution now also
rewrites canonical GenericParamIds, preserving method-owner separation.

Ten fixtures add 20 compiler-checked labels, bringing the cohort to 115 calls
across 55 TypeScript fixtures (109 positive, six negative). The final library
run passed 1,401 tests with 15 existing ignored tests and no failures. Native
source-budget/ID gates checked 71 changed production files. No ignored tests,
expected-target changes or historical four-language snapshot changes were added.
All seven targeted integration checks also passed: five incremental tests and
the TS/JS resolution corpora. `git diff --check` passed; no downstream app build.

⚠️ [generic] This does not close the entire string-to-ID migration. Argument
identifier reads still enter a spelling adapter at the caller cursor; source
addresses must reach those reads too. Imported/qualified type heads, generic
bounds/default syntax, inheritance composition, overload selection, callable-value
context and remaining legacy alias/member bridges are still open. Unsupported
syntax remains a legacy ingestion payload. No language-specific resolver hook,
full corpus recapture, AI performance gain or real-project 99% result is claimed.

### Source-addressed argument values and initializer ownership

```typescript
function invoke<T>(callback: () => T): T { return callback(); }
function use() {
  class Model { save(): void {} }
  function make(): Model { throw 0; }
  invoke(make).save();
  // ✅ [generic] argument_reference: use span -> BindingId -> callable ID -> Function TypeId.
}
class Holder {
  item = identity(alpha);
  values = [identity(alpha)];
  callback = () => factory.identity(beta);
}
holder.item.save();
// ✅ [generic] source_call_initializers: exact declaration slot + call address owns the yield.
holder.values.save();
holder.callback.save();
// ✅ [generic] Neither nested call supplies the whole field value; no invented save target.
```

TS/JS shared argument ingestion now emits `IdentAt(SourceSpan)`, with no name
payload. Recursive leaves preserve their own spans, including parentheses,
await, arrays and ternaries; comments do not consume argument positions.
The source graph binds each argument once at ingestion. Argument reads occupy
a separate map from root calls and member type arguments. LexicalCache reads
type facts and versioned callable provenance at the use's own position without
changing the ambient cursor or borrowing later facts. Missing span/binding/type
evidence remains Unknown, with no name fallback for this variant.

Class values become Constructor(Decl), while named functions and supported
function-expression values carry canonical parameter/return TypeIds. The
higher-order member fixture exposed wrapper inference replacing a source-bound
return with Unknown; the declaration-slot source recipe now protects that
contract even when the legacy extractor return slot is empty.

Initializer prepasses use file-local binding environments and exact row IDs.
Captured expression/selector addresses identify the whole initializer call;
source-symbol attribution and argument-less TypeRef copies no longer determine
its value on this path. Unknown checks use the Type variant, not formatted text.
The isolated chain-pass test failed until ownership was joined by source address.

Eleven fixtures add 19 compiler-checked labels: the cohort now has 66 fixtures
and 134 calls (125 positive, nine negative). The wider library run passed 1,419
tests, with 15 existing ignored tests and no failures. Native source-budget/ID
gates checked 72 changed production files. No expected targets, historical
four-language snapshots or ignored-test annotations were changed for this slice.
All seven targeted integration tests passed: five incremental checks and the
TS/JS resolution corpora. The compiler verifier rechecked every current label;
`git diff --check` also passed. These are targeted checks, not the full workspace.

The offline, locked AlphaT and Lynx checks stopped before compilation because
both consumers require lockfile updates. Their files were left untouched;
consumer compilation is unverified. CallArg and SourceSpan retain their public
re-export paths and legacy serialized variants; IdentAt is a new public variant.

⚠️ [generic] Imported argument bindings still need module/export IDs; absent
local evidence now abstains rather than using the legacy global-name guess.
This is not full expression typing, CFG/closure execution semantics, overload
selection or incremental semantic equivalence. Parenthesized/awaited whole-field
initializers also need explicit ownership recipes. The sequential chain prepass
still rebuilds a file environment per candidate; amortization and performance
measurements remain open. No new language-specific resolver hook, corpus-wide
99% result, speed advantage or token-efficiency gain is claimed.
