import { create } from 'zustand'
import type { ConceptSummary } from '../types/api.types'
import type { D3Node, D3Edge, LoadState } from '../types/graph.types'

interface GraphState {
  nodes: D3Node[]
  edges: D3Edge[]
  concepts: ConceptSummary[]
  loadState: LoadState
  errorMessage: string | null
  overview: { totalNodes: number; totalEdges: number }
  activeConcept: string | null
  sidebarOpen: boolean
}

interface GraphActions {
  setGraphData: (nodes: D3Node[], edges: D3Edge[], concepts: ConceptSummary[]) => void
  setLoadState: (state: LoadState, error?: string) => void
  setActiveConcept: (name: string | null) => void
  toggleSidebar: () => void
  reset: () => void
}

const initialState: GraphState = {
  nodes: [],
  edges: [],
  concepts: [],
  loadState: 'idle',
  errorMessage: null,
  overview: { totalNodes: 0, totalEdges: 0 },
  activeConcept: null,
  sidebarOpen: true,
}

export const useGraphStore = create<GraphState & GraphActions>()((set) => ({
  ...initialState,

  setGraphData: (nodes, edges, concepts) =>
    set({
      nodes,
      edges,
      concepts,
      loadState: 'ready',
      overview: { totalNodes: nodes.length, totalEdges: edges.length },
    }),

  setLoadState: (state, error) =>
    set({ loadState: state, errorMessage: error ?? null }),

  setActiveConcept: (name) =>
    set({ activeConcept: name }),

  toggleSidebar: () =>
    set((s) => ({ sidebarOpen: !s.sidebarOpen })),

  reset: () => set(initialState),
}))
