import { useState } from 'react'
import { Landing } from './components/Landing'
import { Explorer } from './components/Explorer'
import type { IndexStats } from './types'

type AppView = 'landing' | 'explorer'

export function App() {
  const [view, setView] = useState<AppView>('landing')
  const [workspacePath, setWorkspacePath] = useState<string>('')
  const [indexStats, setIndexStats] = useState<IndexStats | null>(null)

  function handleIndexed(path: string, stats: IndexStats) {
    setWorkspacePath(path)
    setIndexStats(stats)
    setView('explorer')
  }

  if (view === 'explorer' && workspacePath && indexStats) {
    return (
      <Explorer
        workspacePath={workspacePath}
        stats={{
          files: indexStats.file_count,
          symbols: indexStats.symbol_count,
          edges: indexStats.edge_count,
        }}
      />
    )
  }

  return <Landing onIndexed={handleIndexed} />
}
