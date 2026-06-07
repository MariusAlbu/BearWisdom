# EXT-2 finish — corrected by Phase 0

The interface side of EXT-2 is closed (external Trait/Interface members are
admitted; structural satisfaction over an external interface is locked by
`project_struct_structurally_satisfies_external_interface`).

This document set out to design demand-driven external **class-member**
admission, on the premise that the `MembersIndex` write-storm guard blocks
chain-walking through an external class's methods (`repo.get().name`). **Phase 0
disproved that premise.** The remaining EXT-2 work is narrower and is incremental
return-type coverage, not an architectural block.

---

## 1. Lifecycle (accurate, retained)

```
full.rs:552   DemandSet::from_parsed_files(&parsed)        # names the project references
full.rs:566   parse_external_sources(.., &demand)          # extract ONLY demanded ext symbols
full.rs:675   resolve_iteration (iter 0)                   # build SymbolIndex + Engine, resolve
                 chain walker miss -> record_chain_miss(current_type, target_name)   [chain.rs:1010]
full.rs:695   while !converged && iter<8:                  # demand-driven expand loop
                 expand_chain_reachability(&chain_misses)  # pull the FILE defining each miss
                 resolve_iteration (re-resolve)
```

## 2. Phase 0 finding — external class-method chains already resolve

The original premise was that `MembersIndex::build_from_parsed_files`
(members.rs:91-106) skips external Class/Struct members, so `repo.get()` can't
find `get`. **That guard is bypassed on the chain-walk path.** When
`MembersIndex.lookup` misses, the walker falls to `qualified_member_lookup`
(chain.rs:605 -> 1204):

```rust
// chain.rs:1219 — qualified_member_lookup
let candidate = format!("{qname}.{seg_name}");          // "Repository.get"
if let Some(hit) = self.lookup.by_qualified_name(&candidate) {   // ext symbols ARE indexed as lookup targets
    return Some(hit.clone());
}
// + external-qname promotion (chai.Assertion / rxjs.Subject / Newtonsoft.*) at 1222-1241
```

External symbols are indexed in `by_qualified_name` as lookup targets, so
`Repository.get` is found there even though `MembersIndex` skipped it. The chain
then continues through `get`'s return type (yielded from its signature /
`SymbolTypeMap`, which INFER-9 slice-2 admits for external callables).

**Verified end to end** by `ext2_external_class_method_chain_resolves_end_to_end`
(`type_checker/engine_tests.rs`): `repo.get().greet()` where `repo` is typed as
an `ext:` class `Repository`, `Repository.get(): User`, and `User.greet()` is an
external method — binds `greet` on the external return type `User`. No member
admission, no engine change.

**Consequence:** the demand-driven member-admission design (member set threaded
into `MembersIndex`, widened gate, loop-condition change) is **not needed** — it
would solve a non-problem. Dropped.

## 3. The real remaining frontier

External chain-walking works when, at each hop, (a) the member symbol is in
`by_qualified_name` and (b) the method's return type is captured. (a) is handled
by the demand/expand loop (pull the file, re-resolve). So the residual gap is
purely **(b): return-type capture for external method shapes the walker does not
yet emit** — exactly CLAUDE.md's "method return-type maps (sourced from pf.refs
TypeRef edges the externals walker doesn't always emit)".

EXT-2 slices 1-4 already capture the common shapes (colon/arrow returns,
module-tagged TypeRef, JVM descriptor decode, C aggregate-return peel). What
remains is the long tail, each a bounded per-shape extractor/signature cut, e.g.:

```
chain breaks at a hop whose external method return type isn't captured:
  - fluent builder returns (Kysely SelectQueryBuilder.where(): SelectQueryBuilder)   # self-returning, may lack a parseable sig
  - generic returns through an external generic (Observable<T>.pipe(): Observable<R>) # needs the generic arg carried
  - alias-target maps for externals (type X = Ext<Y>)                                 # the doc's other captured-as-no gap
```

Each is measurable: a chain that breaks shows up as a `chain_miss` whose
`current_type` is the external return type that failed to yield. None is a deep
architectural change.

## 4. Recommendation

EXT-2 is **not** the open-ended block the (stale) roadmap framing implied. Its
two mechanisms — member resolution (by-qname + demand/expand) and return-type
yield (slices 1-4) — both work. Close it by:

1. Running **DOC-4** (the closeout recapture) to surface which external
   return-type shapes actually break chains in the corpus (the `chain_miss`
   tail on `ext:` current_types).
2. Landing the specific return-type-capture cuts that the tail shows are worth
   it — each a small, bounded extractor/signature change, validated per-project.

There is no demand-driven-admission prerequisite. The interface side is locked;
the class-method side resolves; the residue is data-capture coverage that DOC-4
prioritizes.

## Phase 0 result — PREMISE REFUTED

`ext2_external_class_method_chain_resolves_end_to_end` passes: the external
class-method chain resolves with no engine change. The member-admission design
is dropped; the real frontier is return-type-shape capture, measured by DOC-4.
