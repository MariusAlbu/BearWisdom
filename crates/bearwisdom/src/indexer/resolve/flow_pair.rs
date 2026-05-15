// =============================================================================
// indexer/resolve/flow_pair.rs — Producer/Consumer pairing + flow_edges writer
//
// Cross-file pairing pipeline for FlowEmissions collected during the resolve
// loop. Pairs `NamedChannel { Producer, .. } ↔ NamedChannel { Consumer, .. }` by
// `(kind, normalized_name)` with HTTP-method and streaming-kind
// compatibility filtering, plus wildcard / segment-wildcard fallback passes.
// Pairs `DbQuery` and `MigrationTarget` against `DbEntity` by entity name
// with pluralisation tolerance. Writes everything else as single-ended
// flow_edges rows.
// =============================================================================

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::connectors::url_pattern;

use super::flow_emit::{self, FlowEmission};

/// Streaming-kind compatibility for an RPC pair.
///
/// Returns true when:
/// - Both sides are `None` (non-RPC, or RPC where streaming wasn't inferred).
/// - One side is `None` and the other is `Some(Unary)` (back-compat with
///   detectors that haven't been wired for streaming yet).
/// - Both sides are `Some(StreamKind::Unary)` (canonical unary pair).
/// - Both sides carry the same explicit streaming kind.
///
/// Rejects pairs where one end is unary and the other is a streaming kind,
/// or where the two ends are different streaming kinds (server-stream vs
/// client-stream, etc.).
pub(super) fn streaming_kinds_compatible(
    producer: Option<flow_emit::StreamKind>,
    consumer: Option<flow_emit::StreamKind>,
) -> bool {
    use flow_emit::StreamKind;
    match (producer, consumer) {
        (None, None) => true,
        (None, Some(StreamKind::Unary)) | (Some(StreamKind::Unary), None) => true,
        (Some(p), Some(c)) => p == c,
        _ => false,
    }
}

