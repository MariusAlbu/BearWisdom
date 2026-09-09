1. [generic] Add compiler-checked merged-interface, class/interface, generic, export and scope-isolation cascades before implementation.
2. [profile data] Express merge-compatible declaration kinds in lexical syntax data; aliases and duplicate classes do not become legal merges.
3. [generic] Group source declarations by ScopeId/NameId-derived BindingIds, never qualified-name equality during semantic resolution.
4. [generic] Keep type/value bindings distinct while correlating dual class declarations with their type-space merge group in either source order.
5. [generic] Materialize allowed groups as exact declaration row IDs and retain singleton/invalid groups as fences against legacy name merging.
6. [generic] Canonicalize only attested groups before type recipes or member indexes consume their IDs; retain physical member declaration IDs.
7. [generic] Bind a multi-row lexical type/export only when all physical rows agree on one attested canonical identity.
8. [generic] Persist source-fingerprinted grouping evidence and restore it before DB-loaded canonicalization; stale/deleted rows cannot revive a group.
9. [generic] Normalize generic parameter ownership by canonical declaration identity and ordinal, without matching parameter spellings.
10. [generic] Verify fresh/cold/edited behavior, compiler labels, targeted regressions and source/ID gates; augmentation, namespace merges and overload choice remain explicit follow-ups.

Implementation notes:

- `lexical_merges` correlates dual value/type bindings and exact symbol slots. Merge compatibility is profile kind-pair data plus syntax-captured generic arity; duplicate classes/aliases and unequal arity stay separate. This is not a complete diagnostic checker for conflicting members, generic constraints/defaults, or mixed exported/local declarations.
- Module inputs retain scoped declaration groups, including singleton fences. Compilation installs this evidence before merging members or materializing signatures. Class/interface groups choose the class representative; interface-only groups use a deterministic row. Members retain their physical navigation targets.
- `TypeBinder` and export lowering require all rows to agree on the attested canonical ID. Generic parameters read the canonical declaration's ordinal slot. No semantic lookup compares type parameter spellings.
- A negative scope fixture exposed `chain::lookup_member_on_bounded` falling from a bound nominal ID miss into a same-qname member bucket. Bound nominal misses are now authoritative. Unbound legacy receivers still have compatibility bridges; this does not remove every string-shaped member selector or type-level fallback.
- Older synthetic chain fixtures supplied only qname member tables, despite having an indexed type ID. They now supply parent-ID tables too. The constructor-interface fixture supplies its actual variable declaration/type, rather than relying on an invented `${head}Constructor` relation. Original expected member targets remain unchanged.
- Source hashes do not validate BindingId allocation across binder changes. `ModuleInput.binding_epoch = 2` fences older persisted inputs; existing indexes need current syntax/module metadata recaptured. Full project/configuration/dependency fingerprints remain future work.
- The shared module oracle now exercises each label both fresh and after a database TypeArena/module snapshot restore. This is cold semantic resolution with the same captured source occurrences, not proof of all incremental edge invalidation.

Rust integration follow-up design (required by the authoritative-ID member fence):

1. The Rust corpus fell from its recorded 30/32 baseline to 28/32; trace the constructor-root cascade before relaxing any identity rule.
2. The source probe shows `Builder` and `Widget` have empty ID member buckets: their impl methods all have `parent_index=None`.
3. Add a generic source-ingestion binder for discontiguous declaration bodies, reusing ScopeId/NameId/BindingId rather than qualified-name comparisons.
4. Supply Rust scope, declaration, impl target, generic-wrapper and member node kinds as profile data.
5. Bind local nominal self types in their lexical declaration scope, including forward declarations, while generic parameters and aliases block incorrect borrowing.
6. Correlate source declaration/member anchors with exact extractor slots; existing structural parents remain authoritative.
7. Write the resulting owner slot into `parent_index` before persistence, so Compilation and cold DB loads use the existing containing-ID path.
8. Keep qualified/imported self types and trait dispatch out of this initial local-owner contract instead of guessing their targets.
9. Preserve the extracted impl namespace row as syntax/coverage metadata; it is not the nominal owner of the methods.
10. Verify direct/chained calls, same-named types in separate scopes, forward declarations, generic shadows and cold reload; rerun the Rust corpus against its recorded baseline.

Rust implementation/evidence:

- `lexical_detached` binds a bare local nominal impl target through ScopeId/NameId/BindingId, then correlates source anchors to extractor slots. `rust_lang/owners` supplies syntax data; the resolver receives existing `parent_index`/containing-ID relationships rather than another spelling adapter.
- The direct constructor-chain test checks the exact `Widget.show` declaration ID both fresh and after restoring the persisted TypeArena and symbol database. Extractor tests cover forward declarations, separate modules, generic targets, generic/alias shadows, duplicate declarations, qualified targets and trait-impl exclusions.
- The Rust integration corpus now reaches **31/32**, compared with its recorded **30/32** baseline and the intermediate **28/32** regression. Both chained-constructor probes and the previously failing nested SelfProbe probe resolve. The test still exits nonzero: the existing renamed re-export `AliasDoc::new()` failure remains; its following `touch()` now resolves. Do not relabel this corpus green.
- This binder deliberately does not supply qualified/imported targets or trait-dispatch relationships. An intervening scope with a `use` declaration is conservatively opaque, even for unrelated imports; precise import-name binding is needed before claiming ordinary Rust scope completeness. Inline module scopes do not implicitly inherit outer type names.
- A negative fixture exposed another existing extraction limitation: a function-local impl's callable is emitted as Function rather than Method. Its actual extracted row is still checked for non-interference; the test does not hide it by filtering only Method rows.
- Rust source rewrites already preserve byte positions; the added pass reuses the existing syntax tree. No additional parse, builtin list, fake DB declaration or resolver qualified-name comparison was introduced.

Final verification (2026-09-07):

- Targeted library suite: 1,671 passed, zero failed, 16 ignored, 5,524 filtered out. Includes engine, lexical/flow, persistence/contract, oracle, TS/JS and Rust extractor tests.
- Integration: incremental 5/5, TypeScript resolution corpus 1/1 and JavaScript resolution corpus 1/1 pass. Rust integration exits nonzero with the one remaining AliasDoc constructor pattern described above (31/32).
- Independent TypeScript 5.9.3 checks: module cohort 25 cases / 79 labels and existing scope cohort 66 cases / 134 labels pass (213 labels total). Original baseline oracle snapshots remain unchanged.
- Source-size/ID-ratchet gates pass across 90 changed production source files; `git diff --check` passes. No format/lint command, full corpus recapture, sibling-project mutation or commit was performed.
- This is a targeted F1 checkpoint, not representative multilingual 99%/99.9% evidence or complete incremental/fresh snapshot equivalence. Full configuration fingerprints, merge legality/augmentation, member-selector/heritage ID migration, overload selection and precise Rust imports remain open.

Subsequent continuation: `detached-import-bindings-design.md` supersedes the
blanket use-bearing-scope abstention and function-local Method-classification
limitations above. It also fixes forward-parent contract filtering and bumps
the external extraction cache schema to 32. Imported-target/crate identity and
the AliasDoc constructor remain separate open work.
