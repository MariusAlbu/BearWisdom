import type { SearchMode } from '../../types/api.types'
import styles from './Header.module.css'

interface ModeTabDef {
  key: SearchMode
  label: string
}

interface ModeTabsProps {
  modes: ModeTabDef[]
  activeMode: SearchMode
  onModeChange: (mode: SearchMode) => void
}

export function ModeTabs({ modes, activeMode, onModeChange }: ModeTabsProps) {
  return (
    <div className={styles.modeTabs}>
      {modes.map((m) => (
        <button
          key={m.key}
          className={`${styles.modeTab}${activeMode === m.key ? ' ' + styles.modeTabActive : ''}`}
          onClick={() => onModeChange(m.key)}
        >
          {m.label}
        </button>
      ))}
    </div>
  )
}
