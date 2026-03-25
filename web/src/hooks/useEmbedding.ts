import { useCallback, useEffect } from 'react'
import { api } from '../api'
import { useEmbedStore } from '../stores/embed.store'

export function useEmbedding() {
  const { state: embedState, embedded: embedCount, error: embedError, setStatus, setRunning } = useEmbedStore()

  // Poll embed status every 2 seconds while running
  useEffect(() => {
    if (embedState !== 'running') return

    const intervalId = setInterval(async () => {
      try {
        const status = await api.embedStatus()
        const state = status.state as 'idle' | 'running' | 'done' | 'error'
        setStatus(state, status.embedded, status.error)
      } catch {
        // Silently ignore polling errors
      }
    }, 2000)

    return () => clearInterval(intervalId)
  }, [embedState, setStatus])

  const triggerEmbed = useCallback(
    async (workspacePath: string) => {
      setRunning()
      try {
        await api.embed(workspacePath)
        // Polling will pick up the status from here
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err)
        setStatus('error', 0, msg)
      }
    },
    [setRunning, setStatus],
  )

  return { embedState, embedCount, embedError, triggerEmbed }
}
