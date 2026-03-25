import { useState, useRef, useEffect } from 'react'
import { useEmbedding } from '../../hooks/useEmbedding'
import type {
  SearchResult,
  FuzzyMatch,
  GrepMatch,
  ContentSearchResult,
  HybridSearchResult,
  SearchMode,
} from '../../types/api.types'
import { SearchPanel } from '../SearchPanel'
import { EmbedButton } from './EmbedButton'
import { ModeTabs } from './ModeTabs'
import { SearchInput } from './SearchInput'
import { useHeaderSearch } from './useHeaderSearch'
import styles from './Header.module.css'

interface HeaderProps {
  workspacePath: string
  stats: { files: number; symbols: number; edges: number }
  onSearchResult?: (qualifiedName: string) => void
  onFileNavigate?: (filePath: string, line?: number) => void
}

export type AnyResult =
  | { type: 'symbol'; data: SearchResult }
  | { type: 'fuzzy'; data: FuzzyMatch }
  | { type: 'grep'; data: GrepMatch }
  | { type: 'content'; data: ContentSearchResult }
  | { type: 'hybrid'; data: HybridSearchResult }

const MODES_BASE: { key: SearchMode; label: string; placeholder: string }[] = [
  { key: 'symbols', label: 'Symbols', placeholder: 'Search symbols (FTS)...' },
  { key: 'fuzzy-symbols', label: 'Fuzzy', placeholder: 'Fuzzy match symbol names...' },
  { key: 'fuzzy-files', label: 'Files', placeholder: 'Fuzzy match file paths...' },
  { key: 'content', label: 'Content', placeholder: 'Search file content (trigram, min 3 chars)...' },
  { key: 'grep', label: 'Grep', placeholder: 'Grep pattern across files...' },
]

const AI_MODE = {
  key: 'hybrid' as SearchMode,
  label: 'AI Search',
  placeholder: 'Natural language code search...',
}

export function Header({ workspacePath, stats, onSearchResult, onFileNavigate }: HeaderProps) {
  const [mode, setMode] = useState<SearchMode>('symbols')
  const inputRef = useRef<HTMLInputElement>(null)

  const { embedState, embedCount, embedError, triggerEmbed } = useEmbedding()
  const { query, results, panelOpen, searching, setPanelOpen, search, handleChange, handleKeyDown, clearSearch } =
    useHeaderSearch(workspacePath)

  const embedReady = embedState === 'done'
  const MODES = embedReady ? [...MODES_BASE, AI_MODE] : MODES_BASE
  const currentMode = MODES.find((m) => m.key === mode) ?? MODES[0]

  function handleModeChange(m: SearchMode) {
    setMode(m)
    if (query.trim()) search(query, m)
    inputRef.current?.focus()
  }

  function handleResultClick(result: AnyResult) {
    clearSearch()
    switch (result.type) {
      case 'symbol':
        onSearchResult?.(result.data.qualified_name)
        break
      case 'fuzzy':
        if (result.data.metadata.Symbol) onSearchResult?.(result.data.text)
        else if (result.data.metadata.File) onFileNavigate?.(result.data.text)
        break
      case 'content':
        onFileNavigate?.(result.data.file_path)
        break
      case 'grep':
        onFileNavigate?.(result.data.file_path, result.data.line_number)
        break
      case 'hybrid':
        onFileNavigate?.(result.data.file_path, result.data.start_line)
        break
    }
  }

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        setPanelOpen(false)
        inputRef.current?.blur()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [setPanelOpen])

  useEffect(() => {
    if (mode === 'hybrid' && !embedReady) setMode('symbols')
  }, [embedReady, mode])

  return (
    <header className={styles.header}>
      <div className={styles.logo}>
        <img src="/logo.png" alt="BearWisdom" />
        <span className={styles.logoText}>BearWisdom</span>
      </div>

      <div className={styles.divider} />

      <span className={styles.path} title={workspacePath}>
        {workspacePath}
      </span>

      <div className={styles.searchArea}>
        <ModeTabs modes={MODES} activeMode={mode} onModeChange={handleModeChange} />
        <SearchInput
          value={query}
          placeholder={currentMode.placeholder}
          searching={searching}
          inputRef={inputRef}
          onChange={(e) => handleChange(e, mode)}
          onKeyDown={(e) => handleKeyDown(e, mode)}
          onFocus={() => { if (results.length > 0) setPanelOpen(true) }}
        />
        {panelOpen && results.length > 0 && (
          <SearchPanel
            results={results}
            mode={mode}
            onSelect={handleResultClick}
            onClose={() => setPanelOpen(false)}
          />
        )}
      </div>

      <div className={styles.embedArea}>
        <EmbedButton
          state={embedState}
          count={embedCount}
          error={embedError}
          onEmbed={() => triggerEmbed(workspacePath)}
        />
      </div>

      <div className={styles.stats}>
        <span className={styles.stat}>
          <strong>{stats.files.toLocaleString()}</strong> files
        </span>
        <span className={styles.stat}>
          <strong>{stats.symbols.toLocaleString()}</strong> symbols
        </span>
        <span className={styles.stat}>
          <strong>{stats.edges.toLocaleString()}</strong> edges
        </span>
      </div>
    </header>
  )
}
