import { useState, useEffect, useCallback, useRef } from 'react'
import { api } from '../api'
import { FileViewer } from './FileViewer'
import { FlowExplorer } from './FlowExplorer'
import { Inspector } from './Inspector'
import { HierarchyGraph } from './HierarchyGraph'
import { useEmbedding } from '../hooks/useEmbedding'
import { useHeaderSearch } from './Header/useHeaderSearch'
import { SearchPanel } from './SearchPanel'
import { useHierarchyStore } from '../stores/hierarchy.store'
import type { AnyResult } from './Header'
import type { SearchMode } from '../types/api.types'
import styles from './Explorer.module.css'

type MainView = 'graph' | 'flow' | 'inspector'

interface ExplorerProps {
  workspacePath: string
  stats: { files: number; symbols: number; edges: number }
}

interface FileViewState {
  filePath: string
  line?: number
  content: string | null
  loading: boolean
}

const MODES_BASE: { key: SearchMode; label: string; placeholder: string }[] = [
  { key: 'symbols', label: 'Symbols', placeholder: 'Search symbols...' },
  { key: 'fuzzy-symbols', label: 'Fuzzy', placeholder: 'Fuzzy symbol names...' },
  { key: 'fuzzy-files', label: 'Files', placeholder: 'Fuzzy file paths...' },
  { key: 'content', label: 'Content', placeholder: 'Search file content...' },
  { key: 'grep', label: 'Grep', placeholder: 'Grep pattern...' },
]

const AI_MODE = {
  key: 'hybrid' as SearchMode,
  label: 'AI',
  placeholder: 'Natural language search...',
}

// Workspace name from path (last segment)
function workspaceName(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || path
}

