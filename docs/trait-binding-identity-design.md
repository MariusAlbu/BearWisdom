# Trait binding and implementation evidence

Historical design and source-contract checkpoints follow. For the current implemented
dot-call selector and remaining limitations, see [trait-selection-design.md](trait-selection-design.md).

Historical design and source-contract checkpoints follow. For the current implemented
dot-call selector and remaining limitations, see [trait-selection-design.md](trait-selection-design.md).

1. Scope: the retained three trait/default/generic/qualified Rust oracle misses; implementation is still open.
2. Root cause: namespace_extensions excludes trait-bearing impls; member_selection receives neither implementation relations nor bound-parameter trait evidence.
3. Generic capture must represent trait declaration IDs, receiver type recipes, implementation source identity and member implementation IDs separately from inherent owners.
4. Trait availability is occurrence-scoped import/bound evidence, not the presence of a matching declaration anywhere in the index.
5. Receiver candidates must be ordered before searching inherent/trait members at each candidate; current projection alone does not implement that ordering.
6. Generic bounds and qualified trait calls need their own source TypeId/BindingId recipes, including Self substitution; unresolved constraints remain explicit.
7. Pinned rustc 1.94.0 HIR/THIR independently identifies the trait declaration as the static FnDef target for concrete, generic and qualified calls even when an implementation overrides the method.
8. Therefore static binding edges and implementation-body reachability must be distinct ID relationships; never replace the former with a namesake implementation row to raise recall.
9. Negative/ambiguity cases must cover trait visibility, competing traits, receiver-order precedence, implementation applicability, provider edits/deletion and cold snapshots.
10. Source-labelled compiler targets, IDE navigation policy and flow dispatch candidates need separate assertions; neither graph reachability nor compiler acceptance alone proves correct binding.

```rust
trait Save { fn save(&self); }      // ⚠️ [generic] static declaration target
impl Save for Doc { fn save(&self) {} } // ⚠️ [generic] separate implementation-body relation, not the static target
p.save();                         // ❌ [generic] missing trait evidence in current member selection
<Doc as Save>::save(p);            // ❌ [generic] qualified source binding remains open
```

The compiler contract is executable in `verify_rust_tests.mjs`: three independently
labelled calls validate against the trait declaration, and relabelling them to the
implementation body must fail. This adapter test is not counted as engine recall.

## Source-contract implementation sequence

1. Add generic namespace trait facts; Rust supplies CST forms only, with no language branch in resolution.
2. Address each implementation by its file-local Self BindingId and source span, not a manufactured nominal declaration.
3. Retain trait declaration rows and implementation member rows in separate records, including empty default implementations.
4. Capture exact trait/receiver type recipes and inline/where bounds through the existing source type binder.
5. Keep implementation generic parameters as owner BindingId/position recipes; unresolved applicability is not a blanket impl.
6. Record occurrence-scoped visible type BindingIds, with an explicit incomplete flag for unknown wildcard providers.
7. Persist these contracts in fingerprinted ModuleInputs; lower physical slots through SymbolIds only.
8. Rebind provider-facing recipes against the current module graph, never a cached name or old provider row.
9. Test source shadowing, conditional/filtered declarations, real provider edits/deletion, serialization and fresh-arena cold reload.
10. This is the prerequisite contract for ordered trait selection, not completion of dispatch; keep the three compiler-labelled misses open until selection and yield substitution are implemented.

[Rust method-call candidate order](https://doc.rust-lang.org/reference/expressions/method-call-expr.html)
and [trait bounds](https://doc.rust-lang.org/reference/trait-bounds.html) define the
language-level constraints the source model and generic selection engine must preserve.


## Verified source-contract checkpoint — 2026-09-07

Implemented the prerequisite contract in `namespace_traits`, `namespace_ingest_traits`,
`namespace_type_traits` and `module_trait_inputs`. Implementation Self BindingIds,
physical container/member declaration IDs, exact source spans, parameter positions,
trait/receiver recipes, inline/where/supertrait obligations and negative-impl polarity
remain separate. Conditional, malformed and missing-member source is flagged; unsupported
bound recipes remain Unknown rather than disappearing into an unconditional impl.
Binding epoch 20 / extraction schema 40 invalidate the previous source-ID layout.

Trait availability is a parent-linked ScopeId environment, not ordinary shadowing
lookup. A pinned compiler probe rejected two same-named traits imported at different
lexical depths with E0034: the outer trait still contributes methods. Anonymous
`use Trait as _` imports also retain distinct BindingIds without declaring `_`.
Wildcard-provider incompleteness survives in the scope frames. Each frame is captured
once; a 100-trait/100-caller/200-call test stores 100 file bindings, not 10,000 copies.
Header recipes use an exact source-span index instead of scanning every impl per CST node.

```rust
trait Save { fn save(&self); }     // ✅ [generic] physical parent ID classifies an associated method, not a free function
impl Save for Doc {}               // ✅ [generic] source trait/receiver recipes and a distinct impl identity persist
fn f<T: Save>(p: &T) { p.save(); }  // ⚠️ [generic] obligation IDs captured; trait selection and Self substitution still missing
use api::Save as _;                // ✅ [profile data + generic] anonymous import has a BindingId, not an underscore declaration
```

Tests rebind unchanged implementation recipes after real provider edits, reject a
deleted provider rather than reuse its old IDs, and preserve the source contract
through a fresh-arena cold reload. Trait impl source-container ancestry also fixes
orphaned methods from filtered local-type implementations. Physical lexical ancestry
does not settle all export policy: body-local impls affecting nonlocal types and their
contract dependency closure still need dedicated completeness evidence.

The expanded compiler-labelled trait cohort deliberately retains one wrong-target
baseline: shared-reference trait lookup must precede a mutable inherent receiver.
Static trait selection, implementation applicability, receiver ordering, abstract-method
Self/region substitution and qualified calls are still implementation work, not completed
by these persisted inputs. The compiler fixture labels and observed snapshots are separate.

[Anonymous imports](https://doc.rust-lang.org/reference/items/use-declarations.html#underscore-imports)
are part of the source availability contract, not a special-case builtin list.
