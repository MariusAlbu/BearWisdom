import { useState, useEffect, useCallback } from 'react'
import { api } from '../api'
import { Header } from './Header'
import { KnowledgeTree } from './KnowledgeTree'
import { FileViewer } from './FileViewer'
import styles from './Explorer.module.css'

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
      <div className={styles.main}>
        {fileView ? (
          <FileViewer
            filePath={fileView.filePath}
            line={fileView.line}
            content={fileView.content}
            loading={fileView.loading}
            onClose={() => setFileView(null)}
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