export function Explorer({ workspacePath, stats }: ExplorerProps) {
  const [mainView, setMainView] = useState<MainView>('graph')
  const [fileView, setFileView] = useState<FileViewState | null>(null)
  const [mode, setMode] = useState<SearchMode>('symbols')
  const inputRef = useRef<HTMLInputElement>(null)

  const { embedState, embedCount, embedError, triggerEmbed } = useEmbedding()
  const { query, results, panelOpen, searching, setPanelOpen, search, handleChange, handleKeyDown, clearSearch } =
    useHeaderSearch(workspacePath)

  // Hierarchy store — for search-filter mapping
  const hierarchyNodes = useHierarchyStore((s) => s.nodes)
  const setSearchFilter = useHierarchyStore((s) => s.setSearchFilter)

  const embedReady = embedState === 'done'
  const MODES = embedReady ? [...MODES_BASE, AI_MODE] : MODES_BASE

  function handleModeChange(m: SearchMode) {
    setMode(m)
    if (query.trim()) search(query, m)
    inputRef.current?.focus()
  }

  function handleResultClick(result: AnyResult) {
    // On Flow/Inspector tabs the search query drives live filtering — clicking
    // a result just closes the panel and lets the active filter do the work.
    if (mainView !== 'graph') {
      setPanelOpen(false)
      return
    }

    clearSearch()
    setPanelOpen(false)
    switch (result.type) {
      case 'symbol': {
        // Find the hierarchy node matching this symbol by name or file_path
        const matched = hierarchyNodes.find(
          (n) =>
            n.name === result.data.name ||
            n.id === result.data.qualified_name ||
            n.file_path === result.data.file_path,
        )
        if (matched) {
          setSearchFilter([matched.id])
        }
        break
      }
      case 'fuzzy': {
        // fuzzy-symbol: match by name; fuzzy-file: match by file_path
        const isSymbol = Boolean(result.data.metadata.Symbol)
        const matched = hierarchyNodes.find((n) =>
          isSymbol ? n.name === result.data.text : n.file_path === result.data.text,
        )
        if (matched) {
          setSearchFilter([matched.id])
        } else if (!isSymbol) {
          handleFileNavigate(result.data.text)
        }
        break
      }
      case 'content':
        handleFileNavigate(result.data.file_path)
        break
      case 'grep':
        handleFileNavigate(result.data.file_path, result.data.line_number)
        break
      case 'hybrid':
        handleFileNavigate(result.data.file_path, result.data.start_line)
        break
    }
  }

  const handleFileNavigate = useCallback(
    async (filePath: string, line?: number) => {
      setFileView({ filePath, line, content: null, loading: true })
      try {
        const result = await api.fileContent(workspacePath, filePath)
        setFileView({ filePath, line, content: result.content, loading: false })
      } catch {
        setFileView({ filePath, line, content: '// Failed to load file', loading: false })
      }
    },
    [workspacePath],
  )

  // Map search results to hierarchy node IDs and push into the store filter.
  // Only active when on the graph tab — other tabs handle filtering locally.
  // When the panel closes, results are cleared, or we leave the graph tab, remove the filter.
  useEffect(() => {
    if (mainView !== 'graph' || !panelOpen || results.length === 0) {
      setSearchFilter(null)
      return
    }
    const ids = new Set<string>()
    for (const result of results) {
      switch (result.type) {
        case 'symbol': {
          const matched = hierarchyNodes.find(
            (n) =>
              n.name === result.data.name ||
              n.id === result.data.qualified_name ||
              n.file_path === result.data.file_path,
          )
          if (matched) ids.add(matched.id)
          break
        }
        case 'fuzzy': {
          const isSymbol = Boolean(result.data.metadata.Symbol)
          const matched = hierarchyNodes.find((n) =>
            isSymbol ? n.name === result.data.text : n.file_path === result.data.text,
          )
          if (matched) ids.add(matched.id)
          break
        }
        case 'content':
        case 'hybrid': {
          const fp = result.type === 'content' ? result.data.file_path : result.data.file_path
          const matched = hierarchyNodes.find((n) => n.file_path === fp || n.id === fp)
          if (matched) ids.add(matched.id)
          break
        }
        case 'grep': {
          const matched = hierarchyNodes.find(
            (n) => n.file_path === result.data.file_path || n.id === result.data.file_path,
          )
          if (matched) ids.add(matched.id)
          break
        }
      }
    }
    setSearchFilter(ids.size > 0 ? Array.from(ids) : null)
  }, [mainView, panelOpen, results, hierarchyNodes, setSearchFilter])

  // Scroll to line when content loads
  useEffect(() => {
    if (fileView?.content && fileView.line) {
      setTimeout(() => {
        const lineEl = document.getElementById(`file-line-${fileView.line}`)
        lineEl?.scrollIntoView({ behavior: 'smooth', block: 'center' })
      }, 100)
    }
  }, [fileView?.content, fileView?.line])

  useEffect(() => {
    if (mode === 'hybrid' && !embedReady) setMode('symbols')
  }, [embedReady, mode])

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'p') {
        e.preventDefault()
        inputRef.current?.focus()
      }
      if (e.key === 'Escape') {
        setPanelOpen(false)
        inputRef.current?.blur()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [setPanelOpen])

  const viewLabel = fileView
    ? 'File'
    : mainView === 'graph'
      ? 'Architecture Graph'
      : mainView === 'flow'
        ? 'Flow Explorer'
        : 'Inspector'

  return (
    <div className={styles.explorer}>
      {/* ---- Left Sidebar ---- */}
      <aside className={styles.sidebar}>
        {/* Logo + workspace */}
        <div className={styles.sidebarHeader}>
          <div className={styles.logo}>
            <div className={styles.logoPaw}>
              <img src="/logo.png" alt="" className={styles.logoImg} />
            </div>
            <span className={styles.logoText}>BearWisdom</span>
          </div>

          <div className={styles.workspaceLabel}>
            <span className="material-symbols-outlined">folder</span>
            Workspace
          </div>
          <div className={styles.workspaceSelector}>
            <div className={styles.workspaceDot} />
            <span className={styles.workspaceName} title={workspacePath}>
              {workspaceName(workspacePath)}
            </span>
            <span className={styles.workspaceCaret}>▾</span>
          </div>

          {/* Search */}
          <div className={styles.searchWrapper}>
            <input
              ref={inputRef}
              className={styles.searchInput}
              type="text"
              placeholder="CMD+P Search..."
              value={query}
              onChange={(e) => handleChange(e, mode)}
              onKeyDown={(e) => handleKeyDown(e, mode)}
              onFocus={() => { if (results.length > 0) setPanelOpen(true) }}
              aria-label="Search"
            />
            {searching && <div className={styles.searchSpinner} />}
            {panelOpen && results.length > 0 && (
              <SearchPanel
                results={results}
                mode={mode}
                onSelect={handleResultClick}
                onClose={() => setPanelOpen(false)}
              />
            )}
          </div>

          {/* Mode tabs — compact row */}
          <div className={styles.modeTabs}>
            {MODES.map((m) => (
              <button
                key={m.key}
                className={`${styles.modeTab}${mode === m.key ? ' ' + styles.modeTabActive : ''}`}
                onClick={() => handleModeChange(m.key)}
              >
                {m.label}
              </button>
            ))}
          </div>
        </div>

        {/* Nav */}
        <nav className={styles.sidebarNav}>
          <div className={styles.navSection}>Viewports</div>
          <button
            className={`${styles.navBtn}${mainView === 'graph' && !fileView ? ' ' + styles.navBtnActive : ''}`}
            onClick={() => { setMainView('graph'); setFileView(null) }}
          >
            <span className="material-symbols-outlined">account_tree</span>
            Graph
          </button>
          <button
            className={`${styles.navBtn}${mainView === 'flow' && !fileView ? ' ' + styles.navBtnActive : ''}`}
            onClick={() => { setMainView('flow'); setFileView(null) }}
          >
            <span className="material-symbols-outlined">reorder</span>
            Flow
          </button>
          <button
            className={`${styles.navBtn}${mainView === 'inspector' && !fileView ? ' ' + styles.navBtnActive : ''}`}
            onClick={() => { setMainView('inspector'); setFileView(null) }}
          >
            <span className="material-symbols-outlined">info</span>
            Inspector
          </button>

          {/* Analytics */}
          <div className={styles.navSection} style={{ marginTop: '24px' }}>Analytics</div>
          <div className={styles.statsList}>
            <div className={styles.statEntry}>
              <span className={styles.statEntryLabel}>Files</span>
              <span className={styles.statEntryValue}>{stats.files.toLocaleString()}</span>
            </div>
            <div className={styles.statEntry}>
              <span className={styles.statEntryLabel}>Symbols</span>
              <span className={styles.statEntryValue}>{stats.symbols.toLocaleString()}</span>
            </div>
            <div className={styles.statEntry}>
              <span className={styles.statEntryLabel}>Edges</span>
              <span className={styles.statEntryValue}>{stats.edges.toLocaleString()}</span>
            </div>
          </div>
        </nav>

        {/* Status footer */}
        <div className={styles.sidebarFooter}>
          <div className={styles.footerStatusRow}>
            <span>STATUS</span>
            <span className={styles.footerStatusValue}>INDEXED</span>
          </div>
          <div className={styles.footerEngine}>Rust · SQLite WAL</div>
          <div className={styles.footerEmbedRow}>
            {embedState === 'idle' && (
              <button
                className={styles.embedBtn}
                onClick={() => triggerEmbed(workspacePath)}
              >
                Embed vectors
              </button>
            )}
            {embedState === 'running' && (
              <span className={styles.embedRunning}>
                <div className={styles.embedSpinner} />
                Embedding... {embedCount > 0 && `${embedCount}`}
              </span>
            )}
            {embedState === 'done' && (
              <span className={styles.embedDone}>Vectors ready · AI search on</span>
            )}
            {embedState === 'error' && (
              <button
                className={styles.embedError}
                onClick={() => triggerEmbed(workspacePath)}
                title={embedError ?? undefined}
              >
                Embed failed — retry
              </button>
            )}
          </div>
        </div>
      </aside>

      {/* ---- Main Content ---- */}
      <main className={styles.main}>
        {/* Top bar */}
        <div className={styles.topBar}>
          <div className={styles.topBarLeft}>
            <h2 className={styles.topBarTitle}>{viewLabel}</h2>
            <div className={styles.topBarDivider} />
            <div className={styles.topBarMeta}>
              <span>Nodes: <b className={styles.metaBlue}>{stats.symbols.toLocaleString()}</b></span>
              <span>Edges: <b className={styles.metaAmber}>{stats.edges.toLocaleString()}</b></span>
            </div>
          </div>
          <div className={styles.topBarBreadcrumb}>
            Projects / {workspaceName(workspacePath)} / {viewLabel}
          </div>
        </div>

        {/* Content area */}
        <div className={styles.content}>
          {fileView ? (
            <FileViewer
              filePath={fileView.filePath}
              line={fileView.line}
              content={fileView.content}
              loading={fileView.loading}
              onClose={() => setFileView(null)}
            />
          ) : mainView === 'inspector' ? (
            <Inspector workspacePath={workspacePath} searchQuery={query} />
          ) : mainView === 'flow' ? (
            <FlowExplorer
              workspacePath={workspacePath}
              onFileNavigate={handleFileNavigate}
              searchQuery={query}
            />
          ) : (
            <HierarchyGraph workspacePath={workspacePath} />
          )}
        </div>
      </main>
    </div>
  )
}
