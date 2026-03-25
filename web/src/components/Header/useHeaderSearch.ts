import { useState, useRef, useCallback } from 'react'
import { api } from '../../api'
import type { SearchMode } from '../../types/api.types'
import type { AnyResult } from './Header'

export function useHeaderSearch(workspacePath: string) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<AnyResult[]>([])
  const [panelOpen, setPanelOpen] = useState(false)
  const [searching, setSearching] = useState(false)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const search = useCallback(
    async (q: string, m: SearchMode) => {
      if (!q.trim()) {
        setResults([])
        setPanelOpen(false)
        return
      }
      setSearching(true)
      try {
        let mapped: AnyResult[] = []
        switch (m) {
          case 'symbols': {
            const res = await api.searchSymbols(workspacePath, q)
            mapped = res.map((r) => ({ type: 'symbol' as const, data: r }))
            break
          }
          case 'fuzzy-symbols': {
            const res = await api.fuzzySymbols(workspacePath, q)
            mapped = res.map((r) => ({ type: 'fuzzy' as const, data: r }))
            break
          }
          case 'fuzzy-files': {
            const res = await api.fuzzyFiles(workspacePath, q)
            mapped = res.map((r) => ({ type: 'fuzzy' as const, data: r }))
            break
          }
          case 'content': {
            const res = await api.searchContent(workspacePath, q)
            mapped = res.map((r) => ({ type: 'content' as const, data: r }))
            break
          }
          case 'grep': {
            const res = await api.grep(workspacePath, q)
            mapped = res.map((r) => ({ type: 'grep' as const, data: r }))
            break
          }
          case 'hybrid': {
            const res = await api.hybrid(workspacePath, q)
            mapped = res.map((r) => ({ type: 'hybrid' as const, data: r }))
            break
          }
        }
        setResults(mapped)
        setPanelOpen(true)
      } catch {
        setResults([])
      } finally {
        setSearching(false)
      }
    },
    [workspacePath],
  )

  function handleChange(e: React.ChangeEvent<HTMLInputElement>, mode: SearchMode) {
    const q = e.target.value
    setQuery(q)
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(() => search(q, mode), 300)
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>, mode: SearchMode) {
    if (e.key === 'Enter') {
      if (debounceRef.current) clearTimeout(debounceRef.current)
      search(query, mode)
    }
  }

  function clearSearch() {
    setQuery('')
    setResults([])
    setPanelOpen(false)
  }

  return {
    query,
    results,
    panelOpen,
    searching,
    setPanelOpen,
    search,
    handleChange,
    handleKeyDown,
    clearSearch,
  }
}
