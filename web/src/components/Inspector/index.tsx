import { useEffect, useRef, useCallback } from 'react'
import { api } from '../../api'
import { useAuditStore } from '../../stores/audit.store'
import type { AuditRecord } from '../../types/api.types'
import styles from './Inspector.module.css'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// Native cost multiplier per tool — rough estimate of how many more tokens
// a manual file-read approach would require vs one BW call.
const NATIVE_MULTIPLIER: Record<string, number> = {
  bw_search: 8,
  bw_grep: 3,
  bw_find_references: 12,
  bw_blast_radius: 15,
  bw_call_hierarchy: 10,
  bw_symbol_info: 5,
  bw_file_symbols: 4,
  bw_architecture_overview: 20,
  bw_investigate: 20,
  bw_diagnostics: 4,
  bw_complete: 3,
  bw_context: 20,
}

function nativeEstimate(record: AuditRecord): number {
  const mult = NATIVE_MULTIPLIER[record.tool_name] ?? 6
  return record.token_estimate * mult
}

// Map tool name → badge category class
function toolCategory(name: string): string {
  if (['bw_search', 'bw_grep'].includes(name)) return 'badge-search'
  if (['bw_symbol_info', 'bw_find_references', 'bw_file_symbols', 'bw_definition'].includes(name))
    return 'badge-nav'
  if (['bw_blast_radius', 'bw_call_hierarchy', 'bw_investigate', 'bw_diagnostics'].includes(name))
    return 'badge-analysis'
  if (['bw_architecture_overview'].includes(name)) return 'badge-flow'
  return 'badge-context'
}

function shortTool(name: string): string {
  return name.replace('bw_', '')
}

function formatTs(ts: string): string {
  return ts.split('T')[1]?.split('.')[0] ?? ts.slice(-8)
}

function fmtMs(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
}

function fmtK(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n)
}

