-- ============================================================
-- PER-REF RESOLUTION LOG  (resolution snapshot instrument)
-- ============================================================

-- One row per ref SITE the resolve pipeline processes, written alongside
-- `edges` / `unresolved_refs` at flush time. Unlike `edges` — keyed on
-- (source_id, target_id, kind, source_line), which collapses distinct ref
-- sites that happen to resolve to the same target on the same line — and
-- `unresolved_refs` — which never carries the literal referenced name for a
-- RESOLVED outcome — this table preserves the (file, line, col, target_name,
-- kind) identity of every ref alongside its outcome, so two index runs of
-- the same project can be diffed at ref-site granularity (see
-- `query::ref_snapshot` / `query::resolve_diff`).
--
-- `outcome` is 'resolved' | 'unresolved' | 'drained'. `target_id`,
-- `confidence`, and `strategy` are populated only for 'resolved' rows.
CREATE TABLE IF NOT EXISTS ref_resolutions (
    id          INTEGER PRIMARY KEY,
    source_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    target_name TEXT    NOT NULL,
    kind        TEXT    NOT NULL,
    source_line INTEGER NOT NULL,
    source_col  INTEGER NOT NULL,
    outcome     TEXT    NOT NULL,
    target_id   INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    confidence  REAL,
    strategy    TEXT,
    source_byte INTEGER, -- NULL on legacy rows; byte offset zero is valid
    source_selector_byte INTEGER -- Called identifier anchor, NULL on legacy rows
);

CREATE INDEX IF NOT EXISTS idx_ref_resolutions_source ON ref_resolutions(source_id);
CREATE INDEX IF NOT EXISTS idx_ref_resolutions_target ON ref_resolutions(target_id);

-- Per-file exhaustive occurrence census. Missing rows mean unmeasured, not zero.
-- Aggregates are replaced atomically with resolution output; no per-ref duplication.
CREATE TABLE IF NOT EXISTS resolution_census (
    file_id      INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    counts_json  TEXT NOT NULL
);
