import { useRef, useState } from 'react'
import { useGraphStore } from '../../stores/graph.store'
import { useD3Simulation } from '../../hooks/useD3Simulation'
import { useZoomControls } from '../../hooks/useZoomControls'
import { GraphTooltip } from '../GraphTooltip'
import { GraphLegend } from '../GraphLegend'
import { ZoomControls } from '../ZoomControls'
import type { TooltipState } from '../../types/graph.types'
import styles from './GraphCanvas.module.css'

interface GraphCanvasProps {
  sidebarOpen: boolean
}

const INITIAL_TOOLTIP: TooltipState = { visible: false, x: 0, y: 0, node: null }

export function GraphCanvas({ sidebarOpen }: GraphCanvasProps) {
  const loadState = useGraphStore((s) => s.loadState)
  const svgRef = useRef<SVGSVGElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [tooltip, setTooltip] = useState<TooltipState>(INITIAL_TOOLTIP)

  const { zoomRef } = useD3Simulation(svgRef, containerRef, setTooltip)
  const { handleZoomIn, handleZoomOut, handleZoomReset } = useZoomControls(svgRef, zoomRef)

  return (
    <div
      ref={containerRef}
      className={styles.graphArea}
      style={{
        marginLeft: sidebarOpen ? 240 : 0,
        transition: 'margin-left 0.3s var(--ease-out)',
      }}
    >
      <svg ref={svgRef} role="img" aria-label="Knowledge graph" />

      <GraphTooltip tooltip={tooltip} />
      <GraphLegend />
      <ZoomControls
        onZoomIn={handleZoomIn}
        onZoomOut={handleZoomOut}
        onZoomReset={handleZoomReset}
      />

      {loadState === 'loading' && (
        <div
          className={styles.loadingOverlay}
        >
          <p className={styles.loadingTitle}>Loading graph...</p>
        </div>
      )}
    </div>
  )
}