// Very lightweight JSON syntax highlighter.
function highlight(raw: string): string {
  try {
    const pretty = JSON.stringify(JSON.parse(raw), null, 2)
    return pretty
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/("(?:[^"\\]|\\.)*")(\s*:)/g, '<span class="jk">$1</span>$2')
      .replace(/:\s*("(?:[^"\\]|\\.)*")/g, ': <span class="js">$1</span>')
      .replace(/:\s*(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g, ': <span class="jn">$1</span>')
      .replace(/:\s*(true|false|null)/g, ': <span class="jb">$1</span>')
  } catch {
    return raw.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  }
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function ToolBadge({ name }: { name: string }) {
  const cat = toolCategory(name)
  return (
    <span className={`${styles.toolBadge} ${styles[cat]}`}>{shortTool(name)}</span>
  )
}

function CallRow({ record, active }: { record: AuditRecord; active: boolean }) {
  const setSelected = useAuditStore((s) => s.setSelectedCall)
  const bwTokens = record.token_estimate
  const nativeTokens = nativeEstimate(record)
  const maxTokens = Math.max(nativeTokens, 1)

  return (
    <div
      className={`${styles.callRow}${active ? ' ' + styles.callRowActive : ''}`}
      onClick={() => setSelected(active ? null : record.id)}
    >
      <ToolBadge name={record.tool_name} />
      <div className={styles.callTokenBar}>
        <div
          className={styles.callTokenBw}
          style={{ width: `${Math.min(100, (bwTokens / maxTokens) * 100)}%` }}
          title={`BW: ${bwTokens} tokens`}
        />
        <div
          className={styles.callTokenNative}
          style={{ width: `${Math.min(100, (nativeTokens / maxTokens) * 100)}%` }}
          title={`Native: ~${nativeTokens} tokens`}
        />
      </div>
      <span className={styles.callDuration}>{fmtMs(record.duration_ms)}</span>
      <span className={styles.callTs}>{formatTs(record.ts)}</span>
    </div>
  )
}

function DetailPanel({ record }: { record: AuditRecord | null }) {
  if (!record) {
    return (
      <div className={styles.detail}>
        <div className={styles.detailEmpty}>
          Select a call to inspect params and response
        </div>
      </div>
    )
  }

  const bwTokens = record.token_estimate
  const nativeTokens = nativeEstimate(record)
  const saved = nativeTokens - bwTokens
  const savedPct = nativeTokens > 0 ? Math.round((saved / nativeTokens) * 100) : 0

  return (
    <div className={styles.detail}>
      <div className={styles.detailHeader}>
        <div className={styles.detailTool}>{record.tool_name}</div>
        <div className={styles.detailMeta}>
          {formatTs(record.ts)} · {fmtMs(record.duration_ms)} · session {record.session_id.slice(0, 8)}…
        </div>
      </div>
      <div className={styles.detailBody}>
        <div className={styles.detailSection}>
          <div className={styles.detailSectionLabel}>Parameters</div>
          <div
            className={styles.jsonBlock}
            dangerouslySetInnerHTML={{ __html: highlight(record.params_json) }}
          />
        </div>
        <div className={styles.detailSection}>
          <div className={styles.detailSectionLabel}>Response</div>
          <div
            className={styles.jsonBlock}
            dangerouslySetInnerHTML={{ __html: highlight(record.response_json) }}
          />
        </div>
        <div className={styles.detailSection}>
          <div className={styles.detailSectionLabel}>Token comparison</div>
          <table className={styles.tokenTable}>
            <tbody>
              <tr>
                <td>BearWisdom</td>
                <td>{fmtK(bwTokens)} tokens</td>
              </tr>
              <tr className={styles.nativeRow}>
                <td>Native (estimated)</td>
                <td>~{fmtK(nativeTokens)} tokens</td>
              </tr>
              <tr>
                <td>Saved</td>
                <td className={styles.tokenSavings}>
                  ~{fmtK(saved)} ({savedPct}%)
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export interface InspectorProps {
  workspacePath: string
}

export function Inspector({ workspacePath }: InspectorProps) {
  const {
    sessions,
    activeSessionId,
    calls,
    stats,
    selectedCallId,
    loadingCalls,
    setSessions,
    setActiveSession,
    setCalls,
    prependCalls,
    setStats,
    setLoadingCalls,
    setError,
    removeSession,
  } = useAuditStore()

  const sseRef = useRef<EventSource | null>(null)

  // Load sessions + stats on mount.
  useEffect(() => {
    async function load() {
      try {
        const [s, st] = await Promise.all([
          api.auditSessions(workspacePath),
          api.auditStats(workspacePath),
        ])
        setSessions(s)
        setStats(st)
      } catch {
        // Index may not exist yet — ignore silently.
      }
    }
    load()
  }, [workspacePath, setSessions, setStats])

  // Start SSE stream on mount.
  useEffect(() => {
    const es = api.auditStream(workspacePath)
    sseRef.current = es

    es.onmessage = (evt) => {
      try {
        const records: AuditRecord[] = JSON.parse(evt.data)
        if (records.length > 0) {
          prependCalls(records)
          // Re-fetch sessions + stats to get accurate counts from the DB
          // rather than incrementally tracking in-memory (which double-counts
          // when SSE replays records not yet in the calls array).
          Promise.all([
            api.auditSessions(workspacePath).then(setSessions),
            api.auditStats(workspacePath).then(setStats),
          ]).catch(() => {})
        }
      } catch {
        // Ignore parse errors.
      }
    }

    return () => {
      es.close()
      sseRef.current = null
    }
  }, [workspacePath, prependCalls, setStats])

  // Load calls when active session changes.
  useEffect(() => {
    if (!activeSessionId) return
    setLoadingCalls(true)
    api
      .auditCalls(workspacePath, activeSessionId)
      .then(setCalls)
      .catch((e: Error) => setError(e.message))
  }, [activeSessionId, workspacePath, setCalls, setLoadingCalls, setError])

  const handleDeleteSession = useCallback(
    async (e: React.MouseEvent, sessionId: string) => {
      e.stopPropagation()
      try {
        await api.deleteAuditSession(workspacePath, sessionId)
        removeSession(sessionId)
      } catch {
        // Ignore.
      }
    },
    [workspacePath, removeSession],
  )

  const selectedCall = calls.find((c) => c.id === selectedCallId) ?? null

  // Compute max tokens across sessions for the sidebar bar width.
  const maxSessionTokens = Math.max(...sessions.map((s) => s.total_tokens), 1)

  // Stats bar values.
  const totalBwTokens = stats?.total_tokens ?? 0
  const totalNativeTokens = calls.reduce((sum, c) => sum + nativeEstimate(c), 0)
  const savingsPct =
    totalNativeTokens > 0
      ? Math.round(((totalNativeTokens - totalBwTokens) / totalNativeTokens) * 100)
      : 0

  return (
    <div className={styles.wrapper}>
      {/* Stats bar */}
      <div className={styles.statsBar}>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>Calls</span>
          <span className={styles.statValue}>{stats?.total_calls ?? 0}</span>
        </div>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>Sessions</span>
          <span className={styles.statValue}>{stats?.session_count ?? 0}</span>
        </div>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>BW tokens</span>
          <span className={styles.statValue}>{fmtK(totalBwTokens)}</span>
        </div>
        <div className={styles.statItem}>
          <span className={styles.statLabel}>Avg ms</span>
          <span className={styles.statValue}>
            {stats ? Math.round(stats.avg_duration_ms) : 0}
          </span>
        </div>
        {savingsPct > 0 && (
          <div className={styles.statItem}>
            <span className={styles.statLabel}>Saved vs native</span>
            <span className={`${styles.statValue} ${styles.statSavings}`}>{savingsPct}%</span>
          </div>
        )}
      </div>

      {/* Body */}
      <div className={styles.body}>
        {/* Session sidebar */}
        <div className={styles.sidebar}>
          <div className={styles.sidebarTitle}>Sessions</div>
          {sessions.length === 0 ? (
            <div className={styles.emptyState}>
              No sessions yet.
              <br />
              Run bw-mcp to start recording.
            </div>
          ) : (
            sessions.map((s) => (
              <div
                key={s.session_id}
                className={`${styles.sessionItem}${activeSessionId === s.session_id ? ' ' + styles.sessionItemActive : ''}`}
                onClick={() =>
                  setActiveSession(
                    activeSessionId === s.session_id ? null : s.session_id,
                  )
                }
              >
                <div className={styles.sessionId}>{s.session_id}</div>
                <div className={styles.sessionMeta}>
                  <span className={styles.sessionCalls}>{s.call_count} calls</span>
                  <span className={styles.sessionTokens}>{fmtK(s.total_tokens)} tok</span>
                  <button
                    className={styles.sessionDelete}
                    onClick={(e) => handleDeleteSession(e, s.session_id)}
                    title="Delete session"
                  >
                    ✕
                  </button>
                </div>
                <div className={styles.sessionBar}>
                  <div
                    className={styles.sessionBarFill}
                    style={{
                      width: `${Math.min(100, (s.total_tokens / maxSessionTokens) * 100)}%`,
                    }}
                  />
                </div>
              </div>
            ))
          )}
        </div>

        {/* Call timeline */}
        <div className={styles.timeline}>
          <div className={styles.timelineHeader}>
            {activeSessionId
              ? `Calls — ${activeSessionId.slice(0, 8)}…`
              : 'Live stream — all sessions'}
          </div>
          <div className={styles.callList}>
            {loadingCalls ? (
              <div className={styles.emptyState}>Loading…</div>
            ) : calls.length === 0 ? (
              <div className={styles.emptyState}>
                {activeSessionId
                  ? 'No calls in this session.'
                  : 'Waiting for MCP tool calls…'}
              </div>
            ) : (
              calls.map((c) => (
                <CallRow key={c.id} record={c} active={c.id === selectedCallId} />
              ))
            )}
          </div>
        </div>

        {/* Detail panel */}
        <DetailPanel record={selectedCall} />
      </div>
    </div>
  )
}
