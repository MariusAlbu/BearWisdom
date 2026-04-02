import { create } from 'zustand'
import type { AuditRecord, AuditSessionSummary, AuditStats } from '../types/api.types'

interface AuditState {
  sessions: AuditSessionSummary[]
  activeSessionId: string | null
  calls: AuditRecord[]
  stats: AuditStats | null
  selectedCallId: number | null
  loadingCalls: boolean
  error: string | null
}

interface AuditActions {
  setSessions: (sessions: AuditSessionSummary[]) => void
  setActiveSession: (sessionId: string | null) => void
  setCalls: (calls: AuditRecord[]) => void
  prependCalls: (newCalls: AuditRecord[]) => void
  setStats: (stats: AuditStats) => void
  setSelectedCall: (id: number | null) => void
  setLoadingCalls: (loading: boolean) => void
  setError: (msg: string | null) => void
  removeSession: (sessionId: string) => void
}

const initialState: AuditState = {
  sessions: [],
  activeSessionId: null,
  calls: [],
  stats: null,
  selectedCallId: null,
  loadingCalls: false,
  error: null,
}

export const useAuditStore = create<AuditState & AuditActions>()((set) => ({
  ...initialState,

  setSessions: (sessions) => set({ sessions }),

  setActiveSession: (sessionId) =>
    set({ activeSessionId: sessionId, calls: [], selectedCallId: null }),

  setCalls: (calls) => set({ calls, loadingCalls: false }),

  // Prepend new live calls from the SSE stream, deduplicating by id.
  // Session sidebar counts are refreshed from the DB by the SSE handler
  // in Inspector, so we only update the calls array here.
  prependCalls: (newCalls) =>
    set((state) => {
      const existingIds = new Set(state.calls.map((c) => c.id))
      const fresh = newCalls.filter((c) => !existingIds.has(c.id))
      if (fresh.length === 0) return {}
      // Keep newest first — SSE gives us oldest-first from the DB, so reverse.
      const prepended = [...fresh.reverse(), ...state.calls]
      return { calls: prepended }
    }),

  setStats: (stats) => set({ stats }),

  setSelectedCall: (id) => set({ selectedCallId: id }),

  setLoadingCalls: (loading) => set({ loadingCalls: loading }),

  setError: (msg) => set({ error: msg }),

  removeSession: (sessionId) =>
    set((state) => ({
      sessions: state.sessions.filter((s) => s.session_id !== sessionId),
      activeSessionId: state.activeSessionId === sessionId ? null : state.activeSessionId,
      calls: state.activeSessionId === sessionId ? [] : state.calls,
    })),
}))
