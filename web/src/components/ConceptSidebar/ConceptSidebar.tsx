import { useState } from 'react'
import { useGraphStore } from '../../stores/graph.store'
import { CONCEPT_COLORS } from '../../utils/graph.utils'
import { ConceptItem } from './ConceptItem'
import styles from './ConceptSidebar.module.css'

interface ConceptSidebarProps {
  onConceptClick: (name: string) => void
  onAllClick: () => void
}

export function ConceptSidebar({ onConceptClick, onAllClick }: ConceptSidebarProps) {
  const concepts = useGraphStore((s) => s.concepts)
  const activeConcept = useGraphStore((s) => s.activeConcept)
  const sidebarOpen = useGraphStore((s) => s.sidebarOpen)
  const toggleSidebar = useGraphStore((s) => s.toggleSidebar)
  const overview = useGraphStore((s) => s.overview)

  const [conceptSearch, setConceptSearch] = useState('')

  const filteredConcepts = concepts.filter((c) =>
    c.name.toLowerCase().includes(conceptSearch.toLowerCase()),
  )

  const sidebarLeft = sidebarOpen ? 240 : 0

  return (
    <>
      <button
        className={styles.sidebarToggle}
        style={{ left: sidebarLeft }}
        onClick={toggleSidebar}
        title={sidebarOpen ? 'Close sidebar' : 'Open sidebar'}
      >
        {sidebarOpen ? '\u2039' : '\u203a'}
      </button>

      <div className={`${styles.sidebar}${sidebarOpen ? '' : ' ' + styles.collapsed}`}>
        <div className={styles.sidebarHeader}>
          <div className={styles.sidebarTitle}>Concepts</div>
          <input
            className={styles.sidebarSearch}
            type="text"
            placeholder="Filter concepts..."
            value={conceptSearch}
            onChange={(e) => setConceptSearch(e.target.value)}
            aria-label="Filter concepts"
          />
        </div>

        <div className={styles.conceptList} role="listbox" aria-label="Concepts">
          <button
            className={`${styles.conceptItem}${activeConcept === null ? ' ' + styles.active : ''}`}
            onClick={onAllClick}
          >
            <span className={styles.conceptDot} style={{ background: '#6e7681' }} />
            <span className={styles.conceptName}>All</span>
            <span className={styles.conceptCount}>{overview.totalNodes}</span>
          </button>

          {filteredConcepts.map((concept, i) => {
            const color = CONCEPT_COLORS[i % CONCEPT_COLORS.length]
            return (
              <ConceptItem
                key={concept.name}
                name={concept.name}
                memberCount={concept.member_count}
                color={color}
                isActive={activeConcept === concept.name}
                onClick={() => onConceptClick(concept.name)}
              />
            )
          })}
        </div>

        <div className={styles.sidebarFooter}>
          <div className={styles.sidebarStat}>
            <span>Concepts</span>
            <span className={styles.sidebarStatVal}>{concepts.length}</span>
          </div>
        </div>
      </div>
    </>
  )
}
