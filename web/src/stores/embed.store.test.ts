import { describe, it, expect, beforeEach } from 'vitest'
import { useEmbedStore } from './embed.store'

beforeEach(() => {
  useEmbedStore.setState(useEmbedStore.getInitialState())
})

describe('embed store', () => {
  it('starts in idle state', () => {
    expect(useEmbedStore.getState().state).toBe('idle')
  })

  it('starts with zero embedded count', () => {
    expect(useEmbedStore.getState().embedded).toBe(0)
  })

  it('starts with null error', () => {
    expect(useEmbedStore.getState().error).toBeNull()
  })

  it('setRunning transitions to running and clears error', () => {
    useEmbedStore.getState().setStatus('error', 0, 'previous error')
    useEmbedStore.getState().setRunning()

    const state = useEmbedStore.getState()
    expect(state.state).toBe('running')
    expect(state.error).toBeNull()
  })

  it('setStatus updates all fields', () => {
    useEmbedStore.getState().setStatus('done', 42, null)

    const state = useEmbedStore.getState()
    expect(state.state).toBe('done')
    expect(state.embedded).toBe(42)
    expect(state.error).toBeNull()
  })

  it('setStatus to error stores error message', () => {
    useEmbedStore.getState().setStatus('error', 5, 'embed failed')

    const state = useEmbedStore.getState()
    expect(state.state).toBe('error')
    expect(state.embedded).toBe(5)
    expect(state.error).toBe('embed failed')
  })

  it('setStatus running increments count', () => {
    useEmbedStore.getState().setStatus('running', 10, null)
    expect(useEmbedStore.getState().embedded).toBe(10)
  })
})
