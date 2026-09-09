1. [generic] Capture hierarchical module syntax into a separate FlowMeta namespace model; ordinary local-variable inference is not marked migrated.
2. [profile data] Supply Rust module/declaration/use/path/scope forms and namespace domains as syntax data.
3. [generic] Intern names at capture, bind initial path segments by ScopeId/NameId/domain and carry BindingIds to the existing module graph.
4. [generic] Add ID-keyed references to module binding entries so private local imports can feed aliases without entering the public export table.
5. [generic] Correlate declaration starts to exact extracted slots, then materialize persisted row IDs after writing symbols.
6. [generic] Capture static qualified-call root/selector byte anchors and install exact target facts in the per-file lookup overlay.
7. [generic] Preserve simple declared constructor/function returns as module-dependent TypeId recipes, including exact containing IDs for Self.
8. [generic] Export only source-public module items; local first-segment binding may access its own declarations while later selectors use public exports.
9. [generic] Preserve import collisions and local-module misses as unresolved barriers; keep unsupported crate/config/visibility forms explicit rather than claiming complete Rust scope coverage.
10. [generic] Verify source-level alias/constructor cascades, sibling/module namesakes, missing targets, cold reload and filtered external metadata; retain the crate-wide AliasDoc probe until that path is genuinely bound.

## Verified source-backed slice (2026-09-07)

```rust
use left::RealDoc as AliasDoc;
AliasDoc::new().touch(); // ✅ [generic] Scope/BindingId -> exact nominal/method IDs and return TypeId.
use right::{self as api};
api::RealDoc::new().touch(); // ✅ [generic] Numeric namespace selection; no alias-leaf rewriting.
api::new().touch(); // ✅ [generic] Type-domain namespace base, value-domain factory, ID-bound return.
// ✅ [profile data] Module/use/path/scope/domain and cfg-attribute forms are Rust data.
// ✅ [generic] Scoped competing imports and known missing targets cannot enter legacy name lookup.
// ⚠️ [generic] Wildcard providers are not bound yet: the intervening scope blocks outer-name borrowing.
// ⚠️ [generic] cfg/cfg_attr declarations are conservative barriers, not evaluated configurations.
```

Module bindings now reference other binding IDs with an explicit export domain.
The selector's final value domain cannot change the namespace base's type domain.
Bindings are file-local numeric entries, including function-local imports; nested
modules have separate source-module IDs. Source spellings are read/interned at
capture and serialized in input recipes, then lowered to numeric graph edges.
Static qualified-call roots and selectors enter FileLookup by byte address.
Ordinary local-variable inference is still a separate, unmigrated Rust path.

Constructor `Self` and simple declared return heads retain exact declaration IDs.
Serialized contract payloads omit FlowMeta; hydration rebuilds namespace metadata
against the surviving declaration slots, without restoring body call occurrences.
Binding epoch 4 rejects old persisted binding recipes; extraction cache schema 32
remains valid because the cached declaration layout did not change in this slice.
Existing indexed source needs reindexing to acquire the metadata.

Failing-first tests caught the absent alias root, wildcard shadowing borrowing an
outer namesake, inert attributes wrongly hiding declarations, and value-selector
domains incorrectly propagating into namespace bases. Each targeted test now
passes, checking constructor and downstream method IDs both fresh and cold.

## Integration finding: do not erase IDs to recover a misleading score

The Rust integration suite changed from 31/32 to **30/32**. Its assertions are
unchanged. The previously open crate-wide AliasDoc constructor remains unresolved;
the nested SelfProbe case is also red with the new exact constructor return IDs.
The diagnostic run found this actual selection:

```rust
use resolution_corpus_rust::selfmod::SelfProbe;
let p = SelfProbe::new(); // ❌ [generic] Legacy import path chooses outer SelfProbe.new in src/self_import.rs.
p.poke(); // ❌ [generic] Exact return ID preserves the outer owner, which has no poke member.
// Required target: selfmod.SelfProbe.new -> selfmod.SelfProbe -> selfmod.SelfProbe.poke.
```

`root_import_discipline::apply` still chooses a first same-spelling candidate from
the linked file/package. A file location is not a nested-module declaration
identity. Before ID-bound returns, a later name-based type lookup could hide such
an upstream mismatch. No fallback was restored and no assertion was weakened.
Temporary diagnostic queries were removed after recording the finding.

The next prerequisite is an explicitly configured crate graph: actual Cargo
target roots, dependency identities/renames and source-declared out-of-line
modules, followed by ID-only export traversal. The existing Rust module resolver
uses suffix matching and directory guesses; it is not suitable evidence for
claiming compiler-grade module identity. Private/restricted visibility,
wildcard providers, cfg evaluation, macros, generic/qualified type syntax,
imported impl ownership and complete Rust lexical binding remain open.
Unattested external/crate heads still use the existing resolver; this migration
does not claim to remove every Rust string-based path.

## Verification

- Broad targeted library run: **1,723 passed, 0 failed, 16 ignored** (5,512 filtered).
- Incremental integration: **5/5**; TypeScript and JavaScript integration: **1/1 each**.
- Rust integration: **30/32 checks**, overall test fails as described above.
- Independent TypeScript 5.9.3 verifiers: **79 module + 134 lexical labels**, unchanged.
- Source budget/ID ratchet: **100 changed production files** checked; diff whitespace check passes.
- AlphaT and Lynx compatibility checks attempted with `--lib --locked --offline`:
  both stopped because their Cargo.lock requires updates; their lockfiles were not changed.

These Rust fixtures are authored regression tests, not independent compiler
labels or a representative 99% measurement. No performance claim is made.
