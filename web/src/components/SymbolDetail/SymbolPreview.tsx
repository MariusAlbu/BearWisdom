import { shortPath } from '../../utils/graph.utils'
import type { D3Node } from '../../types/graph.types'
import type { SymbolDetail, CallHierarchyItem } from '../../types/api.types'
import { CallList } from './CallList'
import styles from './SymbolDetail.module.css'

interface SymbolPreviewProps {
  node: D3Node
  detail: SymbolDetail | null
  loading: boolean
  incomingCalls: CallHierarchyItem[]
  outgoingCalls: CallHierarchyItem[]
  selectedIncoming: number
  selectedOutgoing: number
  onNavigate: (qualifiedName: string) => void
}

export function SymbolPreview({
  node,
  detail,
  loading,
  incomingCalls,
  outgoingCalls,
  selectedIncoming,
  selectedOutgoing,
  onNavigate,
}: SymbolPreviewProps) {
  if (loading) {
    return (
      <div className={styles.detailBody}>
        <div className={styles.loadingText}>Loading...</div>
      </div>
    )
  }

  return (
    <div className={styles.detailBody}>
      <div className={styles.detailSection}>
        <div className={styles.detailSectionTitle}>Location</div>
        <div className={styles.detailFilePath} title={node.filePath}>
          {shortPath(node.filePath)}
          {detail && `:${detail.start_line}`}
        </div>
      </div>

      {detail?.signature && (
        <div className={styles.detailSection}>
          <div className={styles.detailSectionTitle}>Signature</div>
          <pre className={styles.detailSignature}>{detail.signature}</pre>
        </div>
      )}

      {detail?.doc_comment && (
        <div className={styles.detailSection}>
          <div className={styles.detailSectionTitle}>Documentation</div>
          <div className={styles.detailDoc}>{detail.doc_comment}</div>
        </div>
      )}

      <CallList title="Incoming" calls={incomingCalls} onNavigate={onNavigate} />
      <CallList title="Outgoing" calls={outgoingCalls} onNavigate={onNavigate} />

      {node.concept && (
        <div className={styles.detailSection}>
          <div className={styles.detailSectionTitle}>Concept</div>
          <span className={styles.conceptTag}>{node.concept}</span>
        </div>
      )}

      {node.annotation && (
        <div className={styles.detailSection}>
          <div className={styles.detailSectionTitle}>Annotation</div>
          <div className={styles.detailDoc}>{node.annotation}</div>
        </div>
      )}

      {!detail && (
        <div className={styles.detailSection}>
          <div className={styles.detailSectionTitle}>Edges</div>
          <div className={styles.statRow}>
            <span>Incoming</span>
            <span className={styles.statVal}>{selectedIncoming}</span>
          </div>
          <div className={styles.statRow}>
            <span>Outgoing</span>
            <span className={styles.statVal}>{selectedOutgoing}</span>
          </div>
        </div>
      )}
    </div>
  )
}
