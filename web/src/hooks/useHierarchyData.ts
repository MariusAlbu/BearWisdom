import { useCallback, useEffect, useRef } from 'react'
import { useHierarchyStore } from '../stores/hierarchy.store'

export function useHierarchyData(workspacePath: string) {
  const setLoadState = useHierarchyStore((s) => s.setLoadState)
  const setData = useHierarchyStore((s) => s.setData)
  const pendingDrill = useHierarchyStore((s) => s.pendingDrill)
  const clearPendingDrill = useHierarchyStore((s) => s.clearPendingDrill)

  // Stable ref for workspacePath to avoid re-creating loadLevel on path change
  const pathRef = useRef(workspacePath)
  pathRef.current = workspacePath

  const loadLevel = useCallback(
    async (level: string, scope?: string) => {
      setLoadState('loading')
      try {
        const params = new URLSearchParams({ path: pathRef.current, level })
        if (scope) params.set('scope', scope)
        const res = await fetch(`/api/hierarchy?${params}`)
        const data = (await res.json()) as { ok: boolean; data?: unknown; error?: string }
        if (data.ok && data.data) {
          setData(data.data as Parameters<typeof setData>[0])
        } else {
          setLoadState('error', data.error ?? 'Unknown error')
        }
      } catch (err) {
        setLoadState('error', err instanceof Error ? err.message : String(err))
      }
    },
    [setLoadState, setData],
  )

  // Consume pendingDrill set by drillDown() / navigateTo()
  useEffect(() => {
    if (!pendingDrill) return
    clearPendingDrill()
    void loadLevel(pendingDrill.level, pendingDrill.scope || undefined)
  }, [pendingDrill, loadLevel, clearPendingDrill])

  // Load default level on mount
  const mountedRef = useRef(false)
  useEffect(() => {
    if (mountedRef.current) return
    mountedRef.current = true
    if (workspacePath) {
      void loadLevel('packages')
    }
  }, [workspacePath, loadLevel])

  return { loadLevel }
}
