import type {
  IndexStats,
  ArchitectureOverview,
  SearchResult,
  SubgraphResult,
  ConceptSummary,
  SymbolDetail,
  CallHierarchyItem,
  BrowseResult,
  FuzzyMatch,
  GrepMatch,
  ContentSearchResult,
  HybridSearchResult,
  FlowEdgesResult,
  FlowStep,
  FullTraceResult,
} from './types';

const enc = encodeURIComponent;

async function apiFetch<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  const json = await res.json();
  if (!json.ok) throw new Error(json.error || 'Unknown error');
  return json.data as T;
}

export const api = {
  index: (path: string) =>
    apiFetch<IndexStats>('/api/index', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path }),
    }),

  status: (path: string) =>
    apiFetch<unknown>(`/api/status?path=${enc(path)}`),

  architecture: (path: string) =>
    apiFetch<ArchitectureOverview>(`/api/architecture?path=${enc(path)}`),

  searchSymbols: (path: string, q: string, limit = 20) =>
    apiFetch<SearchResult[]>(
      `/api/search-symbols?path=${enc(path)}&q=${enc(q)}&limit=${limit}`,
    ),

  fuzzyFiles: (path: string, q: string, limit = 20) =>
    apiFetch<FuzzyMatch[]>(
      `/api/fuzzy-files?path=${enc(path)}&q=${enc(q)}&limit=${limit}`,
    ),

  fuzzySymbols: (path: string, q: string, limit = 20) =>
    apiFetch<FuzzyMatch[]>(
      `/api/fuzzy-symbols?path=${enc(path)}&q=${enc(q)}&limit=${limit}`,
    ),

  searchContent: (path: string, q: string, limit = 20) =>
    apiFetch<ContentSearchResult[]>(
      `/api/search-content?path=${enc(path)}&q=${enc(q)}&limit=${limit}`,
    ),

  grep: (path: string, pattern: string, limit = 100) =>
    apiFetch<GrepMatch[]>(
      `/api/grep?path=${enc(path)}&pattern=${enc(pattern)}&limit=${limit}`,
    ),

  hybrid: (path: string, q: string, limit = 20) =>
    apiFetch<HybridSearchResult[]>(
      `/api/hybrid?path=${enc(path)}&q=${enc(q)}&limit=${limit}`,
    ),

  graph: (path: string, filter?: string, maxNodes = 500) =>
    apiFetch<SubgraphResult>(
      `/api/graph?path=${enc(path)}&filter=${enc(filter || '')}&max_nodes=${maxNodes}`,
    ),

  concepts: (path: string) =>
    apiFetch<ConceptSummary[]>(`/api/concepts?path=${enc(path)}`),

  conceptMembers: (path: string, concept: string, limit = 100) =>
    apiFetch<unknown[]>(
      `/api/concept-members?path=${enc(path)}&concept=${enc(concept)}&limit=${limit}`,
    ),

  symbolInfo: (path: string, symbol: string) =>
    apiFetch<SymbolDetail[]>(
      `/api/symbol-info?path=${enc(path)}&symbol=${enc(symbol)}`,
    ),

  callsIn: (path: string, symbol: string, limit = 50) =>
    apiFetch<CallHierarchyItem[]>(
      `/api/calls-in?path=${enc(path)}&symbol=${enc(symbol)}&limit=${limit}`,
    ),

  callsOut: (path: string, symbol: string, limit = 50) =>
    apiFetch<CallHierarchyItem[]>(
      `/api/calls-out?path=${enc(path)}&symbol=${enc(symbol)}&limit=${limit}`,
    ),

  blastRadius: (path: string, symbol: string, depth = 3) =>
    apiFetch<unknown>(
      `/api/blast-radius?path=${enc(path)}&symbol=${enc(symbol)}&depth=${depth}`,
    ),

  fileContent: (path: string, file: string) =>
    apiFetch<{ content: string }>(
      `/api/file-content?path=${enc(path)}&file=${enc(file)}`,
    ),

  browse: (path?: string) =>
    apiFetch<BrowseResult>(`/api/browse?path=${enc(path || '')}`),

  embed: (path: string) =>
    apiFetch<{ started: boolean; reason?: string }>('/api/embed', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path }),
    }),

  embedStatus: () =>
    apiFetch<{ state: string; embedded: number; error: string | null }>('/api/embed-status'),

  flowEdges: (path: string, limit = 500) =>
    apiFetch<FlowEdgesResult>(`/api/flow-edges?path=${enc(path)}&limit=${limit}`),

  traceFlow: (path: string, file: string, line: number, depth = 3, direction = 'forward') =>
    apiFetch<FlowStep[]>(
      `/api/trace-flow?path=${enc(path)}&file=${enc(file)}&line=${line}&depth=${depth}&direction=${enc(direction)}`,
    ),

  fullTrace: (path: string, symbol?: string, depth = 4, maxTraces = 15) =>
    apiFetch<FullTraceResult>(
      `/api/full-trace?path=${enc(path)}${symbol ? `&symbol=${enc(symbol)}` : ''}&depth=${depth}&max_traces=${maxTraces}`,
    ),
};
