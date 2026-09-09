1. [generic] Capture methods, fields and nominal declarations' access scopes against exact source declaration slots, independently of module exports.
2. [profile data] Describe access-bearing nodes and implicitly public member containers with namespace syntax Forms; no language branch in member selection.
3. [generic] Persist declaration-access recipes and source-module byte spans with the module graph's source fingerprints; advance the binding epoch.
4. [generic] Reuse numeric public/private/restricted scope lowering and module ancestry for member declaration IDs.
5. [generic] Build each file's source-position-to-module-ID context once; reference cursor changes select the requesting module without path strings.
6. [generic] Check both the nominal owner and selected member declaration for accessibility; no-context/private and unknown restrictions cannot grant access.
7. [generic] Keep inaccessible candidates present as barriers, not filtered-away absences that expose inherited namesakes.
8. [generic] Propagate access rejection through the nominal chain so denied methods/fields cannot seed downstream yields or fallback targets.
9. [generic] Test private/restricted/public sibling and descendant access, same-file position isolation, cross-file callers, aliases, fields, factories and cold reload; validate fixture semantics with rustc.
10. [generic] Retain explicit gaps for trait dispatch, conditional module instances, every legacy unbound path and complete compiler diagnostics; no inferred 99% claim.
11. [generic] Verified slice: 48 method/field fixtures cover private, self, super, named-ancestor, crate and public scopes at four caller positions; rustc independently validates acceptance/privacy diagnostics, not target labels. Exact declaration IDs are asserted separately on fresh and persisted snapshots.
12. [generic] Retained failing probe: `impl crate::RealDoc` in a separate source file has no nominal owner attachment. The existing detached-body binder only attaches supported local identifiers; do not infer ownership from the printed type name or count a rejected legal call as privacy correctness.
13. [profile data] Trait members and trait-impl members have implicit-public access metadata, but this does not implement trait dispatch. Enum payload fields currently lack extracted declaration slots. Construction/pattern access validation and dedicated inaccessible diagnostics remain separate work.
