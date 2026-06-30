---
paths:
  - "baseline.json"
  - "crates/bearwisdom/src/quality/**/*.rs"
  - "crates/bearwisdom-cli/src/main.rs"
---

# Quality baseline

The repository tracks **one** baseline file: `baseline.json` — the
current-engine baseline, seeded with the TS corpus and grown one project
at a time as each is brought up. Every tracked project lives there with
assertion thresholds (`min_resolution_rate`, `min_routes`,
`min_flow_edges`, …) attached to each entry. (The pre-cutover
`baseline-all.json` is retired — its rates came from the old engine and
are not a valid reference point.)

To add a project: append a `{project, path, assertions:{}}` entry, then
`bw quality-check --recapture --project <name>` to fill its metrics.

## Subset reindexes — DO NOT create new baseline files

When iterating on a fix that only affects some projects, do NOT extract
those projects into a separate baseline file. Use the `--project` flag
to scope the run; the tool writes back into `baseline.json` with
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

`.gitignore` enforces this: any `baseline-*.json` is ignored (only
`baseline.json`, with no dash, is tracked), so subset files can't
accidentally land in commits.
