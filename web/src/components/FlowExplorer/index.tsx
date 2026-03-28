import { useState, useEffect, useRef, useCallback } from 'react'
import * as d3 from 'd3'
import {
  sankey as d3Sankey,
  sankeyLinkHorizontal,
  sankeyLeft,
  type SankeyNode,
  type SankeyLink,
} from 'd3-sankey'
import { api } from '../../api'
import type { TraceNode, FullTraceResult } from '../../types/api.types'
import styles from './FlowExplorer.module.css'

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface FlowExplorerProps {
  workspacePath: string
  onFileNavigate?: (file: string, line?: number) => void
}

// ---------------------------------------------------------------------------
// Layer definitions
// ---------------------------------------------------------------------------

// Node colors by symbol kind — no layer guessing
const KIND_COLORS: Record<string, string> = {
  class:       '#5b8dd9',
  struct:      '#5b8dd9',
  interface:   '#c06ad0',
  method:      '#3faa8c',
  function:    '#3faa8c',
  constructor: '#e07c3d',
  property:    '#8a8a8a',
  field:       '#8a8a8a',
  enum:        '#d4a740',
  test:        '#7a9a6a',
  namespace:   '#6a7a9a',
}

function kindColor(kind: string): string {
  return KIND_COLORS[kind.toLowerCase()] ?? '#8a9aaa'
}

// ---------------------------------------------------------------------------
// Internal graph types for sankey
// ---------------------------------------------------------------------------

interface SNode {
  id: string
  label: string
  qualifiedName: string
  kind: string
  file: string
  line: number
  depth: number
}

interface SLink {
  sourceId: string
  targetId: string
  value: number
  edgeKind: string
}

// d3-sankey augmented types
type LayoutNode = SankeyNode<SNode, SLink>
type LayoutLink = SankeyLink<SNode, SLink>

// ---------------------------------------------------------------------------
// Data transform — FullTraceResult → sankey nodes + links
// ---------------------------------------------------------------------------

const MAX_NODES = 60

function flattenTrace(
  node: TraceNode,
  nodeMap: Map<string, SNode>,
  linkCountMap: Map<string, { value: number; edgeKind: string }>,
  parentId: string | null,
) {
  const id = node.qualified_name

  if (!nodeMap.has(id)) {
    const parts = node.name.split('.')
    const label = parts.slice(-2).join('.')
    nodeMap.set(id, {
      id,
      label,
      qualifiedName: node.qualified_name,
      kind: node.kind,
      file: node.file_path,
      line: node.line,
      depth: node.depth,
    })
  }

  if (parentId !== null) {
    const key = `${parentId}→${id}`
    const existing = linkCountMap.get(key)
    if (existing) {
      existing.value += 1
    } else {
      linkCountMap.set(key, { value: 1, edgeKind: node.edge_kind })
    }
  }

  for (const child of node.children) {
    flattenTrace(child, nodeMap, linkCountMap, id)
  }
}

