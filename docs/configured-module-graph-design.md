1. [generic] Add neutral package/target/dependency configuration records to manifest ingestion; keep configuration text out of semantic traversal.
2. [profile data] Cargo ingestion supplies actual library/target paths, crate names and dependency aliases; custom paths override conventional defaults.
3. [generic] Snapshot per-package configuration and persist it with module inputs; a supplied fresh configuration wins over a cold snapshot.
4. [generic] Capture out-of-line module declarations and inline module directory ancestry as source-addressed input recipes.
5. [profile data] Rust supplies file suffix/directory-entry/path-attribute forms; source paths are lowered only at graph construction.
6. [generic] Link a declared module only to exact indexed candidate paths; duplicate alternatives and conflicting ownership remain ambiguous.
7. [generic] Carry configured crate roots, parent modules and imported namespace selections as numeric graph targets with explicit domains.
8. [generic] Bind imported constructor roots before member/return inference; never restore the wrong-owner name fallback exposed by SelfProbe.
9. [generic] Test custom roots, module layouts, consumer-scoped dependency aliases, namesakes, deletion/config retargeting and cold restoration.
10. [generic] Verify full constructor-to-method cascades and strengthen identity assertions; do not equate green fixture counts with the independent 99% gate.

Configuration rules: [Cargo targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html),
[dependency specification](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html),
and [Rust module source filenames](https://doc.rust-lang.org/reference/items/modules.html).

## Implemented slice — 2026-09-07

The ecosystem reader now produces package-addressed target/dependency records.
Cargo-format parsing stays in `ecosystem/cargo/module_manifest.rs`; it is not a
language-specific resolution hook. TOML 0.8.23 is parse-only and declares MSRV
1.66, below this workspace's declared 1.75. Both manifest unions and per-package
contexts retain the records. Explicit library names/paths, auto-target discovery,
target discovery switches, path-dependency renames, dependency sections, and
ancestor-workspace dependency inheritance are captured.

Rust supplies module filename/attribute syntax as data. The shared source binder
captures out-of-line declarations, inline directory ancestry, and crate/parent
path recipes. Graph construction lowers these and configured dependency locations
to numeric module/export targets. Semantic traversal never searches filenames,
compares qualified names, or expands alias spellings to select these targets.
The graph stores cross-file parent IDs and separate type/value/macro domains.

```rust
AliasDoc::new().touch(); // ✅ [generic] alias -> declaration -> constructor/return -> member IDs
use api::RealDoc;       // ✅ [generic] consumer-specific configured path dependency
use super::RealDoc;     // ✅ [generic] declared cross-file parent relationship (public export slice)
#[path="layout.rs"]
mod api;               // ✅ [profile data] exact source-path recipe, decoded during ingestion
// api.rs AND api/mod.rs: ✅ [generic] ambiguity, never first-file-wins
// same physical root in two targets: ⚠️ [generic] abstains until module instances exist
```

Binding epoch is 5. Module inputs and package configuration are persisted; fresh
configuration, including an explicitly empty configuration, wins over the stored
snapshot. Tests cover cold restoration, configuration-only root retargeting, and
deleted source roots even when a same-named declaration remains available.
This is snapshot behavior, not automatic detection of on-disk configuration edits
without a freshly built ProjectContext; dependent-edge invalidation remains F2.

## Evidence and the fixture defect

The original Rust corpus briefly reported 32/32 after these changes while the
nested SelfProbe constructor had **no edge**. Its downstream `p.poke()` still
passed through legacy local-type inference. A new exact-ID constructor assertion
failed with `actual=[]` versus the nested constructor ID. This demonstrates why
downstream-hit counts alone are not correctness evidence.

The fixture imported `resolution_corpus_rust::selfmod::SelfProbe`, but `lib.rs`
never declared/re-exported `self_import.rs`. Its intended module wiring is now
explicit: `mod self_import; pub use self_import::{SelfProbe, selfmod};`. Existing
expected targets were not weakened. New assertions check both constructor and
method IDs for the nested SelfProbe and AliasDoc cascades. The repaired corpus
passes 32/32, with its three additional known-red probes still reported separately.
New negative fixtures retain the missing-export case and require the whole direct
constructor/member chain to abstain; the separate-local legacy inference gap is
explicit follow-up work, not silently counted as solved.

Verification: 1,902 selected core tests passed, 16 ignored; the 213 independent
TypeScript compiler labels (79 module + 134 scope) are unchanged and pass. These
are not independent Rust binding labels and do not establish the 99% gate.

All 12 selected integration tests passed: incremental (5), per-file manifests (2),
per-package context (2), and TS/JS/Rust corpora (1 each). A Windows-compatible audit
of 108 changed production files found no file-budget or ID-discipline violations.
`git diff --check` passed. Sibling checks did not reach compilation: Lynx requires
a lockfile update under `--locked`; AlphaT's `--locked --offline` check fails on
the yanked `der ^0.8.0` dependency selected through ureq/ort. No sibling files or
lockfiles were changed, and API compatibility is not claimed as verified.

## Remaining boundaries

- Registry/git dependency manifests and stdlib configuration are not yet connected
  to this graph; unconfigured external roots remain an explicit legacy transition.
- Full Rust lexical occurrence/initializer binding is still incomplete. A rejected
  constructor must eventually invalidate downstream inferred locals by BindingId.
- Wildcard providers, private/restricted export access, cfg/feature evaluation,
  build-generated modules, proc macros, edition-specific resolution, raw/Rust-only
  path-literal escapes, and explicit `package.workspace` selection remain open.
- One physical file is not yet several module instances. Conflicting ownership
  abstains conservatively; the graph must gain target-specific instance identities.
- Rust qualified/imported impl owners, trait dispatch, generic return expressions,
  and full IDE definition/reference APIs are not implemented by this slice.
- No whole-corpus recall, precision, speed, or token-efficiency improvement is
  claimed from these fixture results.
