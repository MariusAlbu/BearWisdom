import { useCallback } from 'react'
import { useSelectionStore } from '../stores/selection.store'

export function useResizePanel() {
  const detailWidth = useSelectionStore((s) => s.detailWidth)
  const setDetailWidth = useSelectionStore((s) => s.setDetailWidth)

  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startWidth = detailWidth

      function onMouseMove(ev: MouseEvent) {
        const delta = startX - ev.clientX
        const newWidth = Math.min(Math.max(startWidth + delta, 280), window.innerWidth * 0.6)
        setDetailWidth(newWidth)
      }

      function onMouseUp() {
        document.removeEventListener('mousemove', onMouseMove)
        document.removeEventListener('mouseup', onMouseUp)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
      }

      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
      document.addEventListener('mousemove', onMouseMove)
      document.addEventListener('mouseup', onMouseUp)
    },
    [detailWidth, setDetailWidth],
  )

  return { handleResizeStart }
}
