import { useCallback, useEffect } from 'react'
import { api } from '../api'
import { useGraphStore } from '../stores/graph.store'
import { useSelectionStore } from '../stores/selection.store'
import { DEFAULT_KIND_COLOR } from '../utils/graph.utils'
import type { D3Node } from '../types/graph.types'

export function useSymbolDetail(workspacePath: string) {
  const selectedNode = useSelectionStore((s) => s.selectedNode)
  const detailTab = useSelectionStore((s) => s.detailTab)
  const symbolDetail = useSelectionStore((s) => s.symbolDetail)
  const { setDetail, setDetailLoading, setCodeContent, setCodeLoading } = useSelectionStore()

  // Fetch symbol detail when selectedNode changes
  useEffect(() => {
    if (!selectedNode || !workspacePath) return

    setDetailLoading(true)
    setDetail(null, [], [])

    const qualifiedName = selectedNode.qualifiedName

    Promise.all([
      api.symbolInfo(workspacePath, qualifiedName),
      api.callsIn(workspacePath, qualifiedName),
      api.callsOut(workspacePath, qualifiedName),
    ])
      .then(([detailArr, incoming, outgoing]) => {
        setDetail(detailArr[0] ?? null, incoming, outgoing)
      })
      .catch(() => {
        // Non-fatal: detail panel shows partial info
      })
      .finally(() => setDetailLoading(false))
  }, [selectedNode, workspacePath, setDetail, setDetailLoading])

  // Fetch code content when tab switches to 'code'
  useEffect(() => {
    if (detailTab !== 'code' || !symbolDetail || !workspacePath) {
      setCodeContent(null)
      return
    }

    setCodeLoading(true)
    api.fileContent(workspacePath, symbolDetail.file_path)
      .then(({ content }) => {
        const lines = content.split('\n')
        const start = Math.max(0, symbolDetail.start_line - 11)
        const end = Math.min(lines.length, symbolDetail.end_line + 10)
        setCodeContent(lines.slice(start, end).join('\n'))
      })
      .catch(() => setCodeContent(null))
      .finally(() => setCodeLoading(false))
  }, [detailTab, symbolDetail, workspacePath, setCodeContent, setCodeLoading])

  const navigateToSymbol = useCallback(
    (qualifiedName: string) => {
      const nodes = useGraphStore.getState().nodes
      const node = nodes.find((n) => n.qualifiedName === qualifiedName)

      if (!node) {
        const synthetic: D3Node = {
          id: -1,
          name: qualifiedName.split('.').pop() ?? qualifiedName,
          qualifiedName,
          kind: 'unknown',
          filePath: '',
          concept: null,
          annotation: null,
          radius: 8,
          color: DEFAULT_KIND_COLOR,
        }
        useSelectionStore.getState().selectNode(synthetic)
        return
      }

      useSelectionStore.getState().selectNode(node)
    },
    [],
  )

  return { navigateToSymbol }
}
