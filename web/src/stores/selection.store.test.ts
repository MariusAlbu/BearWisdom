import { describe, it, expect, beforeEach } from 'vitest'
import { useSelectionStore } from './selection.store'
import type { D3Node } from '../types/graph.types'
import type { SymbolDetail, CallHierarchyItem } from '../types/api.types'

beforeEach(() => {
  useSelectionStore.setState(useSelectionStore.getInitialState())
})

const makeNode = (id: number, name = `node${id}`): D3Node => ({
  id,
  name,
  qualifiedName: `ns::${name}`,
  kind: 'function',
  filePath: 'src/bar.ts',
  concept: null,
  annotation: null,
  radius: 6,
  color: '#3fb950',
})

const makeDetail = (): SymbolDetail => ({
  name: 'foo',
  qualified_name: 'ns::foo',
  kind: 'function',
  file_path: 'src/foo.ts',
  start_line: 10,
  end_line: 20,
  signature: 'fn foo()',
  doc_comment: null,
  visibility: 'pub',
  incoming_edge_count: 2,
  outgoing_edge_count: 1,
  children: [],
})

const makeCall = (name: string): CallHierarchyItem => ({
  name,
  qualified_name: `ns::${name}`,
  kind: 'function',
  file_path: 'src/bar.ts',
  line: 5,
})

describe('selection store', () => {
  it('starts with no selection', () => {
    expect(useSelectionStore.getState().selectedNode).toBeNull()
  })

  it('starts with empty navHistory', () => {
    expect(useSelectionStore.getState().navHistory).toHaveLength(0)
  })

  it('starts with default detailWidth', () => {
    expect(useSelectionStore.getState().detailWidth).toBe(380)
  })

  it('selectNode sets selectedNode and pushes to history', () => {
    const node = makeNode(1)
    useSelectionStore.getState().selectNode(node)

    const state = useSelectionStore.getState()
    expect(state.selectedNode).toBe(node)
    expect(state.navHistory).toHaveLength(1)
    expect(state.navHistory[0].node).toBe(node)
    expect(state.navHistory[0].label).toBe(node.name)
  })

  it('selectNode with same id does not duplicate history', () => {
    const node = makeNode(1)
    useSelectionStore.getState().selectNode(node)
    useSelectionStore.getState().selectNode(node)

    expect(useSelectionStore.getState().navHistory).toHaveLength(1)
  })

  it('selectNode with different node appends to history', () => {
    useSelectionStore.getState().selectNode(makeNode(1))
    useSelectionStore.getState().selectNode(makeNode(2))

    expect(useSelectionStore.getState().navHistory).toHaveLength(2)
  })

  it('selectNode(null) clears all selection state', () => {
    useSelectionStore.getState().selectNode(makeNode(1))
    useSelectionStore.getState().selectNode(null)

    const state = useSelectionStore.getState()
    expect(state.selectedNode).toBeNull()
    expect(state.navHistory).toHaveLength(0)
    expect(state.symbolDetail).toBeNull()
  })

  it('setDetail updates detail fields', () => {
    const detail = makeDetail()
    const incoming = [makeCall('caller')]
    const outgoing = [makeCall('callee')]

    useSelectionStore.getState().setDetail(detail, incoming, outgoing)

    const state = useSelectionStore.getState()
    expect(state.symbolDetail).toBe(detail)
    expect(state.incomingCalls).toHaveLength(1)
    expect(state.outgoingCalls).toHaveLength(1)
  })

  it('setDetailLoading updates detailLoading', () => {
    useSelectionStore.getState().setDetailLoading(true)
    expect(useSelectionStore.getState().detailLoading).toBe(true)
    useSelectionStore.getState().setDetailLoading(false)
    expect(useSelectionStore.getState().detailLoading).toBe(false)
  })

  it('setDetailTab switches tab', () => {
    expect(useSelectionStore.getState().detailTab).toBe('preview')
    useSelectionStore.getState().setDetailTab('code')
    expect(useSelectionStore.getState().detailTab).toBe('code')
  })

  it('setCodeContent stores content', () => {
    useSelectionStore.getState().setCodeContent('fn main() {}')
    expect(useSelectionStore.getState().codeContent).toBe('fn main() {}')
  })

  it('setCodeLoading updates codeLoading', () => {
    useSelectionStore.getState().setCodeLoading(true)
    expect(useSelectionStore.getState().codeLoading).toBe(true)
  })

  it('setDetailWidth clamps and stores width', () => {
    useSelectionStore.getState().setDetailWidth(500)
    expect(useSelectionStore.getState().detailWidth).toBe(500)
  })

  it('navigateBack truncates history and restores node', () => {
    const n1 = makeNode(1)
    const n2 = makeNode(2)
    const n3 = makeNode(3)
    useSelectionStore.getState().selectNode(n1)
    useSelectionStore.getState().selectNode(n2)
    useSelectionStore.getState().selectNode(n3)

    useSelectionStore.getState().navigateBack(0)

    const state = useSelectionStore.getState()
    expect(state.navHistory).toHaveLength(1)
    expect(state.selectedNode?.id).toBe(1)
  })

  it('clearSelection resets everything to initial state', () => {
    useSelectionStore.getState().selectNode(makeNode(1))
    useSelectionStore.getState().setDetailTab('code')
    useSelectionStore.getState().setDetailWidth(600)
    useSelectionStore.getState().clearSelection()

    const state = useSelectionStore.getState()
    expect(state.selectedNode).toBeNull()
    expect(state.detailTab).toBe('preview')
    expect(state.detailWidth).toBe(380)
    expect(state.navHistory).toHaveLength(0)
  })
})
