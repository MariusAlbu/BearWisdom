import { useCallback, useEffect } from 'react'
import { useGraphStore } from '../../stores/graph.store'
import { useGraphData } from '../../hooks/useGraphData'
import { useSymbolDetail } from '../../hooks/useSymbolDetail'
import { GraphCanvas } from '../GraphCanvas'
import { ConceptSidebar } from '../ConceptSidebar'
import { SymbolDetailPanel } from '../SymbolDetail'
import styles from './KnowledgeTree.module.css'

export interface KnowledgeTreeProps {
  workspacePath: string
  stats: { files: number; symbols: number; edges: number }
  onSearchResult?: (qualifiedName: string) => void
  pendingNavigate?: string | null
  onPendingNavigateConsumed?: () => void
}

export function KnowledgeTree({
  workspacePath,
  pendingNavigate,
  onPendingNavigateConsumed,
}: KnowledgeTreeProps) {
  const loadState = useGraphStore((s) => s.loadState)
  const errorMessage = useGraphStore((s) => s.errorMessage)
  const nodes = useGraphStore((s) => s.nodes)
  const sidebarOpen = useGraphStore((s) => s.sidebarOpen)
  const activeConcept = useGraphStore((s) => s.activeConcept)

  const { loadData, handleConceptClick } = useGraphData(workspacePath)
  const { navigateToSymbol } = useSymbolDetail(workspacePath)

  // "All" resets the filter — reuses handleConceptClick toggle by passing the active concept
  const handleAllClick = useCallback(() => {
    if (activeConcept) {
      void handleConceptClick(activeConcept)
    }
  }, [activeConcept, handleConceptClick])

  // Consume pendingNavigate from parent
  useEffect(() => {
    if (!pendingNavigate) return
    navigateToSymbol(pendingNavigate)
    onPendingNavigateConsumed?.()
  }, [pendingNavigate, navigateToSymbol, onPendingNavigateConsumed])

  if (!workspacePath) {
    return (
      <div className={styles.container}>
        <div className={styles.emptyState}>
          <p className={styles.emptyTitle}>No workspace selected</p>
        </div>
      </div>
    )
  }

  if (loadState === 'error') {
    return (
      <div className={styles.container}>
        <div className={styles.emptyState}>
          <p className={styles.emptyTitle}>Failed to load graph</p>
          {errorMessage && <p className={styles.emptySubtitle}>{errorMessage}</p>}
          <button className={styles.retryBtn} onClick={loadData}>
            Retry
          </button>
        </div>
      </div>
    )
  }

  if (loadState === 'ready' && nodes.length === 0) {
    return (
      <div className={styles.container}>
        <div className={styles.emptyState}>
          <p className={styles.emptyTitle}>No symbols found</p>
          <p className={styles.emptySubtitle}>
            The workspace was indexed but contains no symbols.
          </p>
          <button className={styles.retryBtn} onClick={loadData}>
            Retry
          </button>
        </div>
      </div>
    )
  }

  return (
    <div className={styles.container} style={{ paddingBottom: 26 }}>
      <ConceptSidebar onConceptClick={handleConceptClick} onAllClick={handleAllClick} />
      <GraphCanvas sidebarOpen={sidebarOpen} />
      <SymbolDetailPanel workspacePath={workspacePath} />

      <div className={styles.statusBar}>
        <div className={styles.statusLeft}>
          {loadState === 'loading' && <span>Loading graph...</span>}
        </div>
        <div>
          <button className={styles.statusBtn} onClick={loadData}>
            Refresh
          </button>
        </div>
      </div>
    </div>
  )
}
