import type { SearchMode } from '../types'
import type { AnyResult } from './Header/Header'
import styles from './SearchPanel.module.css'

const KIND_COLOR: Record<string, string> = {
  class: '#58a6ff', interface: '#bc8cff', method: '#3fb950', function: '#3fb950',
  enum: '#d29922', struct: '#58a6ff', type: '#bc8cff', module: '#39c5cf',
  constant: '#e3b341', field: '#8b949e', variable: '#8b949e',
}

function kindColor(kind: string): string {
  return KIND_COLOR[kind.toLowerCase()] ?? '#6e7681'
}

interface SearchPanelProps {
  results: AnyResult[]
  mode: SearchMode
  onSelect: (result: AnyResult) => void
  onClose: () => void
}

export function SearchPanel({ results, onSelect }: SearchPanelProps) {
  return (
    <div className={styles.panel} role="listbox">
      <div className={styles.header}>
        {results.length} result{results.length !== 1 ? 's' : ''}
      </div>
      <div className={styles.list}>
        {results.map((r, i) => (
          <button
            key={i}
            className={styles.item}
            role="option"
            aria-selected={false}
            onClick={() => onSelect(r)}
          >
            {r.type === 'symbol' && (
              <>
                <span className={styles.kindBadge} style={{ background: kindColor(r.data.kind) }}>
                  {r.data.kind}
                </span>
                <span className={styles.info}>
                  <span className={styles.name}>{r.data.name}</span>
                  <span className={styles.secondary}>{r.data.file_path}:{r.data.start_line}</span>
                </span>
                <span className={styles.score}>{r.data.score.toFixed(1)}</span>
              </>
            )}

            {r.type === 'fuzzy' && r.data.metadata.Symbol && (
              <>
                <span className={styles.kindBadge} style={{ background: kindColor(r.data.metadata.Symbol.kind) }}>
                  {r.data.metadata.Symbol.kind}
                </span>
                <span className={styles.info}>
                  <span className={styles.name}>{r.data.text}</span>
                  <span className={styles.secondary}>{r.data.metadata.Symbol.file_path}:{r.data.metadata.Symbol.line}</span>
                </span>
                <span className={styles.score}>{r.data.score}</span>
              </>
            )}

            {r.type === 'fuzzy' && r.data.metadata.File && (
              <>
                <span className={styles.kindBadge} style={{ background: '#39c5cf' }}>
                  {r.data.metadata.File.language}
                </span>
                <span className={styles.info}>
                  <span className={styles.name}>{r.data.text}</span>
                  <span className={styles.secondary}>Click to view file</span>
                </span>
                <span className={styles.score}>{r.data.score}</span>
              </>
            )}

            {r.type === 'content' && (
              <>
                <span className={styles.kindBadge} style={{ background: '#39c5cf' }}>
                  {r.data.language}
                </span>
                <span className={styles.info}>
                  <span className={styles.name}>{r.data.file_path}</span>
                  <span className={styles.secondary}>Click to view file</span>
                </span>
                <span className={styles.score}>{r.data.score.toFixed(1)}</span>
              </>
            )}

            {r.type === 'grep' && (
              <>
                <span className={styles.kindBadge} style={{ background: '#C8915C' }}>
                  L{r.data.line_number}
                </span>
                <span className={styles.info}>
                  <span className={styles.name}>{r.data.file_path}</span>
                  <span className={styles.grepLine}>{r.data.line_content.trim()}</span>
                </span>
              </>
            )}

            {r.type === 'hybrid' && (
              <>
                <span className={styles.kindBadge} style={{ background: '#4A7C59' }}>
                  AI
                </span>
                <span className={styles.info}>
                  <span className={styles.name}>
                    {r.data.symbol_name ? `${r.data.symbol_name} — ` : ''}{r.data.file_path}:{r.data.start_line}
                  </span>
                  <span className={styles.grepLine}>{r.data.content_preview.slice(0, 120)}</span>
                </span>
                <span className={styles.score}>{r.data.rrf_score.toFixed(2)}</span>
              </>
            )}
          </button>
        ))}
      </div>
    </div>
  )
}
