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
  external_ref_count: number
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

export interface FlowEdge {
  source_file: string
  source_line: number | null
  source_symbol: string | null
  source_language: string
  target_file: string | null
  target_line: number | null
  target_symbol: string | null
  target_language: string
  edge_type: string
  protocol: string | null
  url_pattern: string | null
}

export interface FlowEdgesResult {
  edges: FlowEdge[]
  summary: {
    total: number
    by_edge_type: Record<string, number>
    by_language_pair: Record<string, number>
  }
}

export interface FlowStep {
  depth: number
  file_path: string
  line: number | null
  symbol: string | null
  language: string
  edge_type: string
  protocol: string | null
}

export interface TraceNode {
  name: string
  qualified_name: string
  kind: string
  file_path: string
  line: number
  edge_kind: string
  depth: number
  children: TraceNode[]
}

export interface TraceRoot {
  entry: TraceNode
  node_count: number
}

export interface FullTraceResult {
  traces: TraceRoot[]
  total_symbols: number
  flow_jumps: number
}

// ---------------------------------------------------------------------------
// Hierarchy (architectural zoom levels)
// ---------------------------------------------------------------------------

export interface HierarchyNode {
  id: string
  name: string
  kind: string // "service" | "package" | "file" | "class" | "method" | etc.
  file_path?: string
  package?: string
  weight: number
  child_count: number
  metadata?: string // JSON string
}

export interface HierarchyEdge {
  source: string
  target: string
  kind: string // "service_dependency" | "cross_package" | "file_dependency" | "calls"
  weight: number
  confidence: number
}

export interface Breadcrumb {
  label: string
  level: string
  scope?: string
}

export interface HierarchyResult {
  nodes: HierarchyNode[]
  edges: HierarchyEdge[]
  level: string // "services" | "packages" | "files" | "symbols"
  scope?: string
  breadcrumbs: Breadcrumb[]
}

// ---------------------------------------------------------------------------
// Dead Code / Entry Points
// ---------------------------------------------------------------------------

export interface DeadCodeEntry {
  symbol_id: number
  name: string
  qualified_name: string
  kind: string
  visibility: string | null
  file_path: string
  line: number
  confidence: number
  reason: 'no_incoming_edges' | 'only_low_confidence_edges'
  potentially_referenced?: boolean
  unresolved_ref_matches?: number
}

export interface ResolutionHealth {
  resolution_rate: number
  resolved_refs: number
  unresolved_refs: number
  assessment: string
}

export interface DeadCodeReport {
  total_symbols_checked: number
  dead_candidates: DeadCodeEntry[]
  entry_points_excluded: number
  test_symbols_excluded: number
  potentially_referenced_count: number
  resolution_health: ResolutionHealth
}

export interface EntryPoint {
  symbol_id: number
  name: string
  qualified_name: string
  kind: string
  file_path: string
  line: number
  entry_kind: 'main' | 'route_handler' | 'event_handler' | 'test_function' | 'exported_api' | 'lifecycle_hook' | 'di_registered'
}

export interface EntryPointsReport {
  total: number
  entry_points: EntryPoint[]
}

// ---------------------------------------------------------------------------
// MCP Audit log
// ---------------------------------------------------------------------------

export interface AuditRecord {
  id: number
  session_id: string
  tool_name: string
  params_json: string
  response_json: string
  duration_ms: number
  token_estimate: number
  ts: string
}

export interface AuditSessionSummary {
  session_id: string
  call_count: number
  total_tokens: number
  first_ts: string
  last_ts: string
}

export interface AuditStats {
  total_calls: number
  total_tokens: number
  avg_duration_ms: number
  session_count: number
  calls_by_tool: [string, number][]
}
