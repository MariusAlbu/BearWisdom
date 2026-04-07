import { useState, useEffect, useCallback } from 'react'
import { api } from '../../api'
import type { DeadCodeReport, DeadCodeEntry, EntryPointsReport, EntryPoint } from '../../types'
import styles from './DeadCode.module.css'

interface DeadCodeProps {
  workspacePath: string
  onFileNavigate: (filePath: string, line?: number) => void
}

type Tab = 'dead' | 'entry'
type Visibility = 'all' | 'private' | 'public'

function confidenceBar(c: number): string {
  if (c >= 0.9) return styles.confHigh
  if (c >= 0.6) return styles.confMed
  return styles.confLow
}

function kindIcon(kind: string): string {
  switch (kind) {
    case 'function': return 'function'
    case 'method': return 'settings_ethernet'
    case 'class': return 'class'
    case 'struct': return 'data_object'
    case 'interface': return 'integration_instructions'
    case 'enum': return 'format_list_numbered'
    case 'trait': return 'integration_instructions'
    case 'type_alias': return 'code'
    default: return 'code'
  }
}

function entryIcon(kind: string): string {
  switch (kind) {
    case 'main': return 'play_arrow'
    case 'route_handler': return 'route'
    case 'event_handler': return 'electric_bolt'
    case 'test_function': return 'science'
    case 'exported_api': return 'public'
    case 'lifecycle_hook': return 'cycle'
    case 'di_registered': return 'hub'
    default: return 'code'
  }
}

function entryLabel(kind: string): string {
  switch (kind) {
    case 'main': return 'Main'
    case 'route_handler': return 'Route'
    case 'event_handler': return 'Event'
    case 'test_function': return 'Test'
    case 'exported_api': return 'API'
    case 'lifecycle_hook': return 'Lifecycle'
    case 'di_registered': return 'DI'
    default: return kind
  }
}

export function DeadCode({ workspacePath, onFileNavigate }: DeadCodeProps) {
  const [tab, setTab] = useState<Tab>('dead')
  const [visibility, setVisibility] = useState<Visibility>('all')
  const [scope, setScope] = useState('')
  const [loading, setLoading] = useState(false)

  const [deadReport, setDeadReport] = useState<DeadCodeReport | null>(null)
  const [entryReport, setEntryReport] = useState<EntryPointsReport | null>(null)

  const fetchDeadCode = useCallback(async () => {
    setLoading(true)
    try {
      const report = await api.deadCode(workspacePath, {
        visibility,
        scope: scope || undefined,
        limit: 200,
      })
      setDeadReport(report)
    } catch (e) {
      console.error('Dead code fetch failed:', e)
    }
    setLoading(false)
  }, [workspacePath, visibility, scope])

  const fetchEntryPoints = useCallback(async () => {
    setLoading(true)
    try {
      const report = await api.entryPoints(workspacePath)
      setEntryReport(report)
    } catch (e) {
      console.error('Entry points fetch failed:', e)
    }
    setLoading(false)
  }, [workspacePath])

  useEffect(() => {
    if (tab === 'dead') fetchDeadCode()
    else fetchEntryPoints()
  }, [tab, fetchDeadCode, fetchEntryPoints])

  return (
    <div className={styles.container}>
      {/* Header */}
      <div className={styles.header}>
        <div className={styles.tabs}>
          <button
            className={`${styles.tab} ${tab === 'dead' ? styles.tabActive : ''}`}
            onClick={() => setTab('dead')}
          >
            <span className="material-symbols-outlined">delete_sweep</span>
            Dead Code
          </button>
          <button
            className={`${styles.tab} ${tab === 'entry' ? styles.tabActive : ''}`}
            onClick={() => setTab('entry')}
          >
            <span className="material-symbols-outlined">login</span>
            Entry Points
          </button>
        </div>

        {tab === 'dead' && (
          <div className={styles.filters}>
            <select
              className={styles.filterSelect}
              value={visibility}
              onChange={(e) => setVisibility(e.target.value as Visibility)}
            >
              <option value="all">All visibility</option>
              <option value="private">Private only</option>
              <option value="public">Public only</option>
            </select>
            <input
              className={styles.filterInput}
              type="text"
              placeholder="Scope (dir prefix)..."
              value={scope}
              onChange={(e) => setScope(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') fetchDeadCode() }}
            />
          </div>
        )}
      </div>

      {/* Resolution health banner */}
      {tab === 'dead' && deadReport && (
        <div className={`${styles.healthBanner} ${
          deadReport.resolution_health.resolution_rate >= 95 ? styles.healthExcellent
          : deadReport.resolution_health.resolution_rate >= 90 ? styles.healthGood
          : deadReport.resolution_health.resolution_rate >= 80 ? styles.healthFair
          : styles.healthLow
        }`}>
          <span className="material-symbols-outlined" style={{ fontSize: 18 }}>
            {deadReport.resolution_health.resolution_rate >= 90 ? 'verified' : 'warning'}
          </span>
          <span className={styles.healthRate}>
            {deadReport.resolution_health.resolution_rate.toFixed(1)}% resolved
          </span>
          <span className={styles.healthText}>{deadReport.resolution_health.assessment}</span>
          {deadReport.potentially_referenced_count > 0 && (
            <span className={styles.healthWarn}>
              {deadReport.potentially_referenced_count} flagged as potentially referenced
            </span>
          )}
        </div>
      )}

      {/* Summary */}
      {tab === 'dead' && deadReport && (
        <div className={styles.summary}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryValue}>
              {deadReport.dead_candidates.filter(c => !c.potentially_referenced).length}
            </span>
            <span className={styles.summaryLabel}>Confirmed dead</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryValue}>{deadReport.potentially_referenced_count}</span>
            <span className={styles.summaryLabel}>Needs review</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryValue}>{deadReport.entry_points_excluded}</span>
            <span className={styles.summaryLabel}>Entry points</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryValue}>{deadReport.test_symbols_excluded}</span>
            <span className={styles.summaryLabel}>Test excluded</span>
          </div>
          <div className={styles.summaryItem}>
            <span className={styles.summaryValue}>{deadReport.total_symbols_checked}</span>
            <span className={styles.summaryLabel}>Checked</span>
          </div>
        </div>
      )}

      {tab === 'entry' && entryReport && (
        <div className={styles.summary}>
          <div className={styles.summaryItem}>
            <span className={styles.summaryValue}>{entryReport.total}</span>
            <span className={styles.summaryLabel}>Total entry points</span>
          </div>
          {(() => {
            const counts: Record<string, number> = {}
            entryReport.entry_points.forEach((ep) => {
              counts[ep.entry_kind] = (counts[ep.entry_kind] || 0) + 1
            })
            return Object.entries(counts)
              .sort((a, b) => b[1] - a[1])
              .map(([kind, count]) => (
                <div key={kind} className={styles.summaryItem}>
                  <span className={styles.summaryValue}>{count}</span>
                  <span className={styles.summaryLabel}>{entryLabel(kind)}</span>
                </div>
              ))
          })()}
        </div>
      )}

      {/* Loading */}
      {loading && (
        <div className={styles.loadingBar}>
          <div className={styles.loadingBarInner} />
        </div>
      )}

      {/* Table */}
      <div className={styles.tableWrapper}>
        {tab === 'dead' && deadReport && (
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.thConf}>Conf</th>
                <th className={styles.thKind}>Kind</th>
                <th className={styles.thName}>Symbol</th>
                <th className={styles.thFile}>File</th>
                <th className={styles.thVis}>Vis</th>
              </tr>
            </thead>
            <tbody>
              {deadReport.dead_candidates.map((entry) => (
                <DeadRow key={entry.symbol_id} entry={entry} onClick={onFileNavigate} />
              ))}
              {deadReport.dead_candidates.length === 0 && (
                <tr><td colSpan={5} className={styles.empty}>No dead code found</td></tr>
              )}
            </tbody>
          </table>
        )}

        {tab === 'entry' && entryReport && (
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.thKind}>Type</th>
                <th className={styles.thKind}>Kind</th>
                <th className={styles.thName}>Symbol</th>
                <th className={styles.thFile}>File</th>
              </tr>
            </thead>
            <tbody>
              {entryReport.entry_points.map((ep) => (
                <EntryRow key={ep.symbol_id} entry={ep} onClick={onFileNavigate} />
              ))}
              {entryReport.entry_points.length === 0 && (
                <tr><td colSpan={4} className={styles.empty}>No entry points found</td></tr>
              )}
            </tbody>
          </table>
        )}
      </div>
    </div>
  )
}

