import { describe, it, expect, beforeEach } from 'vitest'
import { useGraphStore } from './graph.store'
import type { D3Node, D3Edge } from '../types/graph.types'
import type { ConceptSummary } from '../types/api.types'

beforeEach(() => {
  useGraphStore.setState(useGraphStore.getInitialState())
})

const makeNode = (id: number): D3Node => ({
  id,
  name: `node${id}`,
  qualifiedName: `ns::node${id}`,
  kind: 'class',
  filePath: 'src/foo.ts',
  concept: null,
  annotation: null,
  radius: 8,
  color: '#58a6ff',
})

const makeConcept = (name: string): ConceptSummary => ({
  id: 1,
  name,
  description: null,
  auto_pattern: null,
  member_count: 3,
})

describe('graph store', () => {
  it('starts in idle state', () => {
    expect(useGraphStore.getState().loadState).toBe('idle')
  })

  it('starts with empty nodes and edges', () => {
    const { nodes, edges } = useGraphStore.getState()
    expect(nodes).toHaveLength(0)
    expect(edges).toHaveLength(0)
  })

  it('starts with sidebar open', () => {
    expect(useGraphStore.getState().sidebarOpen).toBe(true)
  })

  it('starts with null activeConcept', () => {
    expect(useGraphStore.getState().activeConcept).toBeNull()
  })

  it('setGraphData sets nodes, edges, concepts and moves to ready', () => {
    const nodes = [makeNode(1), makeNode(2)]
    const edges: D3Edge[] = []
    const concepts = [makeConcept('auth')]

    useGraphStore.getState().setGraphData(nodes, edges, concepts)

    const state = useGraphStore.getState()
    expect(state.nodes).toHaveLength(2)
    expect(state.edges).toHaveLength(0)
    expect(state.concepts).toHaveLength(1)
    expect(state.loadState).toBe('ready')
    expect(state.overview.totalNodes).toBe(2)
    expect(state.overview.totalEdges).toBe(0)
  })

  it('setLoadState transitions to loading', () => {
    useGraphStore.getState().setLoadState('loading')
    expect(useGraphStore.getState().loadState).toBe('loading')
    expect(useGraphStore.getState().errorMessage).toBeNull()
  })

  it('setLoadState with error sets errorMessage', () => {
    useGraphStore.getState().setLoadState('error', 'network failure')
    const state = useGraphStore.getState()
    expect(state.loadState).toBe('error')
    expect(state.errorMessage).toBe('network failure')
  })

  it('setActiveConcept updates activeConcept', () => {
    useGraphStore.getState().setActiveConcept('auth')
    expect(useGraphStore.getState().activeConcept).toBe('auth')
  })

  it('setActiveConcept to null clears activeConcept', () => {
    useGraphStore.getState().setActiveConcept('auth')
    useGraphStore.getState().setActiveConcept(null)
    expect(useGraphStore.getState().activeConcept).toBeNull()
  })

  it('toggleSidebar flips sidebarOpen', () => {
    expect(useGraphStore.getState().sidebarOpen).toBe(true)
    useGraphStore.getState().toggleSidebar()
    expect(useGraphStore.getState().sidebarOpen).toBe(false)
    useGraphStore.getState().toggleSidebar()
    expect(useGraphStore.getState().sidebarOpen).toBe(true)
  })

  it('reset returns to initial state', () => {
    useGraphStore.getState().setGraphData([makeNode(1)], [], [])
    useGraphStore.getState().setActiveConcept('auth')
    useGraphStore.getState().reset()

    const state = useGraphStore.getState()
    expect(state.loadState).toBe('idle')
    expect(state.nodes).toHaveLength(0)
    expect(state.activeConcept).toBeNull()
  })
})
