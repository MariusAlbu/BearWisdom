import { create } from 'zustand'
import type { HierarchyNode, HierarchyEdge, Breadcrumb, HierarchyResult } from '../types/api.types'

// The level ordering determines which level to navigate to on drill-down.
const LEVEL_ORDER = ['services', 'packages', 'files', 'symbols'] as const
type Level = (typeof LEVEL_ORDER)[number]

function nextLevel(current: string): Level | null {
  const idx = LEVEL_ORDER.indexOf(current as Level)
  if (idx === -1 || idx === LEVEL_ORDER.length - 1) return null
  return LEVEL_ORDER[idx + 1]
}

interface HierarchyState {
  nodes: HierarchyNode[]
  edges: HierarchyEdge[]
  level: string
  scope: string | null
  breadcrumbs: Breadcrumb[]
  loadState: 'idle' | 'loading' | 'ready' | 'error'
  errorMessage: string | null
  selectedNodeId: string | null
  // Resolved drill-down target — set by drillDown(), consumed by the hook
  pendingDrill: { level: string; scope: string } | null
}

interface HierarchyActions {
  setData(result: HierarchyResult): void
  setLoadState(state: HierarchyState['loadState'], error?: string): void
  drillDown(nodeId: string): void
  navigateTo(level: string, scope?: string): void
  selectNode(id: string | null): void
  clearPendingDrill(): void
  reset(): void
}

const initialState: HierarchyState = {
  nodes: [],
  edges: [],
  level: 'packages',
  scope: null,
  breadcrumbs: [],
  loadState: 'idle',
  errorMessage: null,
  selectedNodeId: null,
  pendingDrill: null,
}

export const useHierarchyStore = create<HierarchyState & HierarchyActions>()((set, get) => ({
  ...initialState,

  setData: (result) =>
    set({
      nodes: result.nodes,
      edges: result.edges,
      level: result.level,
      scope: result.scope ?? null,
      breadcrumbs: result.breadcrumbs,
      loadState: 'ready',
      errorMessage: null,
      pendingDrill: null,
    }),

  setLoadState: (state, error) =>
    set({ loadState: state, errorMessage: error ?? null }),

  drillDown: (nodeId) => {
    const { level, nodes } = get()
    const node = nodes.find((n) => n.id === nodeId)
    if (!node || node.child_count === 0) return
    const target = nextLevel(level)
    if (!target) return
    set({ pendingDrill: { level: target, scope: nodeId } })
  },

  navigateTo: (level, scope) =>
    set({ pendingDrill: { level, scope: scope ?? '' } }),

  selectNode: (id) => set({ selectedNodeId: id }),

  clearPendingDrill: () => set({ pendingDrill: null }),

  reset: () => set(initialState),
}))
