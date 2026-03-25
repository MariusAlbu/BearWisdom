import styles from './ConceptSidebar.module.css'

interface ConceptItemProps {
  name: string
  memberCount: number
  color: string
  isActive: boolean
  onClick: () => void
}

export function ConceptItem({ name, memberCount, color, isActive, onClick }: ConceptItemProps) {
  return (
    <button
      className={`${styles.conceptItem}${isActive ? ' ' + styles.active : ''}`}
      onClick={onClick}
    >
      <span className={styles.conceptDot} style={{ background: color }} />
      <span className={styles.conceptName}>{name}</span>
      <span className={styles.conceptCount}>{memberCount}</span>
    </button>
  )
}
