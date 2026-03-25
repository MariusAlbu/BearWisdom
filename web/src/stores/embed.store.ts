import { create } from 'zustand'

type EmbedRunState = 'idle' | 'running' | 'done' | 'error'

interface EmbedState {
  state: EmbedRunState
  embedded: number
  error: string | null
}

interface EmbedActions {
  setStatus: (state: EmbedRunState, embedded: number, error: string | null) => void
  setRunning: () => void
}

const initialState: EmbedState = {
  state: 'idle',
  embedded: 0,
  error: null,
}

export const useEmbedStore = create<EmbedState & EmbedActions>()((set) => ({
  ...initialState,

  setStatus: (state, embedded, error) =>
    set({ state, embedded, error }),

  setRunning: () =>
    set({ state: 'running', error: null }),
}))
