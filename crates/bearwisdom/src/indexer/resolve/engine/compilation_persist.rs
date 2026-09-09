// =============================================================================
// engine/compilation_persist — symbol_type_info persistence
//
// Writes the Compilation's resolved per-symbol type metadata to the
// `symbol_type_info` table so an incremental pass loads back exactly the full
// pass's ref+signature-derived type info for unchanged symbols. The read side
// (`ingest_from_db`) stays with the index-building code in `compilation.rs`;
// this module owns only the write boundary.
// =============================================================================

use super::compilation::{json_string_array, Compilation};

impl Compilation {
    /// Persist per-symbol resolved type metadata to `symbol_type_info`, so an
    /// incremental pass loads back exactly the full pass's ref+signature-derived
    /// type info for unchanged symbols (not the lossy signature-only
    /// re-derivation). Replaces the table wholesale. Only entries that map to a
    /// DB symbol id are written; the simple-name `generic_params` duplicate keys
    /// are rebuilt from these rows on load.
    pub(crate) fn persist_type_info(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM symbol_type_info", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO symbol_type_info \
                 (symbol_id, generic_params, field_type_id, return_type_id) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (qname, ti) in &self.type_info {
                let Some(sym) = self.by_qname.get(qname) else {
                    continue;
                };
                let param_names: Vec<String> = ti
                    .generic_param_ids
                    .iter()
                    .map(|&id| self.arena.generic_param(id).name)
                    .collect();
                if ti.field_type_id.is_none()
                    && ti.return_type_id.is_none()
                    && param_names.is_empty()
                {
                    continue;
                }
                stmt.execute(rusqlite::params![
                    sym.id,
                    json_string_array(&param_names),
                    ti.field_type_id.map(|t| t.0.get() as i64),
                    ti.return_type_id.map(|t| t.0.get() as i64),
                ])?;
            }
        }
        // Per-id rows from `type_info_by_id`. The qname loop above writes one row
        // per qualified name (first-winner), so a same-qname overload set — every
        // `HttpClient.get`, every `inject` — persists only one declaration's
        // return. Upsert each symbol id's OWN field/return here so an incremental
        // reload restores the overload-accurate metadata the resolver reads by id;
        // COALESCE keeps the qname loop's `generic_params` / type-id columns.
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbol_type_info \
                 (symbol_id, generic_params, field_type_id, return_type_id) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(symbol_id) DO UPDATE SET \
                   generic_params = COALESCE(excluded.generic_params, generic_params), \
                   field_type_id = COALESCE(excluded.field_type_id, field_type_id), \
                   return_type_id = COALESCE(excluded.return_type_id, return_type_id)",
            )?;
            for (id, ti) in &self.type_info_by_id {
                let param_names: Vec<String> = ti
                    .generic_param_ids
                    .iter()
                    .map(|&id| self.arena.generic_param(id).name)
                    .collect();
                if ti.field_type_id.is_none()
                    && ti.return_type_id.is_none()
                    && param_names.is_empty()
                {
                    continue;
                }
                let generic_params =
                    (!param_names.is_empty()).then(|| json_string_array(&param_names));
                stmt.execute(rusqlite::params![
                    id,
                    generic_params,
                    ti.field_type_id.map(|t| t.0.get() as i64),
                    ti.return_type_id.map(|t| t.0.get() as i64),
                ])?;
            }
        }
        // Persist the arena verbatim so the `field_type_id` / `return_type_id`
        // raw indices stay valid: `restore_snapshot` rebuilds an identical arena.
        self.persist_lexical_type_info(&tx)?;
        tx.execute(
            "INSERT OR REPLACE INTO _bearwisdom_meta (key, value) VALUES ('type_arena_snapshot', ?1)",
            rusqlite::params![self.arena.serialize_snapshot()],
        )?;
        tx.commit()
    }
}
