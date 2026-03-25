import { useCallback, useEffect } from 'react'
import { api } from '../api'
import { useGraphStore } from '../stores/graph.store'
import { useSelectionStore } from '../stores/selection.store'
import { toD3Node, toD3Edge } from '../utils/graph.utils'
import type { D3Node, D3Edge } from '../types/graph.types'

function buildGraph(graphData: { nodes: Parameters<typeof toD3Node>[0][]; edges: Parameters<typeof toD3Edge>[0][] }): { nodes: D3Node[]; edges: D3Edge[] } {
  const incomingCount = new Map<number, number>()
  for (const edge of graphData.edges) {
    incomingCount.set(edge.target_id, (incomingCount.get(edge.target_id) ?? 0) + 1)
  }
  const nodes: D3Node[] = graphData.nodes.map((n) =>
    toD3Node(n, incomingCount.get(n.id) ?? 0),
  )
  const nodeMap = new Map<number, D3Node>(nodes.map((n) => [n.id, n]))
  const edges: D3Edge[] = graphData.edges
    .map((e) => toD3Edge(e, nodeMap))
    .filter((e): e is D3Edge => e !== null)
  return { nodes, edges }
}

export function useGraphData(workspacePath: string) {
  const { setGraphData, setLoadState, setActiveConcept, nodes, edges } = useGraphStore()
  const clearSelection = useSelectionStore((s) => s.clearSelection)

  const loadData = useCallback(async () => {
    if (!workspacePath) return
    setLoadState('loading')

    try {
      const [graphData, conceptsData] = await Promise.all([
        api.graph(workspacePath),
        api.concepts(workspacePath),
      ])

      const { nodes, edges } = buildGraph(graphData)
      setGraphData(nodes, edges, conceptsData)
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      setLoadState('error', msg)
    }
  }, [workspacePath, setLoadState, setGraphData])

  useEffect(() => {
    if (workspacePath) {
      void loadData()
    }
  }, [workspacePath, loadData])

  const handleConceptClick = useCallback(async (name: string) => {
    const currentConcept = useGraphStore.getState().activeConcept
    const toggled = currentConcept === name ? null : name
    setActiveConcept(toggled)
    clearSelection()

    if (!workspacePath) return

    try {
      if (toggled) {
        const graphData = await api.graph(workspacePath, `@${toggled}`, 2000)
        const { nodes, edges } = buildGraph(graphData)
        setGraphData(nodes, edges, useGraphStore.getState().concepts)
      } else {
        const graphData = await api.graph(workspacePath)
        const { nodes, edges } = buildGraph(graphData)
        setGraphData(nodes, edges, useGraphStore.getState().concepts)
      }
    } catch {
      // Fall back to existing data
    }
  }, [workspacePath, setActiveConcept, setGraphData, clearSelection])

  return { loadData, handleConceptClick, nodes, edges }
}
