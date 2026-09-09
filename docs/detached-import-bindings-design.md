1. [generic] Replace detached-owner scope-wide import abstention with syntax-captured exposed NameIds and scoped BindingIds.
2. [profile data] Describe import statements, paths, aliases, groups, self leaves, discarded names and wildcards as node/field data.
3. [generic] Intern only exposed names at ingestion; path components never become spurious local bindings.
4. [generic] Explicit imports shadow outer declarations and conflict conservatively with duplicate local declarations; unresolved import targets remain unavailable.
5. [generic] Wildcards block borrowing outer bindings but do not suppress a nominal declared directly in the same scope.
6. [generic] Unknown/malformed import syntax stays opaque; aliases and underscore imports cannot accidentally expose their original names.
7. [profile data] Preserve inline-module barriers and treat module declarations as type-namespace shadows.
8. [extractor] Reuse proper Rust impl extraction for function-local impl bodies; extract existing nested-item handling from oversized calls.rs into a sibling-tested module.
9. [generic] Add failing-first source probes for unrelated imports, aliases/groups/wildcards, collisions, late imports, nested generic shadows and exact parent IDs across cold reload.
10. [generic] Verify the whole affected cascade, record the remaining alias-constructor root-typing gap, and do not claim imported-target or crate-wide resolution from name-binding evidence alone.

Persistence follow-through: function-local items must retain their enclosing callable's parent slot, not become global contract declarations. Impl-to-type links may point forward, so contract ancestry must traverse arbitrary parent order rather than assuming `parent < child`. Test filtered/cache-hydrated owner IDs and bump the external extraction cache epoch; unchanged source bytes do not validate older extraction semantics.

Implementation and evidence:

```rust
use foreign::Other;
struct Model;
impl Model { fn save() {} fn reload() {} }
// ✅ [generic] The import exposes only Other's NameId; both methods keep Model's owner slot.
// ✅ [profile data] Paths, aliases, grouped self imports, raw identifiers and underscore imports.
// ✅ [generic] Explicit imports shadow outer owners; local declarations take precedence over globs.
// ✅ [generic] Function-local impl methods keep Method kind and exact nominal/callable ancestry.
// ✅ [generic] Forward owner links survive filtering and cache/DB serialization.
// ⚠️ [generic] Imported targets, wildcard providers, namespace domains and crate paths still need binding.
```

- `lexical_import_names` captures exposed source names, wildcard presence and unknown syntax separately. `lexical_detached` interns exposed names and stores unknown-target import BindingIds, never re-finds a nominal owner by qualified name. Import aliases expose the alias only; path prefixes are not declared as local symbols.
- Inline module declarations are non-nominal scope barriers. Raw identifier prefixes normalize at ingestion. Malformed syntax and unsupported extern-crate declarations remain opaque; explicit imports whose target namespace is not yet known may conservatively block a same-spelled type.
- `calls_local_items` extracts the previous nested-item handling from `calls.rs` and routes impl bodies through the existing proper impl extractor. Nested declarations now carry their enclosing callable slot, and calls inside methods/helpers retain distinct source IDs.
- `contract_filter::callable_ancestry` follows arbitrary parent order with a memoized linear walk. Cycles and dangling parents cannot attest public contracts. The failing-first forward-owner fixture previously leaked `save` and its parameter after filtering out the local Model itself; it now retains only the outer function.
- External extraction cache schema version is **32**, invalidating old payload keys without deleting data. Source hashes alone do not establish extractor semantic compatibility. Existing indexes still need reindexing to populate new ownership metadata.
- Initial owner probes: unrelated imports and function-local Method classification failed before implementation, while explicit-shadow negatives passed. Fresh/cold constructor cascades with four import shapes now resolve to the exact Widget.show ID. Filtered Rust contracts round-trip through serialized CachedParse while retaining external owner slots and excluding local impl bodies.
- The existing AliasDoc constructor issue is not fixed here: `root_import_discipline::type_candidate` does not treat type_alias as a nominal root and can report UncapturedField before alias expansion. Rust renamed aliases also still capture a bare RHS name in `calls_imports`; fixing only the kind gate would retain a name-re-resolution dependency. Crate/module-aware alias target IDs are required.

Semantic reference: the name-exposure and glob-shadow policy is based on the
[Rust Reference use declarations](https://doc.rust-lang.org/reference/items/use-declarations.html),
not engine output used as ground truth. These unit/integration probes are not an
independently labelled compiler occurrence corpus and do not establish 99%.

Final verification (2026-09-07):

- Targeted library suite: **1,699 passed, zero failed, 16 ignored, 5,512 filtered out**. This selection includes external-parse tests as well as the engine, lexical/flow, persistence/contract, oracle and language tests; the count difference from the previous checkpoint is not a count of newly added tests.
- Integration: incremental **5/5**, TypeScript resolution corpus **1/1** and JavaScript resolution corpus **1/1** pass. The Rust integration test still exits nonzero at **31/32 patterns**: only the existing `AliasDoc::new()` pattern fails. Its downstream `touch()` resolves; constructor chaining and nested SelfProbe remain passing. Separate diagnostic candidate probes are not promoted into passing baseline patterns.
- Independent TypeScript **5.9.3** verification passes for the unchanged module cohort (**25 cases / 79 labels**) and scope cohort (**66 cases / 134 labels**): **213 labels**, not representative multilingual correctness evidence.
- Source-size and ID-ratchet gates pass across **94 changed production source files**; `git diff --check` passes. No formatting/lint command, full corpus recapture, sibling-project mutation or commit was performed.
- The next Rust step is crate/module/declaration-ID binding for imported and renamed targets, including AliasDoc. The overall F1 milestone and independent 99% recall / 99.9% precision gate remain open.

Commands (each Cargo invocation ran alone, with direct output):

```text
cargo test -p bearwisdom --lib -- indexer::resolve::engine indexer::lexical indexer::flow indexer::symbol_ids indexer::write indexer::contract_bindings indexer::contract_filter indexer::external_parse db::lexical_visibility resolution_oracle languages::typescript languages::javascript languages::rust_lang languages::common::call_args types::call_arg --test-threads=1
cargo test -p bearwisdom-tests --test incremental --test resolution_corpus --test resolution_corpus_js --test resolution_corpus_rust -- --test-threads=1
node crates/bearwisdom/src/resolution_oracle/verify_modules.mjs F:/Work/Projects/AlphaT/node_modules/typescript/lib/typescript.js
node crates/bearwisdom/src/resolution_oracle/verify_typescript.mjs F:/Work/Projects/AlphaT/node_modules/typescript/lib/typescript.js
git -c core.safecrlf=false diff --check
```