/// Write resolver-emitted flow edges to the `flow_edges` table.
///
/// Each emission carries a file path (not a DB id — the file may not have
/// been inserted yet when the parallel section ran). This function batches
/// a `SELECT id FROM files WHERE path IN (...)` to resolve the ids, then:
///
/// 1. Pairs `NamedChannel { Producer, .. }` ↔ `NamedChannel { Consumer, .. }` by
///    `(kind, normalized_name)` across files, with HTTP method compatibility
///    filtering.  URL patterns are normalized via `url_pattern::normalize`
///    before keying so `:id`, `<id>`, `{id}` all compare equal to `{}`.
/// 2. Pairs `DbQuery` ↔ `DbEntity` and `MigrationTarget` ↔ `DbEntity` by
///    entity/table name with case-insensitive pluralization tolerance.
///    A single `DbEntity` can accumulate many `DbQuery` and many
///    `MigrationTarget` partners — one `flow_edges` row per pair.
/// 3. Writes single-ended rows for every unpaired emission and all single-
///    ended variants (DiBinding, ConfigLookup, FeatureFlag, AuthGuard,
///    CliCommand, ScheduledJob).
pub(super) fn flush_flow_emissions(
    conn: &rusqlite::Connection,
    emissions: &[(String, u32, FlowEmission)],
) -> Result<u32> {
    if emissions.is_empty() {
        return Ok(0);
    }

    // Batch-look up file_ids for all distinct paths.
    let mut path_to_id: HashMap<&str, i64> = HashMap::new();
    {
        let paths: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            emissions.iter()
                .map(|(p, _, _)| p.as_str())
                .filter(|p| seen.insert(*p))
                .collect()
        };
        for chunk in paths.chunks(256) {
            let placeholders: String = (0..chunk.len())
                .map(|i| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("SELECT id, path FROM files WHERE path IN ({placeholders})");
            let mut stmt = conn.prepare_cached(&sql)
                .context("Failed to prepare file_id lookup for flow emissions")?;
            let params: Vec<_> = chunk.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))
                .context("Failed to query file_ids for flow emissions")?;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let path: String = row.get(1)?;
                path_to_id.entry(
                    emissions.iter()
                        .find(|(p, _, _)| p == &path)
                        .map(|(p, _, _)| p.as_str())
                        .unwrap_or_default(),
                ).or_insert(id);
            }
        }
    }

    use flow_emit::{ChannelRole, FlowEmission};

    // Resolve each emission to its file_id (skip those whose file isn't in the DB).
    struct Resolved<'a> {
        file_id: i64,
        line: u32,
        emission: &'a FlowEmission,
    }
    let resolved: Vec<Resolved<'_>> = emissions.iter()
        .filter_map(|(path, line, emission)| {
            path_to_id.get(path.as_str()).map(|&file_id| Resolved { file_id, line: *line, emission })
        })
        .collect();

    // Pre-resolve every (file_id, line) appearing in `resolved` to the
    // qualified name of the innermost enclosing function/method/constructor.
    // `query::full_trace::FlowJumpMap` keys flow_edges by this name to link
    // them into the trace tree — leaving it NULL (as the previous writer did)
    // means the trace view never picks up cross-process / cross-language
    // jumps even when 70k+ flow_edges exist.
    let enclosing_qname: HashMap<(i64, u32), String> = {
        let mut stmt = conn
            .prepare_cached(
                "SELECT qualified_name FROM symbols
                 WHERE file_id = ?1
                   AND line <= ?2
                   AND (end_line IS NULL OR end_line >= ?2)
                   AND kind IN ('function', 'method', 'constructor')
                 ORDER BY line DESC
                 LIMIT 1",
            )
            .context("Failed to prepare enclosing-function lookup for flow emissions")?;
        // Fallback: nearest function/method whose declaration starts within
        // FALLBACK_LOOKAHEAD lines AFTER the emission line. Decorator
        // emissions (NestJS @Get, TypeORM @Entity) sit ON the decorator
        // line; the method/class they apply to starts on the following line,
        // outside the enclosing-range query above. The lookahead recovers
        // that owner.
        let mut following_stmt = conn
            .prepare_cached(
                "SELECT qualified_name FROM symbols
                 WHERE file_id = ?1
                   AND line > ?2
                   AND line <= ?2 + 20
                   AND kind IN ('function', 'method', 'constructor', 'class', 'struct', 'interface')
                 ORDER BY line ASC
                 LIMIT 1",
            )
            .context("Failed to prepare following-symbol lookup for flow emissions")?;
        let mut map: HashMap<(i64, u32), String> = HashMap::new();
        let mut seen: std::collections::HashSet<(i64, u32)> = Default::default();
        for r in &resolved {
            let key = (r.file_id, r.line);
            if !seen.insert(key) { continue; }
            let qn = stmt
                .query_row(rusqlite::params![r.file_id, r.line], |row| row.get::<_, String>(0))
                .or_else(|_| {
                    following_stmt.query_row(
                        rusqlite::params![r.file_id, r.line],
                        |row| row.get::<_, String>(0),
                    )
                })
                .ok();
            if let Some(qn) = qn {
                map.insert(key, qn);
            }
        }
        map
    };
    let qname_for = |file_id: i64, line: u32| -> Option<&str> {
        enclosing_qname.get(&(file_id, line)).map(String::as_str)
    };

    // -----------------------------------------------------------------------
    // Phase 1: pair NamedChannel Producer ↔ Consumer.
    //
    // Key: (edge_type_str, normalized_name).  URL patterns are normalized
    // before keying so `:id`, `<id>`, `{id}`, and `{}` all hash to the same
    // bucket.  HTTP method compatibility is checked per-pair inside the loop
    // rather than in the key, allowing `Any` to match any concrete method.
    // -----------------------------------------------------------------------
    let mut named_channel_paired: std::collections::HashSet<usize> = Default::default();

    {
        // Pre-normalize every NamedChannel name to avoid allocating inside the
        // nested loop.  Index: emission index → normalized name.
        let mut normalized_names: HashMap<usize, String> = HashMap::new();
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::NamedChannel { name, .. } = r.emission {
                if !name.is_empty() {
                    normalized_names.insert(idx, url_pattern::normalize(name));
                }
            }
        }

        // Build producer/consumer buckets keyed by (edge_type_str, normalized_name).
        let mut producers: HashMap<(&str, &str), Vec<usize>> = Default::default();
        let mut consumers: HashMap<(&str, &str), Vec<usize>> = Default::default();
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::NamedChannel { kind, role, .. } = r.emission {
                if let Some(norm) = normalized_names.get(&idx) {
                    let key = (kind.edge_type_str(), norm.as_str());
                    match role {
                        ChannelRole::Producer => producers.entry(key).or_default().push(idx),
                        ChannelRole::Consumer => consumers.entry(key).or_default().push(idx),
                    }
                }
            }
        }

        let mut pair_stmt = conn
            .prepare_cached(
                "INSERT OR IGNORE INTO flow_edges
                    (source_file_id, source_line, source_symbol,
                     target_file_id, target_line, target_symbol,
                     edge_type, protocol, http_method, url_pattern, confidence, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .context("Failed to prepare paired flow_edges INSERT")?;

        // (source_file_id, source_line, target_file_id, target_line, edge_type)
        // — guards against duplicate edges when the same emission appears
        // multiple times in the resolved list. `edge_type_str()` returns
        // `&'static str` so the lifetime is explicit here for the wildcard
        // pair helper closure that takes this set as an argument.
        let mut seen_pairs: std::collections::HashSet<(i64, u32, i64, u32, &'static str)> =
            Default::default();

        // Helper: pair one producer emission with one consumer emission. Runs
        // the same method-compat / dedup / INSERT logic the exact-match and
        // wildcard passes share. Returns Ok(()) regardless of insert outcome;
        // pair tracking is updated via the captured `seen_pairs` /
        // `named_channel_paired` sets.
        let mut pair_one = |pi: usize,
                            ci: usize,
                            url_pattern_override: Option<&str>,
                            seen_pairs: &mut std::collections::HashSet<(i64, u32, i64, u32, &'static str)>,
                            named_channel_paired: &mut std::collections::HashSet<usize>|
         -> Result<()> {
            let pr = &resolved[pi];
            let cr = &resolved[ci];
            let FlowEmission::NamedChannel { kind, method: prod_method, name, streaming: prod_streaming, .. } = pr.emission else {
                return Ok(());
            };
            let (cons_method, cons_streaming) = if let FlowEmission::NamedChannel { method, streaming, .. } = &cr.emission {
                (method.map(|m| m.as_str()), *streaming)
            } else {
                (None, None)
            };
            if !url_pattern::http_methods_compatible(
                prod_method.map(|m| m.as_str()),
                cons_method,
            ) {
                return Ok(());
            }
            // Streaming compatibility for RPC pairs: a unary producer must
            // not pair with a streaming consumer (or vice versa). `None`
            // and `Some(Unary)` are treated as equivalent so existing
            // detectors that don't set streaming still pair correctly.
            if !streaming_kinds_compatible(*prod_streaming, cons_streaming) {
                return Ok(());
            }
            let edge_type = kind.edge_type_str();
            let pair_key = (pr.file_id, pr.line, cr.file_id, cr.line, edge_type);
            if !seen_pairs.insert(pair_key) {
                named_channel_paired.insert(pi);
                named_channel_paired.insert(ci);
                return Ok(());
            }
            let url = url_pattern_override.unwrap_or(name.as_str());
            // Stream-aware metadata: when the producer or consumer carries
            // a non-unary StreamKind, record it on the edge. Unary RPC and
            // non-RPC edges get NULL metadata.
            let metadata = (*prod_streaming).or(cons_streaming).and_then(|s| {
                if s == crate::indexer::resolve::flow_emit::StreamKind::Unary {
                    None
                } else {
                    Some(s.as_str().to_string())
                }
            });
            let n = pair_stmt.execute(rusqlite::params![
                pr.file_id, pr.line, qname_for(pr.file_id, pr.line),
                cr.file_id, cr.line, qname_for(cr.file_id, cr.line),
                edge_type, kind.protocol_str(),
                prod_method.map(|m| m.as_str()),
                Some(url),
                0.9_f64,
                metadata,
            ]).context("Failed to insert paired flow_edge")?;
            if n > 0 {
                named_channel_paired.insert(pi);
                named_channel_paired.insert(ci);
            }
            Ok(())
        };

        // Exact-match pass.
        for (key, prod_idxs) in &producers {
            if let Some(cons_idxs) = consumers.get(key) {
                for &pi in prod_idxs {
                    for &ci in cons_idxs {
                        pair_one(pi, ci, None, &mut seen_pairs, &mut named_channel_paired)?;
                    }
                }
            }
        }

        // Wildcard pass: bucket key `<prefix>/*` on one role matches every
        // bucket whose key starts with `<prefix>/` on the OTHER role. Used for
        // background-job Consumer constructors (`new Worker('queue', …)` →
        // `queue/*`) that should pair with every concrete `queue.add('job', …)`
        // Producer for the same queue. Bidirectional — a wildcard Producer
        // would pair with concrete Consumers symmetrically, though no current
        // emitter produces that shape.
        //
        // Bucket keys are `(edge_type_str, normalized_name)`. Concrete and
        // wildcard entries live in different buckets, so this pass is purely
        // additive over Phase 1 exact matching.
        // Route-parameter segment matching: `/users/{}/posts` matches
        // `/users/123/posts` segment-by-segment. Segment counts must be
        // equal; `{}` segments accept any non-empty literal segment.
        fn segment_wildcard_matches(wild_key: &str, concrete_key: &str) -> bool {
            let w_segs: Vec<&str> = wild_key.split('/').collect();
            let c_segs: Vec<&str> = concrete_key.split('/').collect();
            if w_segs.len() != c_segs.len() {
                return false;
            }
            for (w, c) in w_segs.iter().zip(c_segs.iter()) {
                if w == c {
                    continue;
                }
                if *w == "{}" && !c.is_empty() {
                    continue;
                }
                return false;
            }
            true
        }

        let bucket_matches_wildcard = |wild_key: &str, concrete_key: &str| -> bool {
            // wild_key ends with `/*`, concrete_key is everything else.
            let Some(prefix) = wild_key.strip_suffix("/*") else {
                return false;
            };
            if prefix.is_empty() || concrete_key == wild_key {
                return false;
            }
            // Match `<prefix>/<anything-non-empty>` — including another
            // wildcard like `<prefix>/*/sub` if it ever appears.
            concrete_key
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
                .map_or(false, |tail| !tail.is_empty())
        };

        // Wildcard Producer × concrete Consumer.
        let wildcard_producer_keys: Vec<(&str, &str)> = producers
            .keys()
            .filter(|(_, name)| name.ends_with("/*"))
            .copied()
            .collect();
        for wkey in wildcard_producer_keys {
            let prod_idxs = producers.get(&wkey).cloned().unwrap_or_default();
            for (ckey, cons_idxs) in &consumers {
                if ckey.0 != wkey.0 {
                    continue;
                }
                if !bucket_matches_wildcard(wkey.1, ckey.1) {
                    continue;
                }
                for &pi in &prod_idxs {
                    for &ci in cons_idxs {
                        pair_one(
                            pi,
                            ci,
                            Some(ckey.1),
                            &mut seen_pairs,
                            &mut named_channel_paired,
                        )?;
                    }
                }
            }
        }

        // Wildcard Consumer × concrete Producer.
        let wildcard_consumer_keys: Vec<(&str, &str)> = consumers
            .keys()
            .filter(|(_, name)| name.ends_with("/*"))
            .copied()
            .collect();
        for wkey in wildcard_consumer_keys {
            let cons_idxs = consumers.get(&wkey).cloned().unwrap_or_default();
            for (pkey, prod_idxs) in &producers {
                if pkey.0 != wkey.0 {
                    continue;
                }
                if !bucket_matches_wildcard(wkey.1, pkey.1) {
                    continue;
                }
                for &pi in prod_idxs {
                    for &ci in &cons_idxs {
                        pair_one(
                            pi,
                            ci,
                            Some(pkey.1),
                            &mut seen_pairs,
                            &mut named_channel_paired,
                        )?;
                    }
                }
            }
        }

        // Segment-wildcard pass: a Consumer URL containing `{}` segments
        // pairs against any Producer URL whose path matches when those
        // positions are filled by concrete values. Implements the natural
        // route-parameter matching semantics — `/api/trpc/{}` Consumer
        // matches `/api/trpc/polls.list`, `/users/{}/posts` matches
        // `/users/123/posts`, etc.
        //
        // Only runs when the Consumer key contains `{}` AND the Producer key
        // doesn't; the both-sides-already-{} case is handled by the
        // exact-match pass above (template literals normalize to the same
        // `{}` form).
        let segment_wildcard_consumer_keys: Vec<(&str, &str)> = consumers
            .keys()
            .filter(|(_, name)| name.contains("/{}"))
            .copied()
            .collect();
        for wkey in segment_wildcard_consumer_keys {
            let cons_idxs = consumers.get(&wkey).cloned().unwrap_or_default();
            for (pkey, prod_idxs) in &producers {
                if pkey.0 != wkey.0 {
                    continue;
                }
                if pkey.1 == wkey.1 {
                    continue; // Exact-match pass handled this.
                }
                if !segment_wildcard_matches(wkey.1, pkey.1) {
                    continue;
                }
                for &pi in prod_idxs {
                    for &ci in &cons_idxs {
                        pair_one(
                            pi,
                            ci,
                            Some(pkey.1),
                            &mut seen_pairs,
                            &mut named_channel_paired,
                        )?;
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 2: pair DbQuery ↔ DbEntity and MigrationTarget ↔ DbEntity.
    //
    // Entity index keys are the canonical names from DbEntity (both the
    // table_name_hint when present, and the base_name_hint as fallback).
    // Matching is case-insensitive with simple suffix-s pluralization
    // tolerance via `url_pattern::entity_names_match`.
    //
    // A single DbEntity can appear in many pairs — one flow_edge row per
    // (query/migration, entity) combination.
    // -----------------------------------------------------------------------
    let mut db_paired: std::collections::HashSet<usize> = Default::default();

    {
        // Build entity list: (key, idx) pairs.  A DbEntity contributes two
        // entries when both table_name_hint and base_name_hint are non-empty,
        // making it reachable under either name.
        let mut entity_entries: Vec<(&str, usize)> = Vec::new();
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::DbEntity { table_name_hint, base_name_hint, .. } = r.emission {
                if let Some(tname) = table_name_hint.as_deref() {
                    if !tname.is_empty() {
                        entity_entries.push((tname, idx));
                    }
                }
                if !base_name_hint.is_empty() {
                    entity_entries.push((base_name_hint.as_str(), idx));
                }
            }
        }

        let mut pair_stmt = conn
            .prepare_cached(
                "INSERT OR IGNORE INTO flow_edges
                    (source_file_id, source_line, source_symbol,
                     target_file_id, target_line, target_symbol,
                     edge_type, url_pattern, confidence, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .context("Failed to prepare db-paired flow_edges INSERT")?;

        // Helper: find entity indices whose key matches `name` via
        // case-insensitive pluralization-tolerant comparison.
        let find_entities = |name: &str| -> Vec<usize> {
            let mut seen = std::collections::HashSet::new();
            entity_entries.iter()
                .filter(|(key, _)| url_pattern::entity_names_match(name, key))
                .map(|(_, idx)| *idx)
                .filter(|idx| seen.insert(*idx))
                .collect()
        };

        // Pair DbQuery ↔ DbEntity.
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::DbQuery { entity_name, operation } = r.emission {
                if entity_name.is_empty() { continue; }
                for ei in find_entities(entity_name) {
                    let er = &resolved[ei];
                    let n = pair_stmt.execute(rusqlite::params![
                        r.file_id, r.line, qname_for(r.file_id, r.line),
                        er.file_id, er.line, qname_for(er.file_id, er.line),
                        "db_query",
                        Some(entity_name.as_str()),
                        0.85_f64,
                        Some(operation.as_str()),
                    ]).context("Failed to insert db-query flow_edge")?;
                    if n > 0 {
                        db_paired.insert(idx);
                        db_paired.insert(ei);
                    }
                }
            }
        }

        // Pair MigrationTarget ↔ DbEntity.
        for (idx, r) in resolved.iter().enumerate() {
            if let FlowEmission::MigrationTarget { table_name, direction } = r.emission {
                if table_name.is_empty() { continue; }
                for ei in find_entities(table_name) {
                    let er = &resolved[ei];
                    let n = pair_stmt.execute(rusqlite::params![
                        r.file_id, r.line, qname_for(r.file_id, r.line),
                        er.file_id, er.line, qname_for(er.file_id, er.line),
                        "migration_target",
                        Some(table_name.as_str()),
                        0.85_f64,
                        Some(direction.as_str()),
                    ]).context("Failed to insert migration-target flow_edge")?;
                    if n > 0 {
                        db_paired.insert(idx);
                        db_paired.insert(ei);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 3: write single-ended rows for everything not already paired.
    // -----------------------------------------------------------------------
    let mut single_stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO flow_edges
                (source_file_id, source_line, source_symbol,
                 edge_type, protocol, http_method, url_pattern, confidence, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .context("Failed to prepare single-ended flow_edges INSERT")?;

    let mut written = named_channel_paired.len() as u32 + db_paired.len() as u32;

    for (idx, r) in resolved.iter().enumerate() {
        if named_channel_paired.contains(&idx) || db_paired.contains(&idx) {
            continue;
        }
        let n = single_stmt.execute(rusqlite::params![
            r.file_id,
            r.line,
            qname_for(r.file_id, r.line),
            r.emission.edge_type(),
            r.emission.protocol(),
            r.emission.http_method_str(),
            r.emission.url_pattern(),
            0.9_f64,
            r.emission.streaming_str(),
        ]).context("Failed to insert single-ended flow_edge")?;
        written += n as u32;
    }

    Ok(written)
}

#[cfg(test)]
pub(crate) fn _test_flush_flow_emissions(
    conn: &rusqlite::Connection,
    emissions: &[(String, u32, FlowEmission)],
) -> Result<u32> {
    flush_flow_emissions(conn, emissions)
}

/// Public entry to the pairing/flush pipeline. Used by post-connector
/// passes (see `full.rs`) that materialise late-arriving DB rows into
/// flow edges. Same semantics as the resolver's own flush.
pub fn flush_flow_emissions_public(
    conn: &rusqlite::Connection,
    emissions: &[(String, u32, FlowEmission)],
) -> Result<u32> {
    flush_flow_emissions(conn, emissions)
}