function DeadRow({ entry, onClick }: { entry: DeadCodeEntry; onClick: (f: string, l?: number) => void }) {
  return (
    <tr
      className={`${styles.row} ${entry.potentially_referenced ? styles.rowWarning : ''}`}
      onClick={() => onClick(entry.file_path, entry.line)}
    >
      <td className={styles.tdConf}>
        <div className={`${styles.confDot} ${confidenceBar(entry.confidence)}`} />
        <span>{(entry.confidence * 100).toFixed(0)}%</span>
      </td>
      <td className={styles.tdKind}>
        <span className="material-symbols-outlined" style={{ fontSize: 16 }}>{kindIcon(entry.kind)}</span>
        {entry.kind}
      </td>
      <td className={styles.tdName}>
        <span className={styles.symbolName}>{entry.name}</span>
        <span className={styles.qualName}>{entry.qualified_name}</span>
        {entry.potentially_referenced && (
          <span className={styles.refWarning} title={`${entry.unresolved_ref_matches} unresolved refs match this name — may still be in use`}>
            <span className="material-symbols-outlined" style={{ fontSize: 14 }}>warning</span>
            {entry.unresolved_ref_matches} unresolved refs
          </span>
        )}
      </td>
      <td className={styles.tdFile}>
        <span className={styles.filePath}>{entry.file_path}</span>
        <span className={styles.fileLine}>:{entry.line}</span>
      </td>
      <td className={styles.tdVis}>
        {entry.visibility && <span className={styles.visBadge}>{entry.visibility}</span>}
      </td>
    </tr>
  )
}

function EntryRow({ entry, onClick }: { entry: EntryPoint; onClick: (f: string, l?: number) => void }) {
  return (
    <tr className={styles.row} onClick={() => onClick(entry.file_path, entry.line)}>
      <td className={styles.tdKind}>
        <span className="material-symbols-outlined" style={{ fontSize: 16 }}>{entryIcon(entry.entry_kind)}</span>
        {entryLabel(entry.entry_kind)}
      </td>
      <td className={styles.tdKind}>
        <span className="material-symbols-outlined" style={{ fontSize: 16 }}>{kindIcon(entry.kind)}</span>
        {entry.kind}
      </td>
      <td className={styles.tdName}>
        <span className={styles.symbolName}>{entry.name}</span>
        <span className={styles.qualName}>{entry.qualified_name}</span>
      </td>
      <td className={styles.tdFile}>
        <span className={styles.filePath}>{entry.file_path}</span>
        <span className={styles.fileLine}>:{entry.line}</span>
      </td>
    </tr>
  )
}