function buildSankeyData(result: FullTraceResult): {
  nodes: SNode[]
  links: SLink[]
} {
  const nodeMap = new Map<string, SNode>()
  const linkCountMap = new Map<string, { value: number; edgeKind: string }>()

  for (const trace of result.traces) {
    flattenTrace(trace.entry, nodeMap, linkCountMap, null)
  }

  // Compute degree for capping
  const degree = new Map<string, number>()
  for (const [key] of linkCountMap) {
    const [src, tgt] = key.split('→')
    degree.set(src, (degree.get(src) ?? 0) + 1)
    degree.set(tgt, (degree.get(tgt) ?? 0) + 1)
  }

  // Cap to MAX_NODES by degree
  let nodes = Array.from(nodeMap.values())
  if (nodes.length > MAX_NODES) {
    nodes.sort((a, b) => (degree.get(b.id) ?? 0) - (degree.get(a.id) ?? 0))
    nodes = nodes.slice(0, MAX_NODES)
  }
  const keptIds = new Set(nodes.map(n => n.id))

  // Build final link list — only kept nodes, no self-loops, no backward-layer links
  const links: SLink[] = []
  for (const [key, { value, edgeKind }] of linkCountMap) {
    const sep = key.indexOf('→')
    const srcId = key.slice(0, sep)
    const tgtId = key.slice(sep + 1)
    if (!keptIds.has(srcId) || !keptIds.has(tgtId)) continue
    if (srcId === tgtId) continue

    const srcNode = nodeMap.get(srcId)!
    const tgtNode = nodeMap.get(tgtId)!
    // Skip backward links (higher depth → lower depth) to keep sankey acyclic
    if (srcNode.depth >= tgtNode.depth) continue

    links.push({ sourceId: srcId, targetId: tgtId, value, edgeKind })
  }

  // Remove orphan nodes (no links after filtering)
  const linkedIds = new Set<string>()
  for (const l of links) {
    linkedIds.add(l.sourceId)
    linkedIds.add(l.targetId)
  }
  const finalNodes = nodes.filter(n => linkedIds.has(n.id))

  return { nodes: finalNodes, links }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function FlowExplorer({ workspacePath, onFileNavigate }: FlowExplorerProps) {
  const [depth, setDepth] = useState(4)
  const [symbolFilter, setSymbolFilter] = useState('')
  const [activeSymbol, setActiveSymbol] = useState<string | undefined>(undefined)
  const [result, setResult] = useState<FullTraceResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [loadKey, setLoadKey] = useState(0)

  const svgRef = useRef<SVGSVGElement>(null)
  const chartRef = useRef<HTMLDivElement>(null)
  const tooltipRef = useRef<HTMLDivElement>(null)

  // Fetch
  useEffect(() => {
    if (!workspacePath) return
    setLoading(true)
    setError(null)
    api
      .fullTrace(workspacePath, activeSymbol, depth, 15)
      .then(r => {
        setResult(r)
        setLoading(false)
      })
      .catch(e => {
        setError(String(e?.message ?? e))
        setLoading(false)
      })
  }, [workspacePath, depth, activeSymbol, loadKey])

  const handleSymbolSearch = useCallback(() => {
    const trimmed = symbolFilter.trim()
    setActiveSymbol(trimmed || undefined)
  }, [symbolFilter])

  // D3 Sankey render
  useEffect(() => {
    if (!result || !svgRef.current || !chartRef.current) return

    const { nodes: rawNodes, links: rawLinks } = buildSankeyData(result)
    if (rawNodes.length === 0 || rawLinks.length === 0) return

    const container = chartRef.current
    const W = container.clientWidth || 900
    const H = container.clientHeight || 520

    const margin = { top: 36, right: 160, bottom: 24, left: 160 }

    const svg = d3.select(svgRef.current)
    svg.selectAll('*').remove()
    svg.attr('width', W).attr('height', H)

    const tooltip = tooltipRef.current

    // ── Build index-mapped sankey input ───────────────────────────────────
    // d3-sankey requires source/target to be node objects (after layout) or indices.
    // We pass objects with an `index` field; sankey uses nodeId to resolve them.

    const sankeyNodes: SNode[] = rawNodes.map(n => ({ ...n }))
    const nodeIdSet = new Set(sankeyNodes.map(n => n.id))

    const sankeyLinks = rawLinks
      .filter(l => nodeIdSet.has(l.sourceId) && nodeIdSet.has(l.targetId))
      .map(l => ({
        source: l.sourceId as any,
        target: l.targetId as any,
        value: l.value,
        edgeKind: l.edgeKind,
        sourceId: l.sourceId,
        targetId: l.targetId,
      }))

    // ── Sankey layout ─────────────────────────────────────────────────────

    const layout = d3Sankey<SNode, SLink>()
      .nodeId((d: SNode) => d.id)
      .nodeWidth(18)
      .nodePadding(14)
      .nodeAlign(sankeyLeft)
      .extent([
        [margin.left, margin.top],
        [W - margin.right, H - margin.bottom],
      ])
      .nodeSort(null)

    const graph = layout({
      nodes: sankeyNodes.map(d => ({ ...d })),
      links: sankeyLinks.map(d => ({ ...d })),
    })

    const layoutNodes = graph.nodes as LayoutNode[]
    const layoutLinks = graph.links as LayoutLink[]

    // Give each link a stable index for path-tracing
    layoutLinks.forEach((l, i) => {
      ;(l as LayoutLink & { index: number }).index = i
    })

    // Color nodes by symbol kind — no layer guessing
    const columnXs = [...new Set(layoutNodes.map(n => n.x0!))].sort((a, b) => a - b)

    function nodeColor(n: LayoutNode): string {
      return kindColor(n.kind)
    }

    // ── Defs — linear gradients per link ──────────────────────────────────

    const defs = svg.append('defs')

    layoutLinks.forEach((l, i) => {
      const srcNode = l.source as LayoutNode
      const tgtNode = l.target as LayoutNode
      const srcColor = nodeColor(srcNode)
      const tgtColor = nodeColor(tgtNode)

      const grad = defs
        .append('linearGradient')
        .attr('id', `sk-grad-${i}`)
        .attr('gradientUnits', 'userSpaceOnUse')
        .attr('x1', srcNode.x1!)
        .attr('x2', tgtNode.x0!)

      grad.append('stop').attr('offset', '0%').attr('stop-color', srcColor)
      grad.append('stop').attr('offset', '100%').attr('stop-color', tgtColor)
    })

    // ── Column guide lines ────────────────────────────────────────────────

    svg
      .append('g')
      .attr('class', 'sk-guides')
      .selectAll('line')
      .data(columnXs)
      .join('line')
      .attr('x1', d => d + 9)
      .attr('x2', d => d + 9)
      .attr('y1', margin.top)
      .attr('y2', H - margin.bottom)
      .attr('stroke', '#2a2a2a')
      .attr('stroke-width', 1)
      .attr('stroke-dasharray', '3,5')

    // ── Column headers ────────────────────────────────────────────────────

    const headerData = columnXs.map((x, i) => ({
      x,
      name: `Depth ${i}`,
      color: '#666',
    }))

    svg
      .append('g')
      .attr('class', 'sk-headers')
      .selectAll('text')
      .data(headerData)
      .join('text')
      .attr('x', d => d.x + 9)
      .attr('y', margin.top - 12)
      .attr('text-anchor', 'middle')
      .attr('fill', d => d.color)
      .attr('font-size', 11)
      .attr('font-weight', 700)
      .attr('letter-spacing', '0.07em')
      .attr('font-family', "'DM Sans', 'Segoe UI', system-ui, sans-serif")
      .text(d => d.name.toUpperCase())

    // ── Links ─────────────────────────────────────────────────────────────

    const linkG = svg.append('g').attr('class', 'sk-links')

    const linkPath = linkG
      .selectAll<SVGPathElement, LayoutLink>('.sk-link')
      .data(layoutLinks)
      .join('path')
      .attr('class', 'sk-link')
      .attr('d', sankeyLinkHorizontal())
      .attr('fill', 'none')
      .attr('stroke', (_, i) => `url(#sk-grad-${i})`)
      .attr('stroke-width', d => Math.max(1, d.width ?? 1))
      .attr('opacity', 0.28)
      .style('transition', 'opacity 0.18s ease')
      .style('cursor', 'pointer')

    // ── Nodes ─────────────────────────────────────────────────────────────

    const nodeG = svg.append('g').attr('class', 'sk-nodes')

    const nodeEl = nodeG
      .selectAll<SVGGElement, LayoutNode>('.sk-node')
      .data(layoutNodes)
      .join('g')
      .attr('class', 'sk-node')
      .attr('transform', d => `translate(${d.x0},${d.y0})`)
      .style('cursor', 'pointer')

    nodeEl
      .append('rect')
      .attr('width', d => d.x1! - d.x0!)
      .attr('height', d => Math.max(1, d.y1! - d.y0!))
      .attr('rx', 3)
      .attr('fill', d => nodeColor(d))
      .attr('stroke', '#1a1a1a')
      .attr('stroke-width', 1.5)
      .style('transition', 'opacity 0.18s ease')

    // Node labels — left side → label right, right side → label left
    nodeEl
      .append('text')
      .attr('class', 'sk-node-label')
      .attr('x', d => {
        const isRightHalf = d.x1! > W / 2
        return isRightHalf ? -(d.x1! - d.x0!) - 8 : (d.x1! - d.x0!) + 8
      })
      .attr('y', d => (d.y1! - d.y0!) / 2)
      .attr('text-anchor', d => (d.x1! > W / 2 ? 'end' : 'start'))
      .attr('dominant-baseline', 'central')
      .attr('fill', '#d8d8d8')
      .attr('font-size', 11)
      .attr('font-weight', 500)
      .attr('font-family', "'DM Sans', 'Segoe UI', system-ui, sans-serif")
      .attr('pointer-events', 'none')
      .text(d => {
        const label = (d as unknown as SNode).label
        return label.length > 22 ? label.slice(0, 20) + '…' : label
      })

    // Conn count below label
    nodeEl
      .append('text')
      .attr('class', 'sk-node-count')
      .attr('x', d => {
        const isRightHalf = d.x1! > W / 2
        return isRightHalf ? -(d.x1! - d.x0!) - 8 : (d.x1! - d.x0!) + 8
      })
      .attr('y', d => (d.y1! - d.y0!) / 2 + 14)
      .attr('text-anchor', d => (d.x1! > W / 2 ? 'end' : 'start'))
      .attr('dominant-baseline', 'central')
      .attr('fill', '#666')
      .attr('font-size', 10)
      .attr('font-family', "'DM Sans', 'Segoe UI', system-ui, sans-serif")
      .attr('pointer-events', 'none')
      .text(d => `${d.value ?? 0} conn.`)

    // ── Path-tracing helpers (same logic as demo) ─────────────────────────

    type IndexedLink = LayoutLink & { index: number }

    function ancestorLinks(node: LayoutNode, visited = new Set<number>()): Set<number> {
      const result = new Set<number>()
      for (const link of node.targetLinks ?? []) {
        const l = link as IndexedLink
        if (visited.has(l.index)) continue
        visited.add(l.index)
        result.add(l.index)
        for (const id of ancestorLinks(l.source as LayoutNode, visited)) result.add(id)
      }
      return result
    }

    function descendantLinks(node: LayoutNode, visited = new Set<number>()): Set<number> {
      const result = new Set<number>()
      for (const link of node.sourceLinks ?? []) {
        const l = link as IndexedLink
        if (visited.has(l.index)) continue
        visited.add(l.index)
        result.add(l.index)
        for (const id of descendantLinks(l.target as LayoutNode, visited)) result.add(id)
      }
      return result
    }

    function getPathLinkIds(link: IndexedLink): Set<number> {
      const ids = new Set<number>([link.index])
      for (const id of ancestorLinks(link.source as LayoutNode)) ids.add(id)
      for (const id of descendantLinks(link.target as LayoutNode)) ids.add(id)
      return ids
    }

    function getPathNodeIndices(linkIds: Set<number>): Set<number> {
      const indices = new Set<number>()
      for (const id of linkIds) {
        const l = layoutLinks[id]
        indices.add((l.source as LayoutNode).index!)
        indices.add((l.target as LayoutNode).index!)
      }
      return indices
    }

    // ── Tooltip helpers ───────────────────────────────────────────────────

    function showNodeTooltip(event: MouseEvent, d: LayoutNode) {
      if (!tooltip) return
      const snode = d as unknown as SNode & LayoutNode
      const shortFile = snode.file.split('/').slice(-3).join('/')

      tooltip.innerHTML = `
        <div class="${styles.ttName}">${snode.qualifiedName}</div>
        <div class="${styles.ttKind}">${snode.kind}</div>
        <div class="${styles.ttFile}">${shortFile}:${snode.line}</div>
      `
      tooltip.classList.add(styles.visible)
      positionTooltip(event)
    }

    function showLinkTooltip(event: MouseEvent, d: LayoutLink) {
      if (!tooltip) return
      const src = d.source as LayoutNode & SNode
      const tgt = d.target as LayoutNode & SNode
      tooltip.innerHTML = `
        <div class="${styles.ttSource}">${src.label}</div>
        <span class="${styles.ttArrow}">→</span>
        <div class="${styles.ttTarget}">${tgt.label}</div>
        <div class="${styles.ttConns}">connections: <span class="${styles.ttCount}">${d.value}</span></div>
      `
      tooltip.classList.add(styles.visible)
      positionTooltip(event)
    }

    function positionTooltip(event: MouseEvent) {
      if (!tooltip || !container) return
      const rect = container.getBoundingClientRect()
      const TW = 250
      const TH = 90
      let x = event.clientX - rect.left + 14
      let y = event.clientY - rect.top + 10
      if (x + TW > W) x = event.clientX - rect.left - TW - 14
      if (y + TH > H) y = event.clientY - rect.top - TH - 10
      tooltip.style.left = `${x}px`
      tooltip.style.top = `${y}px`
    }

    function hideTooltip() {
      tooltip?.classList.remove(styles.visible)
    }

    // ── Pinned highlight state ──────────────────────────────────────────────

    let pinnedLinkIds: Set<number> | null = null
    let pinnedNodeIndices: Set<number> | null = null

    // Apply highlight: merge pinned + hover sets. Pinned at full, hover at medium, rest dimmed.
    function applyHighlight(
      activeLinks: Set<number>,
      activeNodes: Set<number>,
      secondaryLinks?: Set<number>,
      secondaryNodes?: Set<number>,
    ) {
      linkPath.attr('opacity', (l: LayoutLink) => {
        const idx = (l as IndexedLink).index
        if (activeLinks.has(idx)) return 0.82
        if (secondaryLinks?.has(idx)) return 0.45
        return 0.05
      })
      nodeEl.select('rect').attr('opacity', (n: LayoutNode) => {
        if (activeNodes.has(n.index!)) return 1
        if (secondaryNodes?.has(n.index!)) return 0.7
        return 0.15
      })
      nodeEl.each(function (n: LayoutNode) {
        let op = 0.15
        if (activeNodes.has(n.index!)) op = 1
        else if (secondaryNodes?.has(n.index!)) op = 0.7
        d3.select(this).selectAll('text').attr('opacity', op)
      })
    }

    function applyPinnedOnly() {
      if (pinnedLinkIds && pinnedNodeIndices) {
        applyHighlight(pinnedLinkIds, pinnedNodeIndices)
      }
    }

    function clearAll() {
      pinnedLinkIds = null
      pinnedNodeIndices = null
      linkPath.attr('opacity', 0.28)
      nodeEl.select('rect').attr('opacity', 1)
      nodeEl.selectAll('text').attr('opacity', 1 as any)
    }

    // Click SVG background to unpin
    svg.on('click', (event: MouseEvent) => {
      if (event.target === svgRef.current) {
        clearAll()
        hideTooltip()
      }
    })

    // ── Link interactions ────────────────────────────────────────────────

    linkPath
      .on('mouseenter', function (event: MouseEvent, d: LayoutLink) {
        const pathIds = getPathLinkIds(d as IndexedLink)
        const pathNodeIndices = getPathNodeIndices(pathIds)
        if (pinnedLinkIds) {
          // Show hover as secondary, pinned stays primary
          applyHighlight(pinnedLinkIds, pinnedNodeIndices!, pathIds, pathNodeIndices)
        } else {
          applyHighlight(pathIds, pathNodeIndices)
        }
        showLinkTooltip(event, d)
      })
      .on('mousemove', (event: MouseEvent) => positionTooltip(event))
      .on('mouseleave', () => {
        if (pinnedLinkIds) {
          applyPinnedOnly()
        } else {
          clearAll()
        }
        hideTooltip()
      })
      .on('click', function (event: MouseEvent, d: LayoutLink) {
        event.stopPropagation()
        const pathIds = getPathLinkIds(d as IndexedLink)
        const pathNodeIndices = getPathNodeIndices(pathIds)

        // Toggle: if clicking same path, unpin
        if (pinnedLinkIds && pathIds.size === pinnedLinkIds.size &&
            [...pathIds].every(id => pinnedLinkIds!.has(id))) {
          clearAll()
          hideTooltip()
          return
        }

        pinnedLinkIds = pathIds
        pinnedNodeIndices = pathNodeIndices
        applyHighlight(pathIds, pathNodeIndices)
        showLinkTooltip(event, d)
      })

    // ── Node interactions ────────────────────────────────────────────────

    nodeEl
      .on('mouseenter', function (event: MouseEvent, d: LayoutNode) {
        const connectedLinkIds = new Set<number>()
        for (const l of [...(d.sourceLinks ?? []), ...(d.targetLinks ?? [])]) {
          connectedLinkIds.add((l as IndexedLink).index)
        }
        const connectedNodeIndices = getPathNodeIndices(connectedLinkIds)
        connectedNodeIndices.add(d.index!)
        if (pinnedLinkIds) {
          applyHighlight(pinnedLinkIds, pinnedNodeIndices!, connectedLinkIds, connectedNodeIndices)
        } else {
          applyHighlight(connectedLinkIds, connectedNodeIndices)
        }
        showNodeTooltip(event, d)
      })
      .on('mousemove', (event: MouseEvent) => positionTooltip(event))
      .on('mouseleave', () => {
        if (pinnedLinkIds) {
          applyPinnedOnly()
        } else {
          clearAll()
        }
        hideTooltip()
      })
      .on('click', (event: MouseEvent, d: LayoutNode) => {
        event.stopPropagation()
        const snode = d as unknown as SNode & LayoutNode

        // Build full path through this node (ancestors + descendants)
        const linkIds = new Set<number>()
        for (const id of ancestorLinks(d)) linkIds.add(id)
        for (const id of descendantLinks(d)) linkIds.add(id)
        // Also add direct connections
        for (const l of [...(d.sourceLinks ?? []), ...(d.targetLinks ?? [])]) {
          linkIds.add((l as IndexedLink).index)
        }
        const nodeIndices = getPathNodeIndices(linkIds)
        nodeIndices.add(d.index!)

        // Toggle: if clicking same node, unpin
        if (pinnedNodeIndices && pinnedNodeIndices.has(d.index!) && pinnedNodeIndices.size <= nodeIndices.size + 1) {
          clearAll()
          hideTooltip()
          return
        }

        pinnedLinkIds = linkIds
        pinnedNodeIndices = nodeIndices
        applyHighlight(linkIds, nodeIndices)
        showNodeTooltip(event, d)

        // Also navigate to file
        if (onFileNavigate && snode.file) {
          onFileNavigate(snode.file, snode.line)
        }
      })

    // ── Resize observer ────────────────────────────────────────────────────

    const ro = new ResizeObserver(() => {
      // Re-trigger the effect by nudging state would cause infinite loop.
      // Just re-run the layout on the same SVG element by re-sizing it.
      const newW = container.clientWidth
      const newH = container.clientHeight
      svg.attr('width', newW).attr('height', newH)
    })
    ro.observe(container)

    return () => {
      ro.disconnect()
      svg.selectAll('*').remove()
    }
  }, [result, onFileNavigate])

  // ---------------------------------------------------------------------------
  // Derived counts
  // ---------------------------------------------------------------------------

  const handleReload = useCallback(() => {
    setLoadKey(k => k + 1)
  }, [])

  const traceCount = result?.traces.length ?? 0
  const totalSymbols = result?.total_symbols ?? 0
  const flowJumps = result?.flow_jumps ?? 0

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  if (loading && !result) {
    return (
      <div className={styles.container}>
        <SummaryBar
          traceCount={0}
          symbolCount={0}
          flowJumps={0}
          depth={depth}
          onDepthChange={setDepth}
          onReload={handleReload}
          loading
          symbolFilter={symbolFilter}
          onSymbolFilterChange={setSymbolFilter}
          onSymbolSearch={handleSymbolSearch}
        />
        <div className={styles.stateWrapper}>
          <div className={styles.spinner} />
          <div className={styles.stateTitle}>Loading Sankey flow…</div>
          <div className={styles.stateSubtitle}>Tracing call chains across the architecture</div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className={styles.container}>
        <SummaryBar
          traceCount={0}
          symbolCount={0}
          flowJumps={0}
          depth={depth}
          onDepthChange={setDepth}
          onReload={handleReload}
          loading={false}
          symbolFilter={symbolFilter}
          onSymbolFilterChange={setSymbolFilter}
          onSymbolSearch={handleSymbolSearch}
        />
        <div className={styles.stateWrapper}>
          <div className={styles.errorTitle}>Failed to load flow data</div>
          <div className={styles.stateSubtitle}>{error}</div>
        </div>
      </div>
    )
  }

  return (
    <div className={styles.container}>
      <SummaryBar
        traceCount={traceCount}
        symbolCount={totalSymbols}
        flowJumps={flowJumps}
        depth={depth}
        onDepthChange={setDepth}
        onReload={handleReload}
        loading={loading}
        symbolFilter={symbolFilter}
        onSymbolFilterChange={setSymbolFilter}
        onSymbolSearch={handleSymbolSearch}
      />

      <KindLegend />

      {!result || traceCount === 0 ? (
        <div className={styles.stateWrapper}>
          <div className={styles.stateTitle}>No flow data</div>
          <div className={styles.stateSubtitle}>
            Index this project to generate call-flow traces.
          </div>
        </div>
      ) : (
        <div className={styles.chartArea} ref={chartRef}>
          <svg
            ref={svgRef}
            className={styles.graphSvg}
            role="img"
            aria-label="Sankey architecture flow diagram"
          />
          <div className={styles.tooltip} ref={tooltipRef} />
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface SummaryBarProps {
  traceCount: number
  symbolCount: number
  flowJumps: number
  depth: number
  onDepthChange: (v: number) => void
  onReload: () => void
  loading: boolean
  symbolFilter: string
  onSymbolFilterChange: (v: string) => void
  onSymbolSearch: () => void
}

function SummaryBar({
  traceCount,
  symbolCount,
  flowJumps,
  depth,
  onDepthChange,
  onReload,
  loading,
  symbolFilter,
  onSymbolFilterChange,
  onSymbolSearch,
}: SummaryBarProps) {
  return (
    <>
      <div className={styles.summaryBar}>
        <span className={styles.summaryLabel}>Traces</span>
        <span className={styles.summaryTotal}>{traceCount}</span>
        <span className={styles.summaryLabel} style={{ marginLeft: 8 }}>
          Symbols
        </span>
        <span className={styles.summaryTotal}>{symbolCount}</span>
        <span className={styles.summaryLabel} style={{ marginLeft: 8 }}>
          Flow Jumps
        </span>
        <span className={styles.summaryTotal}>{flowJumps}</span>
      </div>
      <div className={styles.filterBar}>
        <input
          type="text"
          className={styles.symbolInput}
          placeholder="Trace from symbol… (empty = all entry points)"
          value={symbolFilter}
          onChange={e => onSymbolFilterChange(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter') onSymbolSearch() }}
        />
        <button className={styles.reloadBtn} onClick={onSymbolSearch}>
          Trace
        </button>
        <span className={styles.depthLabel} style={{ marginLeft: 12 }}>Depth</span>
        <input
          type="range"
          className={styles.depthSlider}
          min={1}
          max={6}
          value={depth}
          onChange={e => onDepthChange(Number(e.target.value))}
        />
        <span className={styles.depthValue}>{depth}</span>
        <button className={styles.reloadBtn} onClick={onReload} disabled={loading}>
          {loading ? 'Loading…' : 'Reload'}
        </button>
      </div>
    </>
  )
}

const KIND_LEGEND = ['class', 'method', 'interface', 'constructor', 'property', 'enum'] as const

function KindLegend() {
  return (
    <div className={styles.layerLegend}>
      {KIND_LEGEND.map(kind => (
        <div key={kind} className={styles.layerLegendItem}>
          <div
            className={styles.layerLegendSwatch}
            style={{ background: kindColor(kind) }}
          />
          <span>{kind}</span>
        </div>
      ))}
    </div>
  )
}
