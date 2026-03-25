import * as d3 from 'd3'
import { useEffect, useRef } from 'react'
import { useGraphStore } from '../stores/graph.store'
import { useSelectionStore } from '../stores/selection.store'
import type { D3Node, D3Edge, TooltipState } from '../types/graph.types'

export function useD3Simulation(
  svgRef: React.RefObject<SVGSVGElement | null>,
  containerRef: React.RefObject<HTMLDivElement | null>,
  setTooltip: React.Dispatch<React.SetStateAction<TooltipState>>,
) {
  const simulationRef = useRef<d3.Simulation<D3Node, D3Edge> | null>(null)
  const zoomRef = useRef<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null)
  const renderGenRef = useRef(0)

  const nodes = useGraphStore((s) => s.nodes)
  const edges = useGraphStore((s) => s.edges)
  const loadState = useGraphStore((s) => s.loadState)
  const activeConcept = useGraphStore((s) => s.activeConcept)
  const selectNode = useSelectionStore((s) => s.selectNode)

  // D3 force simulation
  useEffect(() => {
    if (loadState !== 'ready') return
    if (!svgRef.current || !containerRef.current) return
    if (nodes.length === 0) return

    const gen = ++renderGenRef.current
    const thisSvg = svgRef.current
    const container = containerRef.current

    const width = container.clientWidth || 800
    const height = container.clientHeight || 600

    d3.select(thisSvg).selectAll('*').remove()
    simulationRef.current?.stop()

    const svgSel = d3
      .select<SVGSVGElement, unknown>(thisSvg)
      .attr('width', width)
      .attr('height', height)

    svgSel
      .append('defs')
      .append('marker')
      .attr('id', 'kt-arrow')
      .attr('viewBox', '0 -4 8 8')
      .attr('refX', 10)
      .attr('refY', 0)
      .attr('markerWidth', 6)
      .attr('markerHeight', 6)
      .attr('orient', 'auto')
      .append('path')
      .attr('d', 'M 0 -4 L 8 0 L 0 4')
      .attr('fill', '#484f58')

    const zoomGroup = svgSel.append('g').attr('class', 'kt-zoom-group')
    const linksGroup = zoomGroup.append('g').attr('class', 'kt-links')
    const nodesGroup = zoomGroup.append('g').attr('class', 'kt-nodes')

    const zoom = d3
      .zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.1, 8])
      .on('zoom', (event: d3.D3ZoomEvent<SVGSVGElement, unknown>) => {
        zoomGroup.attr('transform', String(event.transform))
      })
    zoomRef.current = zoom
    svgSel.call(zoom)

    const uniqueConcepts = [
      ...new Set(nodes.map((n) => n.concept).filter((c): c is string => c !== null)),
    ]
    const conceptCentroids = new Map<string, { x: number; y: number }>()
    uniqueConcepts.forEach((name, i) => {
      const angle = (2 * Math.PI * i) / uniqueConcepts.length
      const r = Math.min(width, height) * 0.25
      conceptCentroids.set(name, {
        x: width / 2 + r * Math.cos(angle),
        y: height / 2 + r * Math.sin(angle),
      })
    })

    const visibleNodes = activeConcept
      ? nodes.filter((n) => n.concept === activeConcept)
      : nodes
    const visibleNodeIds = new Set(visibleNodes.map((n) => n.id))
    const visibleEdges = edges.filter((e) => {
      const src = typeof e.source === 'object' ? (e.source as D3Node).id : (e.source as number)
      const tgt = typeof e.target === 'object' ? (e.target as D3Node).id : (e.target as number)
      return visibleNodeIds.has(src) && visibleNodeIds.has(tgt)
    })

    const linkSel = linksGroup
      .selectAll<SVGLineElement, D3Edge>('line')
      .data(visibleEdges)
      .join('line')
      .attr('class', (d) => `kt-link kt-link--${d.kind}`)
      .attr('marker-end', (d) => (d.kind === 'calls' ? 'url(#kt-arrow)' : null))

    const nodeSel = nodesGroup
      .selectAll<SVGGElement, D3Node>('g.kt-node')
      .data(visibleNodes, (d) => String(d.id))
      .join('g')
      .attr('class', 'kt-node')

    nodeSel.each(function (d: D3Node) {
      const g = d3.select(this)
      const k = d.kind.toLowerCase()

      if (k === 'interface') {
        g.append('path')
          .attr('d', `M 0 ${-d.radius} L ${d.radius} 0 L 0 ${d.radius} L ${-d.radius} 0 Z`)
          .attr('fill', d.color)
          .attr('fill-opacity', 0.85)
          .attr('stroke', d.color)
          .attr('stroke-width', 1)
      } else if (k === 'enum') {
        const pts: string[] = []
        for (let i = 0; i < 6; i++) {
          const angle = (Math.PI / 3) * i - Math.PI / 2
          pts.push(`${d.radius * Math.cos(angle)},${d.radius * Math.sin(angle)}`)
        }
        g.append('path')
          .attr('d', `M ${pts.join(' L ')} Z`)
          .attr('fill', d.color)
          .attr('fill-opacity', 0.85)
          .attr('stroke', d.color)
          .attr('stroke-width', 1)
      } else {
        g.append('circle')
          .attr('r', d.radius)
          .attr('fill', d.color)
          .attr('fill-opacity', 0.85)
          .attr('stroke', d.color)
          .attr('stroke-width', 1)
      }
    })

    nodeSel
      .append('text')
      .attr('class', 'kt-node-label')
      .attr('dy', (d) => d.radius + 12)
      .attr('text-anchor', 'middle')
      .text((d) => (d.radius >= 8 ? d.name : ''))

    const drag = d3
      .drag<SVGGElement, D3Node>()
      .on('start', (event: d3.D3DragEvent<SVGGElement, D3Node, D3Node>, d: D3Node) => {
        if (!event.active) simulationRef.current?.alphaTarget(0.3).restart()
        d.fx = d.x
        d.fy = d.y
      })
      .on('drag', (event: d3.D3DragEvent<SVGGElement, D3Node, D3Node>, d: D3Node) => {
        d.fx = event.x
        d.fy = event.y
      })
      .on('end', (event: d3.D3DragEvent<SVGGElement, D3Node, D3Node>, d: D3Node) => {
        if (!event.active) simulationRef.current?.alphaTarget(0)
        d.fx = null
        d.fy = null
      })

    nodeSel.call(drag)

    nodeSel
      .on('mousemove', (event: MouseEvent, d: D3Node) => {
        const rect = container.getBoundingClientRect()
        setTooltip({ visible: true, x: event.clientX - rect.left, y: event.clientY - rect.top, node: d })
        const connectedIds = new Set<number>([d.id])
        visibleEdges.forEach((e) => {
          const src = (e.source as D3Node).id
          const tgt = (e.target as D3Node).id
          if (src === d.id) connectedIds.add(tgt)
          if (tgt === d.id) connectedIds.add(src)
        })
        nodeSel.classed('kt-node--dimmed', (n) => !connectedIds.has(n.id))
        nodeSel.classed('kt-node--highlighted', (n) => connectedIds.has(n.id) && n.id !== d.id)
        nodeSel.selectAll<SVGTextElement, D3Node>('text').classed(
          'kt-node-label--dimmed',
          function () {
            const nd = d3.select(this.parentNode as SVGGElement).datum() as D3Node
            return !connectedIds.has(nd.id)
          },
        )
        linkSel.classed('kt-link--dimmed', (e) => {
          const src = (e.source as D3Node).id
          const tgt = (e.target as D3Node).id
          return src !== d.id && tgt !== d.id
        })
      })
      .on('mouseleave', () => {
        setTooltip((prev) => ({ ...prev, visible: false }))
        nodeSel.classed('kt-node--dimmed', false)
        nodeSel.classed('kt-node--highlighted', false)
        nodeSel.selectAll('text').classed('kt-node-label--dimmed', false)
        linkSel.classed('kt-link--dimmed', false)
      })
      .on('click', (_event: MouseEvent, d: D3Node) => {
        selectNode(d)
      })

    const simulation = d3
      .forceSimulation<D3Node>(visibleNodes)
      .force(
        'link',
        d3
          .forceLink<D3Node, D3Edge>(visibleEdges)
          .id((d) => String(d.id))
          .distance(80)
          .strength(0.3),
      )
      .force('charge', d3.forceManyBody<D3Node>().strength((d) => -(d.radius * 30)))
      .force('center', d3.forceCenter<D3Node>(width / 2, height / 2))
      .force('collision', d3.forceCollide<D3Node>().radius((d) => d.radius + 8))

    simulation.force('concept-cluster', () => {
      const alpha = simulation.alpha()
      for (const node of visibleNodes) {
        if (!node.concept) continue
        const centroid = conceptCentroids.get(node.concept)
        if (!centroid) continue
        node.vx = (node.vx ?? 0) + (centroid.x - (node.x ?? 0)) * alpha * 0.05
        node.vy = (node.vy ?? 0) + (centroid.y - (node.y ?? 0)) * alpha * 0.05
      }
    })

    simulationRef.current = simulation

    simulation.on('tick', () => {
      if (gen !== renderGenRef.current) {
        simulation.stop()
        return
      }

      linkSel
        .attr('x1', (d) => (d.source as D3Node).x ?? 0)
        .attr('y1', (d) => (d.source as D3Node).y ?? 0)
        .attr('x2', (d) => {
          const src = d.source as D3Node
          const tgt = d.target as D3Node
          const dx = (tgt.x ?? 0) - (src.x ?? 0)
          const dy = (tgt.y ?? 0) - (src.y ?? 0)
          const dist = Math.sqrt(dx * dx + dy * dy) || 1
          return (tgt.x ?? 0) - (dx / dist) * tgt.radius
        })
        .attr('y2', (d) => {
          const src = d.source as D3Node
          const tgt = d.target as D3Node
          const dx = (tgt.x ?? 0) - (src.x ?? 0)
          const dy = (tgt.y ?? 0) - (src.y ?? 0)
          const dist = Math.sqrt(dx * dx + dy * dy) || 1
          return (tgt.y ?? 0) - (dy / dist) * tgt.radius
        })

      nodeSel.attr('transform', (d) => `translate(${d.x ?? 0},${d.y ?? 0})`)
    })

    // Zoom to fit visible nodes after a short delay (don't wait for simulation end)
    if (activeConcept && visibleNodes.length > 0) {
      setTimeout(() => {
        if (gen !== renderGenRef.current) return
        let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity
        for (const n of visibleNodes) {
          const x = n.x ?? 0, y = n.y ?? 0
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
      }, 800)
    } else if (!activeConcept) {
      d3.select<SVGSVGElement, unknown>(thisSvg)
        .transition()
        .duration(500)
        .call(zoom.transform, d3.zoomIdentity)
    }

    return () => {
      simulation.stop()
    }
  }, [loadState, nodes, edges, activeConcept, setTooltip, selectNode, svgRef, containerRef])

  // Selection ring effect
  useEffect(() => {
    const selectedNode = useSelectionStore.getState().selectedNode
    if (!svgRef.current) return
    const svg = d3.select(svgRef.current)

    svg.selectAll('.kt-selection-ring').remove()

    if (!selectedNode || typeof selectedNode.x !== 'number') return

    const zoomGroup = svg.select('g')
    if (zoomGroup.empty()) return

    const ring = zoomGroup.append('g')
      .attr('class', 'kt-selection-ring')
      .attr('transform', `translate(${selectedNode.x},${selectedNode.y})`)

    ring.append('circle')
      .attr('r', selectedNode.radius + 8)
      .attr('fill', 'none')
      .attr('stroke', selectedNode.color)
      .attr('stroke-width', 2)
      .attr('stroke-opacity', 0.8)

    ring.append('circle')
      .attr('r', selectedNode.radius + 8)
      .attr('fill', 'none')
      .attr('stroke', selectedNode.color)
      .attr('stroke-width', 2)
      .attr('stroke-opacity', 0.6)
      .append('animate')
      .attr('attributeName', 'r')
      .attr('from', selectedNode.radius + 8)
      .attr('to', selectedNode.radius + 22)
      .attr('dur', '1.5s')
      .attr('repeatCount', 'indefinite')

    ring.select('circle:last-of-type')
      .append('animate')
      .attr('attributeName', 'stroke-opacity')
      .attr('from', '0.6')
      .attr('to', '0')
      .attr('dur', '1.5s')
      .attr('repeatCount', 'indefinite')

    const tickUpdate = () => {
      if (selectedNode && typeof selectedNode.x === 'number') {
        ring.attr('transform', `translate(${selectedNode.x},${selectedNode.y})`)
      }
    }
    if (simulationRef.current) {
      simulationRef.current.on('tick.selection', tickUpdate)
    }

    return () => {
      svg.selectAll('.kt-selection-ring').remove()
      if (simulationRef.current) {
        simulationRef.current.on('tick.selection', null)
      }
    }
  })

  // ResizeObserver
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
          .alpha(0.3)
          .restart()
      }
    })

    ro.observe(container)
    return () => ro.disconnect()
  }, [svgRef, containerRef])

  return { simulationRef, zoomRef }
}
