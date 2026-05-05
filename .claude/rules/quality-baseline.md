---
paths:
  - "baseline-all.json"
  - "crates/bearwisdom/src/quality/**/*.rs"
  - "crates/bearwisdom-cli/src/main.rs"
---

# Quality baseline

The repository tracks **one** baseline file: `baseline-all.json` (240
projects across the corpus). Every project the indexer is benchmarked
against lives there, with assertion thresholds (`min_resolution_rate`,
`min_routes`, `min_flow_edges`, …) attached to each entry.

## Subset reindexes — DO NOT create new baseline files

When iterating on a fix that only affects some projects, do NOT extract
those projects into a separate baseline file. Use the `--project` flag
to scope the run; the tool writes back into `baseline-all.json` with
only the targeted entries refreshed and every other entry preserved
in place.

```bash
# Refresh every project (slow):
bw quality-check --recapture

# Refresh only the projects affected by a TS fix:
bw quality-check --recapture \
  --project ts-nextjs \
  --project python-paperless-ngx \
  --project vue-hoppscotch

# Compare current index state to baseline (no reindex):
bw quality-check

# Reindex + compare (catches indexing regressions):
bw quality-check --reindex
```

`.gitignore` enforces this: any `baseline-*.json` other than
`baseline-all.json` is ignored, so subset files can't accidentally land
in commits.
