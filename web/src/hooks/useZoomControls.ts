import * as d3 from 'd3'
import { useCallback } from 'react'

export function useZoomControls(
  svgRef: React.RefObject<SVGSVGElement | null>,
  zoomRef: React.MutableRefObject<d3.ZoomBehavior<SVGSVGElement, unknown> | null>,
) {
  const handleZoomIn = useCallback(() => {
    if (!svgRef.current || !zoomRef.current) return
    d3.select<SVGSVGElement, unknown>(svgRef.current)
      .transition()
      .duration(200)
      .call(zoomRef.current.scaleBy, 1.4)
  }, [svgRef, zoomRef])

  const handleZoomOut = useCallback(() => {
    if (!svgRef.current || !zoomRef.current) return
    d3.select<SVGSVGElement, unknown>(svgRef.current)
      .transition()
      .duration(200)
      .call(zoomRef.current.scaleBy, 0.7)
  }, [svgRef, zoomRef])

  const handleZoomReset = useCallback(() => {
    if (!svgRef.current || !zoomRef.current) return
    d3.select<SVGSVGElement, unknown>(svgRef.current)
      .transition()
      .duration(300)
      .call(zoomRef.current.transform, d3.zoomIdentity)
  }, [svgRef, zoomRef])

  return { handleZoomIn, handleZoomOut, handleZoomReset }
}
