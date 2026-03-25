import { useSelectionStore } from '../../stores/selection.store'
import { useResizePanel } from '../../hooks/useResizePanel'
import { useSymbolDetail } from '../../hooks/useSymbolDetail'
import { SymbolPreview } from './SymbolPreview'
import { SymbolCodeView } from './SymbolCodeView'
import styles from './SymbolDetail.module.css'

interface SymbolDetailPanelProps {
  workspacePath: string
}

export function SymbolDetailPanel({ workspacePath }: SymbolDetailPanelProps) {
  const selectedNode = useSelectionStore((s) => s.selectedNode)
  const symbolDetail = useSelectionStore((s) => s.symbolDetail)
  const incomingCalls = useSelectionStore((s) => s.incomingCalls)
  const outgoingCalls = useSelectionStore((s) => s.outgoingCalls)
  const detailLoading = useSelectionStore((s) => s.detailLoading)
  const detailTab = useSelectionStore((s) => s.detailTab)
  const setDetailTab = useSelectionStore((s) => s.setDetailTab)
  const codeContent = useSelectionStore((s) => s.codeContent)
  const codeLoading = useSelectionStore((s) => s.codeLoading)
  const navHistory = useSelectionStore((s) => s.navHistory)
  const navigateBack = useSelectionStore((s) => s.navigateBack)
  const clearSelection = useSelectionStore((s) => s.clearSelection)
  const detailWidth = useSelectionStore((s) => s.detailWidth)

  const { handleResizeStart } = useResizePanel()
  const { navigateToSymbol } = useSymbolDetail(workspacePath)

  const detailOpen = selectedNode !== null

  // Edge counts from store edges (computed in the store-aware parent, but
  // since this panel is smart enough to read the store we compute inline)
  const selectedIncoming = incomingCalls.length
  const selectedOutgoing = outgoingCalls.length

  return (
    <div
      className={`${styles.detailPanel}${detailOpen ? ' ' + styles.open : ''}`}
      style={{ width: detailWidth }}
    >
      <div className={styles.detailResizeHandle} onMouseDown={handleResizeStart} />
      {selectedNode && (
        <>
          <div className={styles.detailHeader}>
            <div className={styles.detailTitleBlock}>
              <div className={styles.detailName}>{selectedNode.name}</div>
              <span
                className={styles.detailKindBadge}
                style={{ background: selectedNode.color }}
              >
                {selectedNode.kind}
              </span>
            </div>
            <button className={styles.detailClose} onClick={clearSelection}>
              &times;
            </button>
          </div>

          <div className={styles.tabBar}>
            <button
              className={`${styles.tab}${detailTab === 'preview' ? ' ' + styles.active : ''}`}
              onClick={() => setDetailTab('preview')}
            >
              Preview
            </button>
            <button
              className={`${styles.tab}${detailTab === 'code' ? ' ' + styles.active : ''}`}
              onClick={() => setDetailTab('code')}
            >
              Code
            </button>
          </div>

          {navHistory.length > 1 && (
            <div className={styles.breadcrumb}>
              {navHistory.map((entry, i) => {
                const isLast = i === navHistory.length - 1
                return isLast ? (
                  <span key={entry.node.id} className={styles.breadcrumbCurrent}>
                    {entry.label}
                  </span>
                ) : (
                  <span key={entry.node.id} style={{ display: 'contents' }}>
                    <button
                      className={styles.breadcrumbItem}
                      onClick={() => navigateBack(i)}
                    >
                      {entry.label}
                    </button>
                    <span className={styles.breadcrumbSep}>&rsaquo;</span>
                  </span>
                )
              })}
            </div>
          )}

          {detailTab === 'preview' && (
            <SymbolPreview
              node={selectedNode}
              detail={symbolDetail}
              loading={detailLoading}
              incomingCalls={incomingCalls}
              outgoingCalls={outgoingCalls}
              selectedIncoming={selectedIncoming}
              selectedOutgoing={selectedOutgoing}
              onNavigate={navigateToSymbol}
            />
          )}

          {detailTab === 'code' && (
            <SymbolCodeView
              content={codeContent}
              loading={codeLoading}
              startLine={symbolDetail?.start_line}
              endLine={symbolDetail?.end_line}
            />
          )}
        </>
      )}
    </div>
  )
}
