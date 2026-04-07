import * as d3 from 'd3'
import { useEffect, useRef, useCallback, useState } from 'react'
import { useHierarchyData } from '../../hooks/useHierarchyData'
import { useHierarchyStore } from '../../stores/hierarchy.store'
import { ZoomControls } from '../ZoomControls'
import { CodeModal } from './CodeModal'
import { api } from '../../api'
import type { HierarchyNode, HierarchyEdge } from '../../types/api.types'
import styles from './HierarchyGraph.module.css'

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const HIERARCHY_COLORS: Record<string, string> = {
  service: '#ffbf00', // amber — inner ring
  package: '#6495ed', // cornflower — outer ring
  file: '#3fb950', // green
  class: '#6495ed',
  interface: '#58a6ff',
  method: '#3fb950',
  function: '#ffbf00',
  enum: '#bc8cff',
  struct: '#6495ed',
  module: '#ffbf00',
  constant: '#d29922',
  field: '#8b949e',
}
const DEFAULT_COLOR = '#64748b'

function nodeColor(kind: string): string {
  return HIERARCHY_COLORS[kind.toLowerCase()] ?? DEFAULT_COLOR
}

// Shape paths centered at (0,0) for each node kind.
// Returns an SVG path `d` string sized to the given radius.
function shapePath(kind: string, r: number): string {
  const k = kind.toLowerCase()
  switch (k) {
    case 'service': {
      // Rounded rectangle (wide) — server/container
      const w = r * 2.2, h = r * 1.4, rx = r * 0.25
      return `M ${-w/2 + rx} ${-h/2}
        h ${w - 2*rx} a ${rx} ${rx} 0 0 1 ${rx} ${rx}
        v ${h - 2*rx} a ${rx} ${rx} 0 0 1 ${-rx} ${rx}
        h ${-(w - 2*rx)} a ${rx} ${rx} 0 0 1 ${-rx} ${-rx}
        v ${-(h - 2*rx)} a ${rx} ${rx} 0 0 1 ${rx} ${-rx} Z`
    }
    case 'package': {
      // Hexagon — package/module
      const pts: string[] = []
      for (let i = 0; i < 6; i++) {
        const a = (Math.PI / 3) * i - Math.PI / 2
        pts.push(`${r * 1.1 * Math.cos(a)},${r * 1.1 * Math.sin(a)}`)
      }
      return `M ${pts[0]} L ${pts[1]} L ${pts[2]} L ${pts[3]} L ${pts[4]} L ${pts[5]} Z`
    }
    case 'file': {
      // Rectangle with folded top-right corner — file icon
      const w = r * 1.8, h = r * 1.5, fold = r * 0.35
      return `M ${-w/2} ${-h/2}
        L ${w/2 - fold} ${-h/2}
        L ${w/2} ${-h/2 + fold}
        L ${w/2} ${h/2}
        L ${-w/2} ${h/2} Z`
    }
    case 'class':
    case 'struct': {
      // Rounded square — solid, foundational
      const s = r * 1.6, rx = r * 0.2
      return `M ${-s/2 + rx} ${-s/2}
        h ${s - 2*rx} a ${rx} ${rx} 0 0 1 ${rx} ${rx}
        v ${s - 2*rx} a ${rx} ${rx} 0 0 1 ${-rx} ${rx}
        h ${-(s - 2*rx)} a ${rx} ${rx} 0 0 1 ${-rx} ${-rx}
        v ${-(s - 2*rx)} a ${rx} ${rx} 0 0 1 ${rx} ${-rx} Z`
    }
    case 'interface':
    case 'trait': {
      // Diamond — abstract concept
      const d = r * 1.3
      return `M 0 ${-d} L ${d} 0 L 0 ${d} L ${-d} 0 Z`
    }
    case 'enum': {
      // Octagon — enumerated set
      const s = r * 1.1, c = s * 0.4
      return `M ${-s + c} ${-s}
        L ${s - c} ${-s} L ${s} ${-s + c}
        L ${s} ${s - c} L ${s - c} ${s}
        L ${-s + c} ${s} L ${-s} ${s - c}
        L ${-s} ${-s + c} Z`
    }
    default:
      // Circle (method, function, constant, field, etc.)
      return ''  // empty = use <circle> element instead
  }
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

// ---------------------------------------------------------------------------
// Radial layout
// ---------------------------------------------------------------------------

interface NodePos {
  x: number
  y: number
  size: number
  color: string
}

/**
 * Computes radial positions for all nodes.
 * Services go to the inner ring, everything else to the outer ring.
 * For large node counts (files/symbols) we use multiple rings to avoid overlap.
 */
function radialLayout(
  nodes: HierarchyNode[],
  width: number,
  height: number,
): Map<string, NodePos> {
  const cx = width / 2
  const cy = height / 2
  const positions = new Map<string, NodePos>()

  if (nodes.length === 0) return positions

  const minDim = Math.min(width, height)

  // Determine ring assignment
  const isInner = (n: HierarchyNode) => n.kind === 'service'

  const innerNodes = nodes.filter(isInner)
  const outerNodes = nodes.filter((n) => !isInner(n))

  // For very large outer rings, we split into multiple rings
  const MAX_PER_RING = 24

  function placeRing(
    ring: HierarchyNode[],
    radius: number,
    offsetAngle = 0,
  ): void {
    ring.forEach((n, i) => {
      const angle = offsetAngle + (i / Math.max(ring.length, 1)) * Math.PI * 2
      const size = Math.max(12, Math.min(55, Math.sqrt(n.weight) * 0.9))
      positions.set(n.id, {
        x: cx + Math.cos(angle) * radius,
        y: cy + Math.sin(angle) * radius,
        size,
        color: nodeColor(n.kind),
      })
    })
  }

  if (innerNodes.length > 0) {
    const innerR = minDim * 0.14
    placeRing(innerNodes, innerR, -Math.PI / 2)
  } else if (outerNodes.length > 0 && outerNodes.length <= 4) {
    // If no inner ring and few nodes, just put them all in one ring centered
    const r = minDim * 0.22
    placeRing(outerNodes, r, -Math.PI / 2)
    return positions
  }

  if (outerNodes.length > 0) {
    if (outerNodes.length <= MAX_PER_RING) {
      const outerR = minDim * 0.31
      const offset = innerNodes.length > 0 ? Math.PI / outerNodes.length : -Math.PI / 2
      placeRing(outerNodes, outerR, offset)
    } else {
      // Multi-ring: distribute across concentric rings
      const rings = Math.ceil(outerNodes.length / MAX_PER_RING)
      for (let ring = 0; ring < rings; ring++) {
        const start = ring * MAX_PER_RING
        const slice = outerNodes.slice(start, start + MAX_PER_RING)
        const r = minDim * (0.28 + ring * 0.12)
        const offset = ring % 2 === 0 ? -Math.PI / 2 : -Math.PI / 2 + Math.PI / slice.length
        placeRing(slice, r, offset)
      }
    }
  }

  return positions
}

// ---------------------------------------------------------------------------
// Selected node details (for side panel)
// ---------------------------------------------------------------------------

interface SelectedDetails {
  node: HierarchyNode
  edgesOut: HierarchyEdge[]
  edgesIn: HierarchyEdge[]
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
  const zoomRef = useRef<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null)
  const genRef = useRef(0)

  const { loadLevel } = useHierarchyData(workspacePath)

  const nodes = useHierarchyStore((s) => s.nodes)
  const edges = useHierarchyStore((s) => s.edges)
  const level = useHierarchyStore((s) => s.level)
  const breadcrumbs = useHierarchyStore((s) => s.breadcrumbs)
  const loadState = useHierarchyStore((s) => s.loadState)
  const errorMessage = useHierarchyStore((s) => s.errorMessage)
  const selectedNodeId = useHierarchyStore((s) => s.selectedNodeId)
  const drillDown = useHierarchyStore((s) => s.drillDown)
  const navigateTo = useHierarchyStore((s) => s.navigateTo)
  const selectNode = useHierarchyStore((s) => s.selectNode)
  const searchFilter = useHierarchyStore((s) => s.searchFilter)
  const highlightedEdge = useHierarchyStore((s) => s.highlightedEdge)
  const setSearchFilter = useHierarchyStore((s) => s.setSearchFilter)
  const setHighlightedEdge = useHierarchyStore((s) => s.setHighlightedEdge)

  // Side panel state (React-driven, not D3)
  const [selectedDetails, setSelectedDetails] = useState<SelectedDetails | null>(null)

  // Code modal state
  const [codeModal, setCodeModal] = useState<{
    filePath: string
    content: string | null
    loading: boolean
    error: string | null
  } | null>(null)

  // Minimap ref
  const minimapRef = useRef<SVGSVGElement>(null)

  // -------------------------------------------------------------------------
  // Main D3 render
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

    const svgSel = d3
      .select<SVGSVGElement, unknown>(thisSvg)
      .attr('width', width)
      .attr('height', height)

    // Apply search filter: when active, only render matching nodes + edges between them
    const filterSet = searchFilter ? new Set(searchFilter) : null
    const visibleNodes = filterSet ? nodes.filter((n) => filterSet.has(n.id)) : nodes
    const visibleEdges = filterSet
      ? edges.filter((e) => filterSet.has(e.source) && filterSet.has(e.target))
      : edges

    // Compute radial positions once (using only visible nodes)
    const positions = radialLayout(visibleNodes, width, height)

    // Build node lookup (full set so connections can still resolve names)
    const nodeById = new Map<string, HierarchyNode>(nodes.map((n) => [n.id, n]))

    // Filter edges to those with valid endpoints
    const validEdges = visibleEdges.filter(
      (e) => positions.has(e.source) && positions.has(e.target),
    )

    // ---- Defs (gradients, filters) ----
    const defs = svgSel.append('defs')

    // Blur filter for glow halos
    defs
      .append('filter')
      .attr('id', `hg-glow-${gen}`)
      .attr('x', '-50%')
      .attr('y', '-50%')
      .attr('width', '200%')
      .attr('height', '200%')
      .append('feGaussianBlur')
      .attr('in', 'SourceGraphic')
      .attr('stdDeviation', '4')

    // Per-edge gradients
    validEdges.forEach((e) => {
      const srcPos = positions.get(e.source)!
      const tgtPos = positions.get(e.target)!
      const srcColor = nodeColor(nodeById.get(e.source)?.kind ?? '')
      const tgtColor = nodeColor(nodeById.get(e.target)?.kind ?? '')

      defs
        .append('linearGradient')
        .attr('id', `hg-grad-${gen}-${e.source}-${e.target}`)
        .attr('gradientUnits', 'userSpaceOnUse')
        .attr('x1', srcPos.x)
        .attr('y1', srcPos.y)
        .attr('x2', tgtPos.x)
        .attr('y2', tgtPos.y)
        .selectAll('stop')
        .data([
          { offset: '0%', color: srcColor },
          { offset: '100%', color: tgtColor },
        ])
        .join('stop')
        .attr('offset', (d) => d.offset)
        .attr('stop-color', (d) => d.color)
    })

    // ---- Zoom + pan ----
    const zoomGroup = svgSel.append('g').attr('class', 'hg-zoom-group')
    const edgesGroup = zoomGroup.append('g').attr('class', 'hg-edges')
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
    const edgeGroups = edgesGroup
      .selectAll<SVGGElement, HierarchyEdge>('g.hg-edge-group')
      .data(validEdges)
      .join('g')
      .attr('class', 'hg-edge-group')

    // Bezier path builder
    const buildPath = (e: HierarchyEdge): string => {
      const s = positions.get(e.source)!
      const t = positions.get(e.target)!
      const mx = (s.x + t.x) / 2
      const my = (s.y + t.y) / 2
      const dx = t.x - s.x
      const dy = t.y - s.y
      // Perpendicular offset to curve outward
      const cx1 = mx - dy * 0.2
      const cy1 = my + dx * 0.2
      return `M ${s.x} ${s.y} Q ${cx1} ${cy1} ${t.x} ${t.y}`
    }

    const edgePaths = edgeGroups
      .append('path')
      .attr('class', 'hg-edge-path')
      .attr('d', buildPath)
      .attr('fill', 'none')
      .attr('stroke', (e) => `url(#hg-grad-${gen}-${e.source}-${e.target})`)
      .attr('stroke-width', (e) => Math.max(1, Math.min(4, e.weight / 200)))
      .attr('opacity', 0.4)

    // Flowing dots along each edge
    edgeGroups.append('circle').attr('class', 'hg-flow-dot').attr('r', 2.5).each(function (e) {
      const dot = d3.select(this)
      const srcColor = nodeColor(nodeById.get(e.source)?.kind ?? '')
      dot.attr('fill', srcColor).attr('opacity', 0.7)
      const dur = Math.max(2, 8 - e.weight / 200)
      dot
        .append('animateMotion')
        .attr('dur', `${dur}s`)
        .attr('repeatCount', 'indefinite')
        .attr('path', buildPath(e))
    })

    // ---- Nodes ----
    const nodeGroups = nodesGroup
      .selectAll<SVGGElement, HierarchyNode>('g.hg-node')
      .data(visibleNodes, (d) => d.id)
      .join('g')
      .attr('class', 'hg-node')
      .attr('transform', (d) => {
        const pos = positions.get(d.id)
        return pos ? `translate(${pos.x},${pos.y})` : 'translate(0,0)'
      })

    nodeGroups.each(function (d: HierarchyNode) {
      const g = d3.select(this)
      const pos = positions.get(d.id)
      if (!pos) return
      const { size, color } = pos
      const k = d.kind.toLowerCase()

      const path = shapePath(k, size)

      if (path) {
        // Non-circle shape — use <path> for both glow and main shape
        // Outer glow halo
        g.append('path')
          .attr('d', shapePath(k, size + 10))
          .attr('fill', color)
          .attr('fill-opacity', 0.15)
          .attr('filter', `url(#hg-glow-${gen})`)
          .attr('pointer-events', 'none')

        // Main shape
        g.append('path')
          .attr('class', 'hg-node-shape')
          .attr('d', path)
          .attr('fill', color)
          .attr('fill-opacity', 0.18)
          .attr('stroke', color)
          .attr('stroke-width', 1.5)
          .attr('stroke-opacity', 0.7)

        // Kind label inside shape (small, uppercase)
        g.append('text')
          .attr('class', 'hg-kind-label')
          .attr('text-anchor', 'middle')
          .attr('dominant-baseline', 'central')
          .attr('fill', color)
          .attr('fill-opacity', 0.9)
          .style('font-size', `${Math.max(8, Math.min(12, size * 0.4))}px`)
          .style('font-weight', '700')
          .style('font-family', 'var(--font-mono)')
          .style('text-transform', 'uppercase')
          .style('letter-spacing', '0.05em')
          .style('pointer-events', 'none')
          .text(k.slice(0, 3))
      } else {
        // Circle shape (methods, functions, constants, etc.)
        g.append('circle')
          .attr('r', size + 10)
          .attr('fill', color)
          .attr('fill-opacity', 0.18)
          .attr('filter', `url(#hg-glow-${gen})`)
          .attr('pointer-events', 'none')

        g.append('circle')
          .attr('class', 'hg-node-circle')
          .attr('r', size)
          .attr('fill', color)
          .attr('fill-opacity', 0.82)
          .attr('stroke', color)
          .attr('stroke-width', 1.5)
          .attr('stroke-opacity', 0.6)
      }

      // Pulse rings for service nodes (3 animated expanding rings)
      if (k === 'service') {
        for (let i = 0; i < 3; i++) {
          const ring = g.append('circle')
            .attr('class', 'hg-pulse-ring')
            .attr('r', size * 1.3)
            .attr('fill', 'none')
            .attr('stroke', color)
            .attr('stroke-width', 1.5)
            .attr('opacity', 0)
            .attr('pointer-events', 'none')

          ring
            .append('animate')
            .attr('attributeName', 'r')
            .attr('from', size * 1.3)
            .attr('to', size * 2.6)
            .attr('dur', '3s')
            .attr('begin', `${i}s`)
            .attr('repeatCount', 'indefinite')

          ring
            .append('animate')
            .attr('attributeName', 'opacity')
            .attr('from', 0.5)
            .attr('to', 0)
            .attr('dur', '3s')
            .attr('begin', `${i}s`)
            .attr('repeatCount', 'indefinite')
        }
      }

      // Drillable badge "+" if child_count > 0
      if (d.child_count > 0) {
        g.append('circle')
          .attr('class', 'hg-drill-badge')
          .attr('cx', size * 0.65)
          .attr('cy', -size * 0.65)
          .attr('r', 6)
          .attr('fill', '#ffbf00')
          .attr('stroke', '#0d1117')
          .attr('stroke-width', 1.5)
          .attr('pointer-events', 'none')
        g.append('text')
          .attr('x', size * 0.65)
          .attr('y', -size * 0.65)
          .attr('text-anchor', 'middle')
          .attr('dominant-baseline', 'central')
          .attr('font-size', '8px')
          .attr('font-weight', '700')
          .attr('fill', '#0d1117')
          .attr('pointer-events', 'none')
          .text('+')
      }

      // Name label below node
      const labelOffset = size + 14
      g.append('text')
        .attr('class', 'hg-node-label')
        .attr('y', labelOffset)
        .attr('text-anchor', 'middle')
        .attr('dominant-baseline', 'hanging')
        .attr('fill', color)
        .attr('pointer-events', 'none')
        .text(d.name)

      // Sub-label: child count
      if (d.child_count > 0) {
        g.append('text')
          .attr('class', 'hg-node-sublabel')
          .attr('y', labelOffset + 15)
          .attr('text-anchor', 'middle')
          .attr('dominant-baseline', 'hanging')
          .attr('pointer-events', 'none')
          .text(`${d.child_count} items`)
      }
    })

    // ---- Interactions ----
    nodeGroups
      .style('cursor', 'pointer')
      .on('mouseenter', (_event: MouseEvent, d: HierarchyNode) => {
        const connectedIds = new Set<string>([d.id])
        validEdges.forEach((e) => {
          if (e.source === d.id) connectedIds.add(e.target)
          if (e.target === d.id) connectedIds.add(e.source)
        })
        nodeGroups.classed('hg-node--dimmed', (n) => !connectedIds.has(n.id))
        nodeGroups.classed('hg-node--highlighted', (n) => connectedIds.has(n.id) && n.id !== d.id)
        edgePaths.classed(
          'hg-edge-path--dimmed',
          (e) => e.source !== d.id && e.target !== d.id,
        )
        edgePaths.classed(
          'hg-edge-path--highlight',
          (e) => e.source === d.id || e.target === d.id,
        )
      })
      .on('mouseleave', () => {
        nodeGroups.classed('hg-node--dimmed', false)
        nodeGroups.classed('hg-node--highlighted', false)
        edgePaths.classed('hg-edge-path--dimmed', false)
        edgePaths.classed('hg-edge-path--highlight', false)
      })
      .on('click', (_event: MouseEvent, d: HierarchyNode) => {
        selectNode(d.id)
        const edgesOut = validEdges.filter((e) => e.source === d.id)
        const edgesIn = validEdges.filter((e) => e.target === d.id)
        setSelectedDetails({ node: d, edgesOut, edgesIn })
      })
      .on('dblclick', (event: MouseEvent, d: HierarchyNode) => {
        event.stopPropagation()
        drillDown(d.id)
      })

    // Drag — positional drag updates translate directly (no sim)
    const drag = d3
      .drag<SVGGElement, HierarchyNode>()
      .on('drag', function (event: d3.D3DragEvent<SVGGElement, HierarchyNode, HierarchyNode>) {
        const g = d3.select<SVGGElement, HierarchyNode>(this)
        // Read current transform, apply delta
        const current = (this as SVGGElement).getAttribute('transform') ?? 'translate(0,0)'
        const match = current.match(/translate\(([^,]+),([^)]+)\)/)
        const cx = match ? parseFloat(match[1]) : 0
        const cy = match ? parseFloat(match[2]) : 0
        const nx = cx + event.dx
        const ny = cy + event.dy
        g.attr('transform', `translate(${nx},${ny})`)

        // Update position map for edge redraw (best-effort during drag)
        const d = event.subject
        const pos = positions.get(d.id)
        if (pos) {
          pos.x = nx
          pos.y = ny
          // Redraw connected edges
          edgePaths.filter((e) => e.source === d.id || e.target === d.id).attr('d', buildPath)
        }
      })

    nodeGroups.call(drag)

    // ---- Minimap ----
    renderMinimap(positions, visibleNodes, width, height)

    // ---- Auto-fit on load ----
    if (nodes.length > 0) {
      let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity
      positions.forEach(({ x, y, size }) => {
        if (x - size < minX) minX = x - size
        if (x + size > maxX) maxX = x + size
        if (y - size < minY) minY = y - size
        if (y + size > maxY) maxY = y + size
      })
      const pad = 100
      const bw = maxX - minX + pad * 2
      const bh = maxY - minY + pad * 2
      const scale = Math.min(width / bw, height / bh, 2.5)
      const midX = (minX + maxX) / 2
      const midY = (minY + maxY) / 2
      svgSel
        .transition()
        .duration(400)
        .call(
          zoom.transform,
          d3.zoomIdentity
            .translate(width / 2 - midX * scale, height / 2 - midY * scale)
            .scale(scale),
        )
    }

    // Clear selected details when level changes
    setSelectedDetails(null)

    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadState, nodes, edges, level, searchFilter, drillDown, selectNode])

  // ---- Selection highlight ----
  useEffect(() => {
    if (!svgRef.current) return
    const svgSel = d3.select(svgRef.current)
    svgSel.selectAll('.hg-selection-ring').remove()

    if (!selectedNodeId) return

    const nodeGroups = svgSel.selectAll<SVGGElement, HierarchyNode>('g.hg-node')
    nodeGroups.each(function (d: HierarchyNode) {
      if (d.id !== selectedNodeId) return
      const g = d3.select(this)
      const container = containerRef.current
      const width = container?.clientWidth || 900
      const height = container?.clientHeight || 600
      const pos = radialLayout(nodes, width, height).get(d.id)
      const size = pos?.size ?? 24

      const ring = g.append('g').attr('class', 'hg-selection-ring')
      ring
        .append('circle')
        .attr('r', size + 8)
        .attr('fill', 'none')
        .attr('stroke', nodeColor(d.kind))
        .attr('stroke-width', 2)
        .attr('stroke-opacity', 0.9)
        .attr('pointer-events', 'none')
    })
  }, [selectedNodeId, nodes])

  // ---- Highlighted edge (from panel connection click) ----
  useEffect(() => {
    if (!svgRef.current) return
    const svgSel = d3.select(svgRef.current)
    const edgePaths = svgSel.selectAll<SVGPathElement, HierarchyEdge>('.hg-edge-path')

    if (!highlightedEdge) {
      // Remove any lingering highlight-edge classes
      edgePaths.classed('hg-edge-path--edge-highlight', false)
      edgePaths.classed('hg-edge-path--edge-dimmed', false)
      return
    }

    edgePaths.classed(
      'hg-edge-path--edge-highlight',
      (e) => e.source === highlightedEdge.source && e.target === highlightedEdge.target,
    )
    edgePaths.classed(
      'hg-edge-path--edge-dimmed',
      (e) => !(e.source === highlightedEdge.source && e.target === highlightedEdge.target),
    )
  }, [highlightedEdge])

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
    })
    ro.observe(container)
    return () => ro.disconnect()
  }, [])

  // ---- Minimap render ----
  function renderMinimap(
    positions: Map<string, NodePos>,
    nodes: HierarchyNode[],
    width: number,
    height: number,
  ) {
    const mm = minimapRef.current
    if (!mm) return

    const mw = 160
    const mh = 100
    const scaleX = mw / width
    const scaleY = mh / height

    d3.select(mm).selectAll('*').remove()
    const mmSel = d3.select(mm)

    nodes.forEach((n) => {
      const pos = positions.get(n.id)
      if (!pos) return
      const r = Math.max(3, pos.size * Math.min(scaleX, scaleY))
      mmSel
        .append('circle')
        .attr('cx', pos.x * scaleX)
        .attr('cy', pos.y * scaleY)
        .attr('r', r)
        .attr('fill', pos.color)
        .attr('fill-opacity', 0.75)
    })
  }

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

  // ---- Breadcrumb click ----
  const handleBreadcrumbClick = useCallback(
    (level: string, scope: string | undefined) => {
      navigateTo(level, scope)
    },
    [navigateTo],
  )

  const isEmpty = loadState === 'ready' && nodes.length === 0

  // ---- Code modal helpers ----
  async function openCodeModal(filePath: string) {
    setCodeModal({ filePath, content: null, loading: true, error: null })
    try {
      const result = await api.fileContent(workspacePath, filePath)
      setCodeModal({ filePath, content: result.content, loading: false, error: null })
    } catch (err) {
      setCodeModal({
        filePath,
        content: null,
        loading: false,
        error: err instanceof Error ? err.message : 'Failed to load file',
      })
    }
  }

  // ---- Side panel contents ----
  function renderPanel() {
    if (!selectedDetails) {
      return (
        <p className={styles.panelPlaceholder}>Click a node to see details</p>
      )
    }
    const { node, edgesOut, edgesIn } = selectedDetails
    const color = nodeColor(node.kind)

    function renderConnItem(
      e: HierarchyEdge,
      dir: 'out' | 'in',
      peer: HierarchyNode | undefined,
      key: string,
    ) {
      const isHighlighted =
        highlightedEdge?.source === e.source && highlightedEdge?.target === e.target
      const peerFilePath = peer?.file_path
      const label = dir === 'out' ? `→ ${peer?.name ?? e.target}` : `← ${peer?.name ?? e.source}`

      function handleConnClick() {
        if (isHighlighted) {
          setHighlightedEdge(null)
        } else {
          setHighlightedEdge({ source: e.source, target: e.target })
        }
      }

      return (
        <div
          key={key}
          className={`${styles.connItem} ${isHighlighted ? styles.connItemHighlighted : ''}`}
        >
          <button className={styles.connClickArea} onClick={handleConnClick} title="Highlight edge">
            <span className={styles.connDir}>{label}</span>
            <span className={styles.connCount}>{e.weight}</span>
          </button>
          {peerFilePath ? (
            <button
              className={styles.connCodeBtn}
              title={`View code: ${peerFilePath}`}
              onClick={() => openCodeModal(peerFilePath)}
            >
              <span className="material-symbols-outlined">code</span>
            </button>
          ) : (
            <span className={styles.connCodeBtnDisabled} title="Navigate to file level to see code">
              <span className="material-symbols-outlined">code</span>
            </span>
          )}
        </div>
      )
    }

    return (
      <>
        <div className={styles.panelNodeName} style={{ color }}>
          {node.name}
        </div>
        <div className={styles.panelStats}>
          <div className={styles.statCard}>
            <div className={styles.statValue} style={{ color }}>
              {node.child_count}
            </div>
            <div className={styles.statLabel}>Items</div>
          </div>
          <div className={styles.statCard}>
            <div className={styles.statValue} style={{ color }}>
              {node.weight.toLocaleString()}
            </div>
            <div className={styles.statLabel}>Weight</div>
          </div>
        </div>
        {node.kind && (
          <div className={styles.panelKind} style={{ color }}>
            {node.kind.toUpperCase()}
          </div>
        )}
        {(edgesOut.length > 0 || edgesIn.length > 0) && (
          <div className={styles.panelSection}>
            <div className={styles.panelSectionTitle}>Connections</div>
            {edgesOut.map((e) => {
              const tgt = nodes.find((n) => n.id === e.target)
              return renderConnItem(e, 'out', tgt, `out-${e.target}`)
            })}
            {edgesIn.map((e) => {
              const src = nodes.find((n) => n.id === e.source)
              return renderConnItem(e, 'in', src, `in-${e.source}`)
            })}
          </div>
        )}
        {node.file_path && (
          <div className={styles.panelSection}>
            <div className={styles.panelSectionTitle}>File</div>
            <div className={styles.connItem}>
              <span className={styles.fileName}>{node.file_path}</span>
              <button
                className={styles.connCodeBtn}
                title={`View code: ${node.file_path}`}
                onClick={() => openCodeModal(node.file_path!)}
              >
                <span className="material-symbols-outlined">code</span>
              </button>
            </div>
          </div>
        )}
        {node.child_count > 0 && (
          <button
            className={styles.drillBtn}
            onClick={() => drillDown(node.id)}
          >
            Drill down →
          </button>
        )}
      </>
    )
  }

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
        <span className={styles.breadcrumbEntry}>
          <span className={styles.breadcrumbSep} aria-hidden="true">›</span>
          <span className={styles.breadcrumbCurrent}>{levelLabel(level)}</span>
        </span>
      </div>

      {/* Main area: graph canvas + side panel */}
      <div className={styles.mainArea}>
        {/* Graph canvas */}
        <div ref={containerRef} className={styles.canvas}>
          <svg
            ref={svgRef}
            role="img"
            aria-label={`Architecture graph — ${levelLabel(level)} level`}
          />

          {loadState === 'loading' && (
            <div className={styles.overlay}>
              <p className={styles.overlayText}>Loading {levelLabel(level).toLowerCase()}...</p>
            </div>
          )}

          {loadState === 'error' && (
            <div className={styles.overlay}>
              <p className={styles.overlayTitle}>Failed to load hierarchy</p>
              {errorMessage && <p className={styles.overlayText}>{errorMessage}</p>}
              <button className={styles.retryBtn} onClick={() => loadLevel(level)}>
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

          {/* Level badge */}
          <div className={styles.levelBadge}>{levelLabel(level)}</div>

          {/* Search filter badge */}
          {searchFilter !== null && (
            <div className={styles.filterBadge}>
              <span>
                Filtered: {searchFilter.length} node{searchFilter.length !== 1 ? 's' : ''}
              </span>
              <button
                className={styles.filterClearBtn}
                onClick={() => setSearchFilter(null)}
                aria-label="Clear filter"
              >
                ×
              </button>
            </div>
          )}

          {/* Minimap */}
          <div className={styles.minimap}>
            <svg
              ref={minimapRef}
              width={160}
              height={100}
              aria-hidden="true"
            />
          </div>

          {/* Shape legend */}
          <div className={styles.shapeLegend}>
            {[
              { kind: 'service', label: 'Service', shape: 'rrect' },
              { kind: 'package', label: 'Package', shape: 'hex' },
              { kind: 'file', label: 'File', shape: 'filerect' },
              { kind: 'class', label: 'Class', shape: 'square' },
              { kind: 'interface', label: 'Interface', shape: 'diamond' },
              { kind: 'enum', label: 'Enum', shape: 'octagon' },
              { kind: 'function', label: 'Function', shape: 'circle' },
            ].map(({ kind, label, shape }) => (
              <div key={kind} className={styles.legendEntry}>
                <svg width="18" height="18" viewBox="-10 -10 20 20">
                  {shape === 'circle' && (
                    <circle r="7" fill={nodeColor(kind)} fillOpacity="0.8" stroke={nodeColor(kind)} strokeWidth="1" strokeOpacity="0.6" />
                  )}
                  {shape === 'rrect' && (
                    <rect x="-9" y="-6" width="18" height="12" rx="3" fill={nodeColor(kind)} fillOpacity="0.18" stroke={nodeColor(kind)} strokeWidth="1.2" strokeOpacity="0.7" />
                  )}
                  {shape === 'hex' && (
                    <path d="M 0 -8 L 7 -4 L 7 4 L 0 8 L -7 4 L -7 -4 Z" fill={nodeColor(kind)} fillOpacity="0.18" stroke={nodeColor(kind)} strokeWidth="1.2" strokeOpacity="0.7" />
                  )}
                  {shape === 'filerect' && (
                    <path d="M -8 -7 L 5 -7 L 8 -4 L 8 7 L -8 7 Z" fill={nodeColor(kind)} fillOpacity="0.18" stroke={nodeColor(kind)} strokeWidth="1.2" strokeOpacity="0.7" />
                  )}
                  {shape === 'square' && (
                    <rect x="-7" y="-7" width="14" height="14" rx="2" fill={nodeColor(kind)} fillOpacity="0.18" stroke={nodeColor(kind)} strokeWidth="1.2" strokeOpacity="0.7" />
                  )}
                  {shape === 'diamond' && (
                    <path d="M 0 -8 L 8 0 L 0 8 L -8 0 Z" fill={nodeColor(kind)} fillOpacity="0.18" stroke={nodeColor(kind)} strokeWidth="1.2" strokeOpacity="0.7" />
                  )}
                  {shape === 'octagon' && (
                    <path d="M -3 -7 L 3 -7 L 7 -3 L 7 3 L 3 7 L -3 7 L -7 3 L -7 -3 Z" fill={nodeColor(kind)} fillOpacity="0.18" stroke={nodeColor(kind)} strokeWidth="1.2" strokeOpacity="0.7" />
                  )}
                </svg>
                <span className={styles.legendText}>{label}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Side panel */}
        <div className={styles.panel}>
          <div className={styles.panelTitle}>Node Details</div>
          {renderPanel()}
        </div>
      </div>

      {/* Hint bar */}
      <div className={styles.hintBar}>
        <span>Click node to inspect</span>
        <span className={styles.hintSep}>·</span>
        <span>Double-click to drill down</span>
        <span className={styles.hintSep}>·</span>
        <span>Scroll to zoom</span>
      </div>

      {/* Code preview modal */}
      {codeModal && (
        <CodeModal
          filePath={codeModal.filePath}
          content={codeModal.content}
          loading={codeModal.loading}
          error={codeModal.error}
          onClose={() => setCodeModal(null)}
        />
      )}
    </div>
  )
}
