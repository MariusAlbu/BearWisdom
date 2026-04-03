// =============================================================================
// db/audit.rs  —  MCP audit log — per-call records and aggregate stats
//
// All methods live on `Database` (second impl block — Rust allows this).
// The web API and SSE handler call these to read; the MCP server calls
// `write_audit_record` after every tool invocation.
// =============================================================================

use super::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One row from `mcp_audit`.
#[derive(Debug, serde::Serialize)]
pub struct AuditRecord {
    pub id: i64,
    pub session_id: String,
    pub tool_name: String,
    pub params_json: String,
    pub response_json: String,
    pub duration_ms: i64,
    pub token_estimate: i64,
    pub ts: String,
}

/// Per-session aggregate for the sidebar.
#[derive(Debug, serde::Serialize)]
pub struct AuditSessionSummary {
    pub session_id: String,
    pub call_count: i64,
    pub total_tokens: i64,
    pub first_ts: String,
    pub last_ts: String,
}

/// Aggregate stats across all sessions for the stats bar.
#[derive(Debug, serde::Serialize)]
pub struct AuditStats {
    pub total_calls: i64,
    pub total_tokens: i64,
    pub avg_duration_ms: f64,
    pub session_count: i64,
    /// `(tool_name, call_count)` sorted by call_count desc.
    pub calls_by_tool: Vec<(String, i64)>,
}

// ---------------------------------------------------------------------------
// impl Database — audit methods
// ---------------------------------------------------------------------------

impl Database {
    /// Insert one audit record.  Silently ignores the result — a failed write
    /// must not abort the MCP tool call that caused it.
    pub fn write_audit_record(
        &self,
        session_id: &str,
        tool_name: &str,
        params_json: &str,
        response_json: &str,
        duration_ms: u64,
        token_estimate: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO mcp_audit \
             (session_id, tool_name, params_json, response_json, duration_ms, token_estimate) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                session_id,
                tool_name,
                params_json,
                response_json,
                duration_ms as i64,
                token_estimate
            ],
        )?;
        Ok(())
    }

    /// All sessions, newest last-call first.
    pub fn list_audit_sessions(&self) -> rusqlite::Result<Vec<AuditSessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id,
                    COUNT(*)                        AS call_count,
                    COALESCE(SUM(token_estimate), 0) AS total_tokens,
                    MIN(ts)                          AS first_ts,
                    MAX(ts)                          AS last_ts
             FROM mcp_audit
             GROUP BY session_id
             ORDER BY last_ts DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AuditSessionSummary {
                session_id:   row.get(0)?,
                call_count:   row.get(1)?,
                total_tokens: row.get(2)?,
                first_ts:     row.get(3)?,
                last_ts:      row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Calls for one session, newest first, paginated.
    pub fn list_audit_calls(
        &self,
        session_id: &str,
        limit: i64,
        offset: i64,
    ) -> rusqlite::Result<Vec<AuditRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, params_json, response_json,
                    duration_ms, token_estimate, ts
             FROM mcp_audit
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id, limit, offset], |row| {
            Ok(AuditRecord {
                id:             row.get(0)?,
                session_id:     row.get(1)?,
                tool_name:      row.get(2)?,
                params_json:    row.get(3)?,
                response_json:  row.get(4)?,
                duration_ms:    row.get(5)?,
                token_estimate: row.get(6)?,
                ts:             row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Records with id > `after_id`, oldest first, capped at 50.
    /// Used by the SSE tail loop.
    pub fn list_new_audit_records(&self, after_id: i64) -> rusqlite::Result<Vec<AuditRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, params_json, response_json,
                    duration_ms, token_estimate, ts
             FROM mcp_audit
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT 50",
        )?;
        let rows = stmt.query_map(rusqlite::params![after_id], |row| {
            Ok(AuditRecord {
                id:             row.get(0)?,
                session_id:     row.get(1)?,
                tool_name:      row.get(2)?,
                params_json:    row.get(3)?,
                response_json:  row.get(4)?,
                duration_ms:    row.get(5)?,
                token_estimate: row.get(6)?,
                ts:             row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Aggregate stats across all sessions.
    pub fn get_audit_stats(&self) -> rusqlite::Result<AuditStats> {
        let (total_calls, total_tokens, avg_duration_ms, session_count): (i64, i64, f64, i64) =
            self.conn.query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(token_estimate), 0),
                        COALESCE(AVG(CAST(duration_ms AS REAL)), 0.0),
                        COUNT(DISTINCT session_id)
                 FROM mcp_audit",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;

        let mut stmt = self.conn.prepare(
            "SELECT tool_name, COUNT(*) AS cnt
             FROM mcp_audit
             GROUP BY tool_name
             ORDER BY cnt DESC",
        )?;
        let calls_by_tool: Vec<(String, i64)> = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(AuditStats {
            total_calls,
            total_tokens,
            avg_duration_ms,
            session_count,
            calls_by_tool,
        })
    }

    /// Delete all records for one session.  Returns the number of rows removed.
    pub fn delete_audit_session(&self, session_id: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM mcp_audit WHERE session_id = ?1",
            rusqlite::params![session_id],
        )
    }
}
