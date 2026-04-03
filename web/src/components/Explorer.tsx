import { useState, useEffect, useCallback } from 'react'
import { api } from '../api'
import { Header } from './Header'
import { KnowledgeTree } from './KnowledgeTree'
import { FileViewer } from './FileViewer'
import { FlowExplorer } from './FlowExplorer'
import { Inspector } from './Inspector'
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

export function Explorer({ workspacePath, stats }: ExplorerProps) {
  const [mainView, setMainView] = useState<MainView>('graph')
  const [pendingSearch, setPendingSearch] = useState<string | null>(null)
  const [fileView, setFileView] = useState<FileViewState | null>(null)

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

  // Scroll to line when content loads
  useEffect(() => {
    if (fileView?.content && fileView.line) {
      setTimeout(() => {
        const lineEl = document.getElementById(`file-line-${fileView.line}`)
        lineEl?.scrollIntoView({ behavior: 'smooth', block: 'center' })
      }, 100)
    }
  }, [fileView?.content, fileView?.line])

  return (
    <div className={styles.explorer}>
      <Header
        workspacePath={workspacePath}
        stats={stats}
        onSearchResult={(qn) => {
          setFileView(null)
          setPendingSearch(qn)
        }}
        onFileNavigate={handleFileNavigate}
      />
      <div className={styles.viewTabs}>
        <button
          className={`${styles.viewTab}${mainView === 'graph' ? ' ' + styles.viewTabActive : ''}`}
          onClick={() => { setMainView('graph'); setFileView(null) }}
        >
          Graph
        </button>
        <button
          className={`${styles.viewTab}${mainView === 'flow' ? ' ' + styles.viewTabActive : ''}`}
          onClick={() => { setMainView('flow'); setFileView(null) }}
        >
          Flow
        </button>
        <button
          className={`${styles.viewTab}${mainView === 'inspector' ? ' ' + styles.viewTabActive : ''}`}
          onClick={() => { setMainView('inspector'); setFileView(null) }}
        >
          Inspector
        </button>
      </div>
      <div className={styles.main}>
        {fileView ? (
          <FileViewer
            filePath={fileView.filePath}
            line={fileView.line}
            content={fileView.content}
            loading={fileView.loading}
            onClose={() => setFileView(null)}
          />
        ) : mainView === 'inspector' ? (
          <Inspector workspacePath={workspacePath} />
        ) : mainView === 'flow' ? (
          <FlowExplorer
            workspacePath={workspacePath}
            onFileNavigate={handleFileNavigate}
          />
        ) : (
          <KnowledgeTree
            workspacePath={workspacePath}
            stats={stats}
            pendingNavigate={pendingSearch}
            onPendingNavigateConsumed={() => setPendingSearch(null)}
          />
        )}
      </div>
    </div>
  )
}
