# Configured registry modules

1. [ingestion] Extend Cargo manifest ingestion, not the generic method selector or global-name fallback.
2. [ingestion] Join consumer manifest dependencies to exact lockfile package edges, retaining aliases and dependency kinds.
3. [ingestion] Validate version requirements, registry checksums, installed manifest identities and unique source locations.
4. [ingestion] Reuse Cargo target discovery and source recipes; expose installed library roots under the existing external file address scheme.
5. [generic] Feed those package records into the existing ModuleId/export/BindingId graph without semantic string comparisons.
6. [ingestion] Preserve missing, conditional and conflicting evidence as incomplete; never select the newest installed package or a namesake.
7. [identity] Reject multiple versions sharing today's legacy external virtual path; versioned source/module instances remain explicit follow-up work.
8. [snapshot] Fingerprint lockfile and provider configuration changes, and retain the resulting configuration across fresh-arena reloads.
9. [tests] Cover exact imported targets, trait availability and return cascades, aliases, provider changes, stale locks, missing metadata and version conflicts.
10. [verification] Repair the corpus fixture's absent manifests/module declarations without weakening its external-target assertions; rerun core, integrations and consumers.

## Implemented evidence path — 2026-09-07

`cargo/registry_modules` extends the existing manifest reader. It does not execute
Cargo, install packages, fetch source, or scan symbols. Each dependency must match
an exact consumer lockfile edge and a compatible version requirement; `semver` is
used for requirement semantics, not a hand-written version comparator. The installed
package must have a matching manifest name/version, unique location and matching
package-checksum metadata. Custom library names/paths, dependency aliases and dependency
kinds then enter the existing `ModulePackage`/ModuleId/export graph.

Lockfile and provider-configuration fingerprints are retained with the source package
records. Tests cover mismatched checksums/manifests, stale requirements, absent direct
edges, optional dependencies, duplicate locations, version collisions within/across
lockfiles, and unresolved named-registry configuration. The full pipeline test filters
the provider to its external contract, verifies exact trait/static and return-cascade
targets, retargets an actual provider re-export, restores a fresh arena, and deletes
the provider without reviving stale bindings.

```rust
use api::{Thing, Load};       // ✅ [ingestion + generic] alias + pinned provider → module/export IDs
fn f(p: &Thing) {
    p.load().touch();         // ✅ [generic] static Load.load ID → bound Doc → exact Doc.touch ID
}
// provider changes a::{Thing, Load, Doc} → b::{Thing, Load, Doc}
// ✅ [generic] unchanged caller recipes follow current IDs, including after cold reload
// provider deleted / incompatible pinned version
// ✅ [generic] calls remain unresolved; no namesake fallback
```

The old corpus fixture supplied library source but omitted registry manifests,
checksum metadata, the consumer lockfile edge and its caller's crate-module declaration.
Those inputs are now present; its external-call assertions were not relaxed. The
existing external seam passes, alongside the stronger exact-occurrence pipeline test.

Limitations remain explicit: today's external virtual paths omit package versions,
so colliding package identities abstain instead of sharing a source address. Full
versioned source instances, git/stdlib providers, named registry/source-replacement
configuration, active cfg/features and complete configuration-driven edge invalidation
remain roadmap work. Checksum metadata is source identity evidence, not a new security
or file-integrity attestation. These tests are not added to the compiler-labelled recall denominator.

Primary semantics: [Cargo dependency requirements and aliases](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html),
[resolved metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html), and
[source replacement](https://doc.rust-lang.org/cargo/reference/source-replacement.html).
