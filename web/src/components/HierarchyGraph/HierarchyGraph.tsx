import * as d3 from 'd3'
import { useEffect, useRef, useCallback } from 'react'
import { useHierarchyData } from '../../hooks/useHierarchyData'
import { ZoomControls } from '../ZoomControls'
import type { HierarchyNode } from '../../types/api.types'
import styles from './HierarchyGraph.module.css'

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const HIERARCHY_COLORS: Record<string, string> = {
  service: '#f85149',
  package: '#58a6ff',
  file: '#3fb950',
  class: '#58a6ff',
  interface: '#bc8cff',
  method: '#3fb950',
  function: '#3fb950',
  enum: '#d29922',
  struct: '#58a6ff',
  module: '#39c5cf',
  constant: '#e3b341',
  field: '#8b949e',
}
const DEFAULT_COLOR = '#6e7681'

const EDGE_DASH: Record<string, string | null> = {
  service_dependency: null,
  cross_package: '6 3',
  file_dependency: '4 2',
  calls: '3 3',
}

// Per-level force simulation parameters
const LEVEL_FORCES: Record<string, { charge: number; linkDistance: number; collisionPad: number }> = {
  services: { charge: -600, linkDistance: 220, collisionPad: 24 },
  packages: { charge: -400, linkDistance: 160, collisionPad: 18 },
  files: { charge: -280, linkDistance: 110, collisionPad: 12 },
  symbols: { charge: -200, linkDistance: 80, collisionPad: 8 },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function nodeColor(kind: string): string {
  return HIERARCHY_COLORS[kind.toLowerCase()] ?? DEFAULT_COLOR
}

function nodeRadius(node: HierarchyNode): number {
  return Math.max(20, Math.min(60, 15 + Math.sqrt(node.weight) * 3))
}

function edgeThickness(weight: number): number {
  return 1 + Math.log2(Math.max(1, weight))
}

function levelLabel(level: string): string {
  const map: Record<string, string> = {
    services: 'Services',
    packages: 'Packages',
    files: 'Files',
    symbols: 'Symbols',
  }
  return map[level] ?? level
}

// D3 simulation node — extends HierarchyNode with simulation coords
interface SimNode extends d3.SimulationNodeDatum, HierarchyNode {
  radius: number
  color: string
}

// D3 simulation link
interface SimEdge extends d3.SimulationLinkDatum<SimNode> {
  kind: string
  weight: number
  confidence: number
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface HierarchyGraphProps {
  workspacePath: string
}

export function HierarchyGraph({ workspacePath }: HierarchyGraphProps) {
  const svgRef = useRef<SVGSVGElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const simulationRef = useRef<d3.Simulation<SimNode, SimEdge> | null>(null)
  const zoomRef = useRef<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null)
  const genRef = useRef(0)

  const {
    nodes,
    edges,
    level,
    breadcrumbs,
    loadState,
    errorMessage,
    selectedNodeId,
    loadLevel,
    drillDown,
    navigateTo,
    selectNode,
  } = useHierarchyData(workspacePath)

  // -------------------------------------------------------------------------
  // D3 render
  // -------------------------------------------------------------------------
  useEffect(() => {
    if (loadState !== 'ready') return
    if (!svgRef.current || !containerRef.current) return

    const gen = ++genRef.current
    const thisSvg = svgRef.current
    const container = containerRef.current

    const width = container.clientWidth || 900
    const height = container.clientHeight || 600

    d3.select(thisSvg).selectAll('*').remove()
    simulationRef.current?.stop()

    const svgSel = d3
      .select<SVGSVGElement, unknown>(thisSvg)
      .attr('width', width)
      .attr('height', height)

    // Arrow marker
    const defs = svgSel.append('defs')
    defs
      .append('marker')
      .attr('id', `hg-arrow-${gen}`)
      .attr('viewBox', '0 -4 8 8')
      .attr('refX', 12)
      .attr('refY', 0)
      .attr('markerWidth', 6)
      .attr('markerHeight', 6)
      .attr('orient', 'auto')
      .append('path')
      .attr('d', 'M 0 -4 L 8 0 L 0 4')
      .attr('fill', '#484f58')

    // Build sim nodes/edges
    const simNodes: SimNode[] = nodes.map((n) => ({
      ...n,
      radius: nodeRadius(n),
      color: nodeColor(n.kind),
    }))
    const nodeById = new Map<string, SimNode>(simNodes.map((n) => [n.id, n]))

    const simEdges: SimEdge[] = edges
      .filter((e) => nodeById.has(e.source) && nodeById.has(e.target))
      .map((e) => ({
        source: nodeById.get(e.source)!,
        target: nodeById.get(e.target)!,
        kind: e.kind,
        weight: e.weight,
        confidence: e.confidence,
      }))

    // Zoom + pan
    const zoomGroup = svgSel.append('g').attr('class', 'hg-zoom-group')
    const edgesGroup = zoomGroup.append('g').attr('class', 'hg-edges')
    const edgeLabelsGroup = zoomGroup.append('g').attr('class', 'hg-edge-labels')
    const nodesGroup = zoomGroup.append('g').attr('class', 'hg-nodes')

    const zoom = d3
      .zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.05, 10])
      .on('zoom', (event: d3.D3ZoomEvent<SVGSVGElement, unknown>) => {
        zoomGroup.attr('transform', String(event.transform))
      })
    zoomRef.current = zoom
    svgSel.call(zoom)

    // ---- Edges ----
    const edgeSel = edgesGroup
      .selectAll<SVGLineElement, SimEdge>('line')
      .data(simEdges)
      .join('line')
      .attr('class', 'hg-edge')
      .attr('stroke', '#9da5ae')
      .attr('stroke-opacity', 0.5)
      .attr('stroke-width', (d) => edgeThickness(d.weight))
      .attr('stroke-dasharray', (d) => EDGE_DASH[d.kind] ?? null)
      .attr('marker-end', (d) =>
        d.kind === 'calls' || d.kind === 'service_dependency'
          ? `url(#hg-arrow-${gen})`
          : null,
      )

    // Edge weight labels (only for heavier edges to avoid clutter)
    const edgeLabelSel = edgeLabelsGroup
      .selectAll<SVGTextElement, SimEdge>('text')
      .data(simEdges.filter((e) => e.weight > 1))
      .join('text')
      .attr('class', 'hg-edge-label')
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'middle')
      .attr('font-size', '9px')
      .attr('fill', '#8b949e')
      .attr('pointer-events', 'none')
      .text((d) => String(d.weight))

    // ---- Nodes ----
    const nodeSel = nodesGroup
      .selectAll<SVGGElement, SimNode>('g.hg-node')
      .data(simNodes, (d) => d.id)
      .join('g')
      .attr('class', 'hg-node')

    // Render shape by kind — rectangles for service/package/file, circles/diamonds/hexagons for symbols
    nodeSel.each(function (d: SimNode) {
      const g = d3.select(this)
      const k = d.kind.toLowerCase()
      const r = d.radius
      const col = d.color

      if (k === 'service') {
        // Large rounded rectangle
        const w = r * 2.8
        const h = r * 1.6
        g.append('rect')
          .attr('x', -w / 2)
          .attr('y', -h / 2)
          .attr('width', w)
          .attr('height', h)
          .attr('rx', 8)
          .attr('fill', col)
          .attr('fill-opacity', 0.2)
          .attr('stroke', col)
          .attr('stroke-width', 2)
      } else if (k === 'package') {
        // Medium rounded rectangle with tab notch
        const w = r * 2.4
        const h = r * 1.4
        g.append('rect')
          .attr('x', -w / 2)
          .attr('y', -h / 2)
          .attr('width', w)
          .attr('height', h)
          .attr('rx', 5)
          .attr('fill', col)
          .attr('fill-opacity', 0.18)
          .attr('stroke', col)
          .attr('stroke-width', 1.5)
      } else if (k === 'file') {
        // Small rect with slightly angled top-right corner (file icon feel)
        const w = r * 2.0
        const h = r * 1.3
        const cut = r * 0.28
        g.append('path')
          .attr(
            'd',
            `M ${-w / 2} ${-h / 2}
             L ${w / 2 - cut} ${-h / 2}
             L ${w / 2} ${-h / 2 + cut}
             L ${w / 2} ${h / 2}
             L ${-w / 2} ${h / 2} Z`,
          )
          .attr('fill', col)
          .attr('fill-opacity', 0.16)
          .attr('stroke', col)
          .attr('stroke-width', 1.2)
      } else if (k === 'interface') {
        g.append('path')
          .attr('d', `M 0 ${-r} L ${r} 0 L 0 ${r} L ${-r} 0 Z`)
          .attr('fill', col)
          .attr('fill-opacity', 0.85)
          .attr('stroke', col)
          .attr('stroke-width', 1)
      } else if (k === 'enum') {
        const pts: string[] = []
        for (let i = 0; i < 6; i++) {
          const angle = (Math.PI / 3) * i - Math.PI / 2
          pts.push(`${r * Math.cos(angle)},${r * Math.sin(angle)}`)
        }
        g.append('path')
          .attr('d', `M ${pts.join(' L ')} Z`)
          .attr('fill', col)
          .attr('fill-opacity', 0.85)
          .attr('stroke', col)
          .attr('stroke-width', 1)
      } else {
        g.append('circle')
          .attr('r', r)
          .attr('fill', col)
          .attr('fill-opacity', 0.85)
          .attr('stroke', col)
          .attr('stroke-width', 1)
      }

      // Drillable badge — "+" on top-right when child_count > 0
      if (d.child_count > 0) {
        g.append('circle')
          .attr('class', 'hg-drill-badge')
          .attr('cx', r * 0.7)
          .attr('cy', -r * 0.7)
          .attr('r', 6)
          .attr('fill', '#C8915C')
          .attr('stroke', '#1A1410')
          .attr('stroke-width', 1.5)
        g.append('text')
          .attr('class', 'hg-drill-badge-text')
          .attr('x', r * 0.7)
          .attr('y', -r * 0.7)
          .attr('text-anchor', 'middle')
          .attr('dominant-baseline', 'central')
          .attr('font-size', '8px')
          .attr('font-weight', '700')
          .attr('fill', '#1A1410')
          .attr('pointer-events', 'none')
          .text('+')
      }
    })

    // Node labels
    nodeSel
      .append('text')
      .attr('class', 'hg-node-label')
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'middle')
      .attr('dy', (d) => {
        const k = d.kind.toLowerCase()
        const r = d.radius
        if (k === 'service') return r * 1.6 / 2 + 13
        if (k === 'package') return r * 1.4 / 2 + 12
        if (k === 'file') return r * 1.3 / 2 + 11
        return r + 12
      })
      .text((d) => d.name)

    // Sub-label: child count or package path
    nodeSel
      .filter((d) => {
        const k = d.kind.toLowerCase()
        return (k === 'package' || k === 'service' || k === 'file') && d.child_count > 0
      })
      .append('text')
      .attr('class', 'hg-node-sublabel')
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'middle')
      .attr('dy', (d) => {
        const k = d.kind.toLowerCase()
        const r = d.radius
        if (k === 'service') return r * 1.6 / 2 + 25
        if (k === 'package') return r * 1.4 / 2 + 24
        return r * 1.3 / 2 + 23
      })
      .text((d) => `${d.child_count} items`)

    // ---- Interaction ----
    const drag = d3
      .drag<SVGGElement, SimNode>()
      .on('start', (event: d3.D3DragEvent<SVGGElement, SimNode, SimNode>, d: SimNode) => {
        if (!event.active) simulationRef.current?.alphaTarget(0.3).restart()
        d.fx = d.x
        d.fy = d.y
      })
      .on('drag', (event: d3.D3DragEvent<SVGGElement, SimNode, SimNode>, d: SimNode) => {
        d.fx = event.x
        d.fy = event.y
      })
      .on('end', (event: d3.D3DragEvent<SVGGElement, SimNode, SimNode>, d: SimNode) => {
        if (!event.active) simulationRef.current?.alphaTarget(0)
        d.fx = null
        d.fy = null
      })

    nodeSel.call(drag)

    nodeSel
      .on('click', (_event: MouseEvent, d: SimNode) => {
        selectNode(d.id)
      })
      .on('dblclick', (_event: MouseEvent, d: SimNode) => {
        _event.stopPropagation()
        drillDown(d.id)
      })
      .on('mouseenter', (_event: MouseEvent, d: SimNode) => {
        // Highlight connected edges and neighbors
        const connectedIds = new Set<string>([d.id])
        simEdges.forEach((e) => {
          const src = (e.source as SimNode).id
          const tgt = (e.target as SimNode).id
          if (src === d.id) connectedIds.add(tgt)
          if (tgt === d.id) connectedIds.add(src)
        })
        nodeSel.classed('hg-node--dimmed', (n) => !connectedIds.has(n.id))
        nodeSel.classed('hg-node--highlighted', (n) => connectedIds.has(n.id) && n.id !== d.id)
        edgeSel.classed('hg-edge--dimmed', (e) => {
          const src = (e.source as SimNode).id
          const tgt = (e.target as SimNode).id
          return src !== d.id && tgt !== d.id
        })
      })
      .on('mouseleave', () => {
        nodeSel.classed('hg-node--dimmed', false)
        nodeSel.classed('hg-node--highlighted', false)
        edgeSel.classed('hg-edge--dimmed', false)
      })

    // ---- Force simulation ----
    const forces = LEVEL_FORCES[level] ?? LEVEL_FORCES.packages

    const simulation = d3
      .forceSimulation<SimNode>(simNodes)
      .force(
        'link',
        d3
          .forceLink<SimNode, SimEdge>(simEdges)
          .id((d) => d.id)
          .distance(forces.linkDistance)
          .strength(0.4),
      )
      .force('charge', d3.forceManyBody<SimNode>().strength(forces.charge))
      .force('center', d3.forceCenter<SimNode>(width / 2, height / 2))
      .force('collision', d3.forceCollide<SimNode>().radius((d) => d.radius + forces.collisionPad))

    simulationRef.current = simulation

    simulation.on('tick', () => {
      if (gen !== genRef.current) {
        simulation.stop()
        return
      }

      edgeSel
        .attr('x1', (d) => (d.source as SimNode).x ?? 0)
        .attr('y1', (d) => (d.source as SimNode).y ?? 0)
        .attr('x2', (d) => {
          const src = d.source as SimNode
          const tgt = d.target as SimNode
          const dx = (tgt.x ?? 0) - (src.x ?? 0)
          const dy = (tgt.y ?? 0) - (src.y ?? 0)
          const dist = Math.sqrt(dx * dx + dy * dy) || 1
          return (tgt.x ?? 0) - (dx / dist) * tgt.radius
        })
        .attr('y2', (d) => {
          const src = d.source as SimNode
          const tgt = d.target as SimNode
          const dx = (tgt.x ?? 0) - (src.x ?? 0)
          const dy = (tgt.y ?? 0) - (src.y ?? 0)
          const dist = Math.sqrt(dx * dx + dy * dy) || 1
          return (tgt.y ?? 0) - (dy / dist) * tgt.radius
        })

      edgeLabelSel
        .attr('x', (d) => {
          const sx = (d.source as SimNode).x ?? 0
          const tx = (d.target as SimNode).x ?? 0
          return (sx + tx) / 2
        })
        .attr('y', (d) => {
          const sy = (d.source as SimNode).y ?? 0
          const ty = (d.target as SimNode).y ?? 0
          return (sy + ty) / 2
        })

      nodeSel.attr('transform', (d) => `translate(${d.x ?? 0},${d.y ?? 0})`)
    })

    // Fit all nodes after simulation settles a bit
    setTimeout(() => {
      if (gen !== genRef.current) return
      if (simNodes.length === 0) return
      let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity
      for (const n of simNodes) {
        const x = n.x ?? 0
        const y = n.y ?? 0
        if (x - n.radius < minX) minX = x - n.radius
        if (x + n.radius > maxX) maxX = x + n.radius
        if (y - n.radius < minY) minY = y - n.radius
        if (y + n.radius > maxY) maxY = y + n.radius
      }
      const pad = 80
      const bw = maxX - minX + pad * 2
      const bh = maxY - minY + pad * 2
      const scale = Math.min(width / bw, height / bh, 2.5)
      const cx = (minX + maxX) / 2
      const cy = (minY + maxY) / 2
      d3.select<SVGSVGElement, unknown>(thisSvg)
        .transition()
        .duration(500)
        .call(
          zoom.transform,
          d3.zoomIdentity.translate(width / 2 - cx * scale, height / 2 - cy * scale).scale(scale),
        )
    }, 900)

    return () => {
      simulation.stop()
    }
  }, [loadState, nodes, edges, level, drillDown, selectNode])

  // ---- Selection ring ----
  useEffect(() => {
    if (!svgRef.current || !selectedNodeId) return
    const svg = d3.select(svgRef.current)
    svg.selectAll('.hg-selection-ring').remove()

    const nodeSel = svg.selectAll<SVGGElement, SimNode>('g.hg-node')
    nodeSel.each(function (d: SimNode) {
      if (d.id !== selectedNodeId) return
      const g = d3.select(this)
      const r = d.radius
      const ring = g.append('g').attr('class', 'hg-selection-ring')
      ring
        .append('circle')
        .attr('r', r + 8)
        .attr('fill', 'none')
        .attr('stroke', d.color)
        .attr('stroke-width', 2)
        .attr('stroke-opacity', 0.8)
      ring
        .append('circle')
        .attr('r', r + 8)
        .attr('fill', 'none')
        .attr('stroke', d.color)
        .attr('stroke-width', 2)
        .attr('stroke-opacity', 0.6)
        .append('animate')
        .attr('attributeName', 'r')
        .attr('from', r + 8)
        .attr('to', r + 20)
        .attr('dur', '1.5s')
        .attr('repeatCount', 'indefinite')
    })
  })

  // ---- ResizeObserver ----
  useEffect(() => {
    const container = containerRef.current
    const svgEl = svgRef.current
    if (!container || !svgEl) return

    const ro = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (!entry) return
      const { width, height } = entry.contentRect
      svgEl.setAttribute('width', String(width))
      svgEl.setAttribute('height', String(height))
      if (simulationRef.current) {
        simulationRef.current
          .force('center', d3.forceCenter(width / 2, height / 2))
          .alpha(0.2)
          .restart()
      }
    })
    ro.observe(container)
    return () => ro.disconnect()
  }, [])

  // ---- Zoom controls ----
  const handleZoomIn = useCallback(() => {
    if (!svgRef.current || !zoomRef.current) return
    d3.select<SVGSVGElement, unknown>(svgRef.current)
      .transition()
      .duration(250)
      .call(zoomRef.current.scaleBy, 1.4)
  }, [])

  const handleZoomOut = useCallback(() => {
    if (!svgRef.current || !zoomRef.current) return
    d3.select<SVGSVGElement, unknown>(svgRef.current)
      .transition()
      .duration(250)
      .call(zoomRef.current.scaleBy, 1 / 1.4)
  }, [])

  const handleZoomReset = useCallback(() => {
    if (!svgRef.current || !zoomRef.current) return
    d3.select<SVGSVGElement, unknown>(svgRef.current)
      .transition()
      .duration(400)
      .call(zoomRef.current.transform, d3.zoomIdentity)
  }, [])

  // ---- Breadcrumb click handler ----
  const handleBreadcrumbClick = useCallback(
    (level: string, scope: string | undefined) => {
      navigateTo(level, scope)
    },
    [navigateTo],
  )

  // ---- Empty state after ready ----
  const isEmpty = loadState === 'ready' && nodes.length === 0

  return (
    <div className={styles.hierarchyGraph}>
      {/* Breadcrumb bar */}
      <div className={styles.breadcrumbs} aria-label="Navigation breadcrumbs">
        <button
          className={styles.breadcrumbItem}
          onClick={() => handleBreadcrumbClick('packages', undefined)}
        >
          Workspace
        </button>
        {breadcrumbs.map((crumb, i) => (
          <span key={i} className={styles.breadcrumbEntry}>
            <span className={styles.breadcrumbSep} aria-hidden="true">›</span>
            <button
              className={styles.breadcrumbItem}
              onClick={() => handleBreadcrumbClick(crumb.level, crumb.scope)}
            >
              {crumb.label}
            </button>
          </span>
        ))}
        {/* Current level indicator */}
        <span className={styles.breadcrumbEntry}>
          <span className={styles.breadcrumbSep} aria-hidden="true">›</span>
          <span className={styles.breadcrumbCurrent}>{levelLabel(level)}</span>
        </span>
      </div>

      {/* Graph canvas */}
      <div ref={containerRef} className={styles.canvas}>
        <svg ref={svgRef} role="img" aria-label={`Architecture graph — ${levelLabel(level)} level`} />

        {loadState === 'loading' && (
          <div className={styles.overlay}>
            <p className={styles.overlayText}>Loading {levelLabel(level).toLowerCase()}...</p>
          </div>
        )}

        {loadState === 'error' && (
          <div className={styles.overlay}>
            <p className={styles.overlayTitle}>Failed to load hierarchy</p>
            {errorMessage && <p className={styles.overlayText}>{errorMessage}</p>}
            <button
              className={styles.retryBtn}
              onClick={() => loadLevel(level)}
            >
              Retry
            </button>
          </div>
        )}

        {isEmpty && (
          <div className={styles.overlay}>
            <p className={styles.overlayTitle}>No data at this level</p>
            <p className={styles.overlayText}>
              The backend returned no nodes for the current scope.
            </p>
          </div>
        )}

        <ZoomControls
          onZoomIn={handleZoomIn}
          onZoomOut={handleZoomOut}
          onZoomReset={handleZoomReset}
        />

        {/* Level indicator badge */}
        <div className={styles.levelBadge}>{levelLabel(level)}</div>

        {/* Legend */}
        <div className={styles.legend}>
          {Object.entries(HIERARCHY_COLORS)
            .filter(([k]) => ['service', 'package', 'file', 'class', 'interface', 'enum'].includes(k))
            .map(([kind, color]) => (
              <div key={kind} className={styles.legendItem}>
                <span className={styles.legendSwatch} style={{ background: color }} />
                <span className={styles.legendLabel}>{kind}</span>
              </div>
            ))}
          <div className={`${styles.legendItem} ${styles.legendItemDrill}`}>
            <span className={styles.drillIndicator}>+</span>
            <span className={styles.legendLabel}>drillable</span>
          </div>
        </div>
      </div>

      {/* Hint bar */}
      <div className={styles.hintBar}>
        <span>Double-click a node to drill down</span>
        <span className={styles.hintSep}>·</span>
        <span>Drag to rearrange</span>
        <span className={styles.hintSep}>·</span>
        <span>Scroll to zoom</span>
      </div>
    </div>
  )
}
