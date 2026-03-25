import type * as d3 from 'd3'

export interface D3Node extends d3.SimulationNodeDatum {
  id: number
  name: string
  qualifiedName: string
  kind: string
  filePath: string
  concept: string | null
  annotation: string | null
  radius: number
  color: string
}

export interface D3Edge extends d3.SimulationLinkDatum<D3Node> {
  kind: string
  confidence: number
}

export type LoadState = 'idle' | 'loading' | 'ready' | 'error'

export interface TooltipState {
  visible: boolean
  x: number
  y: number
  node: D3Node | null
}
