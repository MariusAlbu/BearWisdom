import { useCallback, useEffect } from 'react'
import { useHierarchyStore } from '../stores/hierarchy.store'

export function useHierarchyData(workspacePath: string) {
  const store = useHierarchyStore()
  const pendingDrill = useHierarchyStore((s) => s.pendingDrill)
  const clearPendingDrill = useHierarchyStore((s) => s.clearPendingDrill)

  const loadLevel = useCallback(
    async (level: string, scope?: string) => {
      store.setLoadState('loading')
      try {
        const params = new URLSearchParams({ path: workspacePath, level })
        if (scope) params.set('scope', scope)
        const res = await fetch(`/api/hierarchy?${params}`)
        const data = (await res.json()) as { ok: boolean; data?: unknown; error?: string }
        if (data.ok && data.data) {
          store.setData(data.data as Parameters<typeof store.setData>[0])
        } else {
          store.setLoadState('error', data.error ?? 'Unknown error')
        }
      } catch (err) {
        store.setLoadState('error', err instanceof Error ? err.message : String(err))
      }
    },
    [workspacePath, store],
  )

  // Consume pendingDrill set by drillDown() / navigateTo()
  useEffect(() => {
    if (!pendingDrill) return
    clearPendingDrill()
    void loadLevel(pendingDrill.level, pendingDrill.scope || undefined)
  }, [pendingDrill, loadLevel, clearPendingDrill])

  // Load default level on mount / workspace change
  useEffect(() => {
    if (workspacePath) {
      void loadLevel('packages')
    }
  }, [workspacePath, loadLevel])

  return { ...store, loadLevel }
}
