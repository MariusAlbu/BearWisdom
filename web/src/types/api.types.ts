export interface GraphNode {
  id: number
  name: string
  qualified_name: string
  kind: string
  file_path: string
  concept: string | null
  annotation: string | null
}

export interface GraphEdge {
  source_id: number
  target_id: number
  kind: string
  confidence: number
}

export interface SubgraphResult {
  nodes: GraphNode[]
  edges: GraphEdge[]
}

export interface ConceptSummary {
  id: number
  name: string
  description: string | null
  auto_pattern: string | null
  member_count: number
}

export interface SymbolDetail {
  name: string
  qualified_name: string
  kind: string
  file_path: string
  start_line: number
  end_line: number
  signature: string | null
  doc_comment: string | null
  visibility: string | null
  incoming_edge_count: number
  outgoing_edge_count: number
  children: {
    name: string
    qualified_name: string
    kind: string
    file_path: string
    line: number
  }[]
}

export interface CallHierarchyItem {
  name: string
  qualified_name: string
  kind: string
  file_path: string
  line: number
}

export interface SearchResult {
  name: string
  qualified_name: string
  kind: string
  file_path: string
  start_line: number
  signature: string | null
  score: number
}

export interface IndexStats {
  db_path: string
  file_count: number
  symbol_count: number
  edge_count: number
  unresolved_ref_count: number
  duration_ms: number
}

export interface ArchitectureOverview {
  total_files: number
  total_symbols: number
  total_edges: number
  languages: { language: string; file_count: number; symbol_count: number }[]
  hotspots: {
    name: string
    qualified_name: string
    kind: string
    file_path: string
    incoming_refs: number
  }[]
  entry_points: {
    name: string
    qualified_name: string
    kind: string
    file_path: string
    line: number
  }[]
}

export interface BrowseResult {
  dirs: string[]
  files: string[]
}

export interface FuzzyMatch {
  text: string
  score: number
  indices: number[]
  metadata: { File?: { language: string }; Symbol?: { kind: string; file_path: string; line: number } }
}

export interface GrepMatch {
  file_path: string
  line_number: number
  column: number
  line_content: string
  match_start: number
  match_end: number
}

export interface ContentSearchResult {
  file_id: number
  file_path: string
  language: string
  score: number
}

export interface HybridSearchResult {
  file_path: string
  symbol_name: string | null
  kind: string | null
  start_line: number
  end_line: number
  content_preview: string
  rrf_score: number
  text_rank: number | null
  vector_rank: number | null
}

export type SearchMode = 'symbols' | 'fuzzy-symbols' | 'fuzzy-files' | 'content' | 'grep' | 'hybrid'
