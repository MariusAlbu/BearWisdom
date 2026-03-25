import type { GraphNode, GraphEdge } from '../types/api.types'
import type { D3Node, D3Edge } from '../types/graph.types'

// ---------------------------------------------------------------------------
// Color constants
// ---------------------------------------------------------------------------

export const KIND_COLOR: Record<string, string> = {
  class: '#58a6ff',
  interface: '#bc8cff',
  method: '#3fb950',
  function: '#3fb950',
  enum: '#d29922',
  struct: '#58a6ff',
  type: '#bc8cff',
  module: '#39c5cf',
  constant: '#e3b341',
  field: '#8b949e',
  variable: '#8b949e',
}

export const DEFAULT_KIND_COLOR = '#6e7681'

export const CONCEPT_COLORS = [
  '#58a6ff',
  '#bc8cff',
  '#3fb950',
  '#d29922',
  '#39c5cf',
  '#f85149',
  '#e3b341',
  '#56d4dd',
]

// ---------------------------------------------------------------------------
// Pure utility functions
// ---------------------------------------------------------------------------

export function kindColor(kind: string): string {
  return KIND_COLOR[kind.toLowerCase()] ?? DEFAULT_KIND_COLOR
}

export function computeRadius(incomingCount: number): number {
  const base = 6
  const scale = Math.sqrt(incomingCount)
  return Math.min(Math.max(base + scale * 2, base), 28)
}

export function shortPath(filePath: string): string {
  const parts = filePath.replace(/\\/g, '/').split('/')
  if (parts.length <= 3) return filePath
  return `\u2026/${parts.slice(-2).join('/')}`
}

// ---------------------------------------------------------------------------
// Wire → D3 transforms
// ---------------------------------------------------------------------------

export function toD3Node(node: GraphNode, incomingCount: number): D3Node {
  return {
    id: node.id,
    name: node.name,
    qualifiedName: node.qualified_name,
    kind: node.kind,
    filePath: node.file_path,
    concept: node.concept,
    annotation: node.annotation,
    radius: computeRadius(incomingCount),
    color: kindColor(node.kind),
  }
}

export function toD3Edge(edge: GraphEdge, nodeMap: Map<number, D3Node>): D3Edge | null {
  const source = nodeMap.get(edge.source_id)
  const target = nodeMap.get(edge.target_id)
  if (!source || !target) return null
  return { source, target, kind: edge.kind, confidence: edge.confidence }
}
