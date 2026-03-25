import { describe, it, expect } from 'vitest'
import {
  kindColor,
  computeRadius,
  shortPath,
  toD3Node,
  DEFAULT_KIND_COLOR,
} from './graph.utils'
import type { GraphNode } from '../types/api.types'

const makeWireNode = (overrides: Partial<GraphNode> = {}): GraphNode => ({
  id: 1,
  name: 'MyClass',
  qualified_name: 'ns::MyClass',
  kind: 'class',
  file_path: 'src/foo.ts',
  concept: null,
  annotation: null,
  ...overrides,
})

describe('kindColor', () => {
  it('returns correct color for class', () => {
    expect(kindColor('class')).toBe('#58a6ff')
  })

  it('returns correct color for interface', () => {
    expect(kindColor('interface')).toBe('#bc8cff')
  })

  it('returns correct color for method', () => {
    expect(kindColor('method')).toBe('#3fb950')
  })

  it('returns correct color for function', () => {
    expect(kindColor('function')).toBe('#3fb950')
  })

  it('returns correct color for enum', () => {
    expect(kindColor('enum')).toBe('#d29922')
  })

  it('returns default color for unknown kind', () => {
    expect(kindColor('unknown')).toBe(DEFAULT_KIND_COLOR)
  })

  it('is case-insensitive', () => {
    expect(kindColor('CLASS')).toBe('#58a6ff')
    expect(kindColor('Method')).toBe('#3fb950')
  })
})

describe('computeRadius', () => {
  it('returns base radius for zero incoming edges', () => {
    expect(computeRadius(0)).toBe(6)
  })

  it('returns base radius for one incoming edge', () => {
    // base=6, scale=sqrt(1)*2=2, clamped min=6, so 6+2=8
    expect(computeRadius(1)).toBe(8)
  })

  it('is capped at max radius of 28', () => {
    expect(computeRadius(1000)).toBe(28)
  })

  it('never goes below base radius of 6', () => {
    expect(computeRadius(0)).toBeGreaterThanOrEqual(6)
  })

  it('grows with incoming count up to max', () => {
    const r1 = computeRadius(1)
    const r10 = computeRadius(10)
    const r100 = computeRadius(100)
    expect(r10).toBeGreaterThan(r1)
    expect(r100).toBeGreaterThanOrEqual(r10)
  })
})

describe('shortPath', () => {
  it('returns path unchanged when 3 or fewer segments', () => {
    expect(shortPath('src/foo.ts')).toBe('src/foo.ts')
    expect(shortPath('a/b/c')).toBe('a/b/c')
  })

  it('truncates long paths with ellipsis prefix', () => {
    const result = shortPath('a/b/c/d/e.ts')
    expect(result).toBe('\u2026/d/e.ts')
  })

  it('normalizes backslashes to forward slashes before truncating', () => {
    const result = shortPath('a\\b\\c\\d\\e.ts')
    expect(result).toBe('\u2026/d/e.ts')
  })

  it('preserves paths with exactly 3 segments', () => {
    expect(shortPath('a/b/c')).toBe('a/b/c')
  })
})

describe('toD3Node', () => {
  it('maps wire fields to camelCase D3Node fields', () => {
    const wire = makeWireNode()
    const node = toD3Node(wire, 0)

    expect(node.id).toBe(wire.id)
    expect(node.name).toBe(wire.name)
    expect(node.qualifiedName).toBe(wire.qualified_name)
    expect(node.kind).toBe(wire.kind)
    expect(node.filePath).toBe(wire.file_path)
    expect(node.concept).toBeNull()
    expect(node.annotation).toBeNull()
  })

  it('sets color based on kind', () => {
    const node = toD3Node(makeWireNode({ kind: 'class' }), 0)
    expect(node.color).toBe('#58a6ff')
  })

  it('sets color to default for unknown kind', () => {
    const node = toD3Node(makeWireNode({ kind: 'trait' }), 0)
    expect(node.color).toBe(DEFAULT_KIND_COLOR)
  })

  it('computes radius from incomingCount', () => {
    const node0 = toD3Node(makeWireNode(), 0)
    const node4 = toD3Node(makeWireNode(), 4)
    expect(node4.radius).toBeGreaterThan(node0.radius)
  })

  it('preserves concept and annotation', () => {
    const node = toD3Node(makeWireNode({ concept: 'auth', annotation: 'hot path' }), 0)
    expect(node.concept).toBe('auth')
    expect(node.annotation).toBe('hot path')
  })
})
