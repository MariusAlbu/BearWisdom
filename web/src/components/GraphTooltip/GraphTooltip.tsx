import { shortPath } from '../../utils/graph.utils'
import type { TooltipState } from '../../types/graph.types'
import styles from './GraphTooltip.module.css'

interface GraphTooltipProps {
  tooltip: TooltipState
}

export function GraphTooltip({ tooltip }: GraphTooltipProps) {
  if (!tooltip.node) return null

  return (
    <div
      className={`${styles.tooltip}${tooltip.visible ? ' ' + styles.visible : ''}`}
      style={{ left: tooltip.x + 12, top: tooltip.y + 12 }}
    >
      <div className={styles.tooltipName}>{tooltip.node.name}</div>
      <div className={styles.tooltipKind} style={{ color: tooltip.node.color }}>
        {tooltip.node.kind}
      </div>
      <div className={styles.tooltipPath}>{shortPath(tooltip.node.filePath)}</div>
      {tooltip.node.concept && (
        <div className={styles.tooltipConcept}>concept: {tooltip.node.concept}</div>
      )}
    </div>
  )
}
