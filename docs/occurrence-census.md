# Occurrence census: the first 99% foundation step

`bw resolution-gate <project>` now returns `occurrences` alongside the unchanged
legacy `breakdown` and `health` fields. Rust consumers can call
`bearwisdom::occurrence_census(&db)`. This query does not re-index the project.

## Contract

The census starts with `ParsedFile.refs`, before the resolver's early exits.
Each extracted reference has exactly one disposition:

- `resolved`: the engine selected a declaration; correctness is not established.
- `unresolved`: resolution ran but did not bind.
- `drained`: the resolver classified a builtin/non-project construct.
- `primitive`: an extractor marker or primitive type reference was excluded.
- `duplicate`: the same extracted site was emitted again.
- `missing_source_symbol`: extraction provided no valid source declaration.
- `missing_source_id`: a source declaration exists but its persisted ID is absent.
- `unsupported_language`: there is no resolver profile for the host file language.

Distinct byte offsets on the same line stay distinct. The legacy `edges` table
still deduplicates by source/target/kind/line; the census never counts those edges.
The existing site-deduplication policy is unchanged by this instrumentation.

`raw` contains all current measured internal-file inputs. `code` symmetrically
excludes generated Dart source, snippets and Markdown/MDX document imports for
every disposition. Exclusion counters are disjoint (generated, then snippet,
then document import). Embedded language attribution comes from extraction.
Other generated/vendored files reclassified as external remain outside the
internal-file population, just as they do in existing reports.

Binding coverage is:

`resolved / (resolved + unresolved + missing_source_symbol + missing_source_id + unsupported_language)`

Drains, primitives and duplicates are observable, not counted as successful
bindings. A zero eligible denominator produces `null`, never 100%. Missing or
stale file measurements suppress the project percentage; partial counts remain
available with the incomplete file counts explicitly reported.

## Persistence and freshness

`resolution_census` stores one content-hashed aggregate per internal file,
including zero-reference and symbol-less files. It is written in the same
transaction as edges, unresolved references and occurrence logs. Full resolution
replaces the census; incremental resolution replaces only participating files;
file deletion cascades. A mismatched source hash is reported as stale. The
write rejects a partition whose counts do not sum to the extracted input count.

The additional table is created on a normal writable database open. Older
read-only databases without the table remain readable and report unmeasured
files. An incremental index upgrades only its participating files; a full index
is needed for a complete baseline. Do not silently re-index a large project
just to display the report.

## What this does not prove

This is **binding coverage of extracted input**, not binding precision, correct
binding recall, extraction recall, or all-language 99% accuracy. The precision
and correct-recall fields stay `null` until an independent oracle exists.
References the extractor never emitted are still outside this input population.
Unsupported-file references retain the pipeline's no-profile classification;
their internal duplicate/primitive structure has not been evaluated.

Hash freshness is source-content freshness only. Semantic dependency changes
can still leave unchanged callers stale; full/incremental semantic parity is a
separate F2 task. The old `breakdown.precision` field remains a compatibility
alias for an edge-weighted coverage metric, explicitly labelled as such in the
gate's `measurement_contract`.

No semantic string fallback was added. The next resolver step is scoped binding
identity, followed by module/declaration identity and ID-keyed flow typing.
Cross-service UI → DB flows additionally require explicit route, message, DI
and database-mapping evidence; high binding coverage alone cannot establish them.

## Verification (2026-09-05)

The same-line-call test was run red before connecting the collector, then green:
four resolved Python call occurrences remain four, while the unchanged graph has
three call edges. Synthetic pipeline fixtures cover all early exits, including
symbol-less files, absent source IDs, unsupported profiles and primitive refs.

Persistence tests cover full replacement, incremental/fresh census equality on
the fixture edits, replacement with zero references, deletion, transaction
rollback, stale/missing measurements and rejection of an unindexed file path.
An absent file must never be converted into an auto-allocated SQLite primary key.

Final library verification: 746 targeted tests passed across the engine, schema,
statistics, occurrence census and reference-snapshot modules. This is a regression
check on the touched pipeline, not a full corpus recapture or a proof that known
binding defects have been fixed. No full project/corpus index was run.

The CLI gate compatibility test also passed. File-budget and ID-discipline
checks passed, including Windows-native equivalents of the repository hooks;
`git diff --check` passed. Existing compiler warnings remain unrelated to this
slice. Sibling AlphaT/Lynx dependencies were inspected; no existing public
function or response field was removed or changed.
