1. [generic] Extend the existing frozen module graph, not a second Rust-only resolver: nested modules get numeric identities distinct from their source file.
2. [generic] Introduce explicit value/type/macro export domains; keep the current bool API only as a TS/JS caller adapter.
3. [generic] Persist source-local module IDs, parent IDs and exact declaration targets in source-fingerprinted module inputs.
4. [generic] Allocate validated nested module IDs before lowering imports; duplicate IDs, dangling parents and cycles cannot bind.
5. [generic] Lower local namespace/export targets by module ID, never synthetic qualified-name strings or file-suffix matching.
6. [generic] Permit an explicitly typed selector path so a type-domain module can expose a value-domain function or macro-domain item.
7. [profile data] Move wildcard-name exclusions out of the generic walker: TS excludes default; other module systems must not inherit that rule.
8. [generic] Keep wildcard ambiguity, shared-provider cycle bounds, missing-target fences and domain-aware query cache keys through the migration.
9. [generic] Add source-recipe/cold-reload tests for nested namesakes, rename chains, three domains, malformed ownership and changed/deleted providers; rerun the independent TS cohort.
10. [generic] This prerequisite does not yet capture Rust crate declarations/import occurrence bindings: wire those to this graph next, retaining AliasDoc as an explicitly failing end-to-end probe until then.

Implemented contract:

```rust
graph.binding_in(file, module, binding, ExportDomain::Type);
// ✅ [generic] (ModuleId, BindingId, domain) identifies the result, not a qualified name.
graph.binding_in(file, module, binding, ExportDomain::Value);
// ✅ [generic] Same spelling/binding number in another domain cannot reuse the type result.
graph.binding_in(file, module, binding, ExportDomain::Macro);
// ✅ [generic] Explicit macro export facts remain separate from value/type facts.
// ✅ [generic] Nested source-local module IDs lower to distinct compilation ModuleIds.
// ✅ [generic] Type-domain module selectors can lead to value- or macro-domain terminals.
// ✅ [profile data] Wildcard exclusions belong to ModuleForms (TS/JS excludes default).
// ⚠️ [generic] This graph consumes attested input recipes; it does not create Rust evidence.
AliasDoc::new();
// ❌ [generic] Rust import ingestion still does not supply these recipes to constructor lookup.
```

The source-ingestion boundary persists `SourceModuleId`, parent ID, explicit
export domain and exact declaration row IDs. Rebuild validates ownership before
allocating global module identities. Forward parents are valid; duplicate IDs,
cycles, dangling parents and attempts to overwrite root ID zero are not.
Normalized duplicate/inconsistent source keys cannot replace a valid root.
Names are interned once when recipes lower to the frozen graph. No module path
is synthesized from a declaration's qualified name.

`InputTarget::LocalNamespace` and `LocalExport` retain source-module identity.
`Select` carries the export domain of each selector, while the existing `Path`
adapter uses the caller's domain for existing TS/JS recipes. The bool lookup API
remains an adapter for current TS/JS consumers; traversal and query caches use
`ExportDomain`. Wildcard traversal remains iterative across shared barrels and
keeps its cycle/work bounds. Unknown wildcard providers preserve incompleteness.

Import tables retain all explicit targets instead of silently overwriting the
previous entry. Different attested targets are ambiguous; an uncaptured explicit
competitor makes the result incomplete rather than proving the one remaining
candidate unique. This applies to explicit exports too. It is a precision fence,
not complete duplicate-declaration or invalid-code navigation diagnostics.

Failing-first evidence:

- A wildcard re-export of `default` without any language exclusion returned
  `Missing` instead of the exact declaration ID 71. Moving that policy to data
  fixes the generic case while keeping TS/JS's exclusion.
- An import with both an attested target and an uncaptured competitor returned
  `Bound(72)` instead of `Incomplete`. Candidate aggregation now preserves the
  missing evidence.

Persistence uses module-binding epoch **3**. Old bool-domain payloads are still
readable but cannot attest current bindings, even with matching source hashes.
New nested units and all three domains round-trip through the existing metadata
store. Tests check source-hash rejection, retargeting and complete removal of a
source's module identities. These are graph-input tests, not independently
labelled Rust source-occurrence tests or full dependency invalidation evidence.

Semantic references: [Rust namespaces](https://doc.rust-lang.org/reference/names/namespaces.html)
and [Rust modules](https://doc.rust-lang.org/reference/items/modules.html).
Modules occupy the type domain; context can select a value or macro inside one.
Rust also has lifetime/label namespaces and macro sub-namespaces. Those are not
implemented by adding this three-domain export representation.

Next integration work remains source-addressed Rust module/import capture,
crate-root and dependency IDs, explicit out-of-line module declarations,
visibility/restricted exports, cfg/path attributes, scoped import occurrences,
and bound alias/constructor yields. Do not toggle the Rust imported-target
roadmap parent complete on the strength of these prerequisite tests.

Verification (2026-09-07):

- Final targeted library run: **1,710 passed, zero failed, 16 ignored, 5,512
  filtered out**. Eleven tests were added in this continuation; existing graph
  tests were migrated to explicit domains without weakening their expectations.
- Independent TypeScript 5.9.3 verifiers: **25 module cases / 79 labels** and
  **66 scope cases / 134 labels** pass. Those 213 labels are unchanged; none are
  evidence of Rust source-occurrence binding.
- Incremental integration **5/5**, TypeScript resolution corpus **1/1** and
  JavaScript resolution corpus **1/1** pass; a separate repeat of those three
  integration binaries exits zero.
- Rust integration remains **31/32 patterns**, exiting nonzero only for
  `AliasDoc::new()`. Its downstream touch, constructor chains and nested SelfProbe
  remain passing. No baseline pattern or expectation was changed.
- Source-size/ID-ratchet gates pass across **95 changed production source files**;
  `git diff --check` passes. No full corpus recapture, format/lint, commit,
  deletion or sibling-project mutation was performed.

Commands:

```text
cargo test -p bearwisdom --lib -- indexer::resolve::engine indexer::lexical indexer::flow indexer::symbol_ids indexer::write indexer::contract_bindings indexer::contract_filter indexer::external_parse db::lexical_visibility resolution_oracle languages::typescript languages::javascript languages::rust_lang languages::common::call_args types::call_arg --test-threads=1
cargo test -p bearwisdom-tests --test incremental --test resolution_corpus --test resolution_corpus_js --test resolution_corpus_rust -- --test-threads=1
cargo test -p bearwisdom-tests --test incremental --test resolution_corpus --test resolution_corpus_js -- --test-threads=1
node crates/bearwisdom/src/resolution_oracle/verify_modules.mjs F:/Work/Projects/AlphaT/node_modules/typescript/lib/typescript.js
node crates/bearwisdom/src/resolution_oracle/verify_typescript.mjs F:/Work/Projects/AlphaT/node_modules/typescript/lib/typescript.js
git -c core.safecrlf=false diff --check
```
