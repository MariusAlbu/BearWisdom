import { kindColor } from '../../utils/graph.utils'
import type { CallHierarchyItem } from '../../types/api.types'
import styles from './SymbolDetail.module.css'

interface CallListProps {
  title: string
  calls: CallHierarchyItem[]
  onNavigate: (qualifiedName: string) => void
}

export function CallList({ title, calls, onNavigate }: CallListProps) {
  if (calls.length === 0) return null

  return (
    <div className={styles.detailSection}>
      <div className={styles.detailSectionTitle}>
        {title} ({calls.length})
      </div>
      {calls.map((call) => (
        <button
          key={`${call.qualified_name}::${call.line}`}
          className={styles.refItem}
          onClick={() => onNavigate(call.qualified_name)}
          title={call.file_path}
        >
          <span className={styles.refDot} style={{ background: kindColor(call.kind) }} />
          {call.name}
        </button>
      ))}
    </div>
  )
}
