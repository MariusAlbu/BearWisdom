import { create } from 'zustand'
import type { SymbolDetail, CallHierarchyItem } from '../types/api.types'
import type { D3Node } from '../types/graph.types'

interface SelectionState {
  selectedNode: D3Node | null
  symbolDetail: SymbolDetail | null
  incomingCalls: CallHierarchyItem[]
  outgoingCalls: CallHierarchyItem[]
  detailLoading: boolean
  detailTab: 'preview' | 'code'
  codeContent: string | null
  codeLoading: boolean
  navHistory: Array<{ node: D3Node; label: string }>
  detailWidth: number
}

interface SelectionActions {
  selectNode: (node: D3Node | null) => void
  setDetail: (
    symbolDetail: SymbolDetail | null,
    incoming: CallHierarchyItem[],
    outgoing: CallHierarchyItem[],
  ) => void
  setDetailLoading: (loading: boolean) => void
  setDetailTab: (tab: 'preview' | 'code') => void
  setCodeContent: (content: string | null) => void
  setCodeLoading: (loading: boolean) => void
  setDetailWidth: (width: number) => void
  navigateBack: (index: number) => void
  clearSelection: () => void
}

const initialState: SelectionState = {
  selectedNode: null,
  symbolDetail: null,
  incomingCalls: [],
  outgoingCalls: [],
  detailLoading: false,
  detailTab: 'preview',
  codeContent: null,
  codeLoading: false,
  navHistory: [],
  detailWidth: 380,
}

export const useSelectionStore = create<SelectionState & SelectionActions>()((set, get) => ({
  ...initialState,

  selectNode: (node) => {
    if (node === null) {
      set({
        selectedNode: null,
        symbolDetail: null,
        incomingCalls: [],
        outgoingCalls: [],
        detailLoading: false,
        detailTab: 'preview',
        codeContent: null,
        navHistory: [],
      })
      return
    }

    const { navHistory } = get()
    const lastId = navHistory.length > 0 ? navHistory[navHistory.length - 1].node.id : null
    const newHistory =
      lastId === node.id
        ? navHistory
        : [...navHistory, { node, label: node.name }]

    set({
      selectedNode: node,
      symbolDetail: null,
      incomingCalls: [],
      outgoingCalls: [],
      detailLoading: false,
      codeContent: null,
      navHistory: newHistory,
    })
  },

  setDetail: (symbolDetail, incoming, outgoing) =>
    set({ symbolDetail, incomingCalls: incoming, outgoingCalls: outgoing }),

  setDetailLoading: (loading) => set({ detailLoading: loading }),

  setDetailTab: (tab) => set({ detailTab: tab }),

  setCodeContent: (content) => set({ codeContent: content }),

  setCodeLoading: (loading) => set({ codeLoading: loading }),

  setDetailWidth: (width) => set({ detailWidth: width }),

  navigateBack: (index) => {
    const { navHistory } = get()
    const entry = navHistory[index]
    if (!entry) return
    set({
      navHistory: navHistory.slice(0, index + 1),
      selectedNode: entry.node,
    })
  },

  clearSelection: () => set(initialState),
}))
