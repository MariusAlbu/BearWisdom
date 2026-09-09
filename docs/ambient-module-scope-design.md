# Ambient module source identities

1. [evidence] All 54 remaining configured Query Core source barriers contain ambient external modules; their syntax now parses, but scopes and module identities are not represented.
2. [generic] Give module/namespace/global-augmentation bodies independent var-hoisting boundaries using profile-attested CST forms; never combine sibling declarations by spelling.
3. [profile data] Add module-container and augmentation shapes to the existing lexical module forms, not TypeScript branches in the generic binder.
4. [generic] Allocate source-local module IDs with parent, scope, source spans, declaration provenance and interned names before capturing imports or exports.
5. [generic] Install every import into its owning lexical scope before export/type/selector capture; detached selections retain the originating module ID and ambiguity barrier.
6. [boundary] Decode quoted module/export names once, including escaped Unicode; invalid literal evidence remains missing rather than matching raw spellings.
7. [generic] Capture root and per-module export surfaces separately; preserve explicit/implicit exports, unsupported syntax and declaration-set ambiguity rather than leaking module locals globally.
8. [snapshot] Lower source units/import origins to durable ModuleInput recipes and invalidate prior binding/extraction caches; verify filtering and cold-source restoration.
9. [integration] Keep the program incomplete until ambient provider selection and global-augmentation provenance are consumed by program-owned module binding; no workspace-global provider fallback.
10. [verification] Start with sibling var/import collision regressions, then test exact IDs, shadows, source spans, aliases, malformed/duplicate providers and unchanged real-project occurrence labels.

## Implemented source layer (2026-09-07)

Module/global nodes now establish var-hoisting boundaries through profile data.
Source-local units retain ID, parent, lexical scope, kind, decoded name, declaration
and body spans, source `declare` context, and explicit capture completeness.
Imports are installed in the owning body scope before type uses and export aliases;
detached namespace selectors preserve their original unit and duplicate barrier.

The source `declare` bit is not `.d.ts` ambient inference or configured module
legality. The compiler distinguishes export modifiers from export declarations:
an ambient interface remains implicitly exported beside `export interface`, but an
explicit `export { ... }` replaces that implicit export surface. Seven shared
fixtures have independent TypeScript 5.9.3 evidence for eleven export sets; two
same-spelling variable uses have independently checked, distinct declaration spans.

Durable `InputUnit` recipes now include source-scope evidence and unit-local
imports/exports. File-level BindingId queries use numeric forwarding edges into
their real owning unit. Export aliases refer to those binding IDs. Incomplete
unit/root evidence fences namespace, export and binding traversal; sibling units
remain independent. Binding epoch 34 and extraction schema 54 invalidate older
allocators and external source caches.

The expression-statement wrapper around a namespace can have exactly the same
byte range as its child declaration. Export capture checks the profile-attested
node kind as well as spans, then unwraps the declared wrapper; a range alone is
not enough to identify the declaration. Unsupported dotted namespace syntax,
export assignments, import attributes/equalities and unrepresentable quoted names
remain incomplete. Invalid exported aliases/module strings never become local
name fallbacks.

## Still required before configured providers are complete

- Capture nested global declaration contributions with their owning source-unit
  IDs, then validate legal augmentation context in the configured program.
- Allocate program-owned ambient providers and augmentation groups; bind imports
  against those instances, not a workspace-global literal-name lookup.
- Represent dotted namespaces, import-equals/export-assignment and implicit
  declaration-file ambient context with independent compiler evidence.
- Validate actual standard-library merging and richer source type recipes.

`lexical_globals.complete` is deliberately not relaxed by this change. Source
module capture is a prerequisite, not completion of F1 or the 99% correctness gate.

## Verification and unchanged real-project evidence

- Original scope regressions failed with the same BindingId for sibling vars and
  no scoped import binding. They now pass. Additional tests exposed absent durable
  units, dotted-name completeness and same-range wrapper/declaration confusion.
- 1,924 selected language/core/engine/oracle tests passed; the subsequently added
  filtered/portable ambient-unit test also passed. Seven TS/JS/Rust corpus and
  per-file/per-package integrations passed. No workspace/lint/format run.
- Parsed namespace exports select distinct physical rows even when every lookup
  display name is poisoned; late provider arrival, cold reload, real source export
  edits and provider deletion preserve the expected exact-ID outcomes.
- Native budget/ID audit: 283 production files, 95 resolver/type-checker files,
  zero violations. `git diff --check` passed. No public API signature changed.
- New configured report: `2026-09-07-query-core-module-scope-program-report.json`,
  SHA256 `23c160ade9176897759bc210970465b9002136dc5cb99c9162113b30e20a63bc`.
  All 187 supplied sources remain present; 54 global-capture barriers still fence
  all 690 labelled calls, with zero fresh/cold differences.
- New legacy report: `2026-09-07-query-core-module-scope-legacy-report.json`,
  SHA256 `3bf1ec00bda439a5e55c02de9f476cb9f900a6191a45b0c6028d92d5801a6164`.
  557 correct, 40 declaration-kind-only disagreements, 93 unresolved; fresh/cold
  equal. Both modes use the unchanged configured compiler manifest. Original
  reports and labels were not overwritten; this step has no measured recall gain.
