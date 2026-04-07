# MCP Token Optimization Plan

## Problem

BearWisdom MCP tools return JSON responses that are verbose for LLM consumption. A single `bw_architecture_overview` call for eShop produces ~25KB / ~6,200 tokens. Field names (`file_path`, `qualified_name`, `route_template`) are repeated hundreds of times. File paths like `src/Catalog.API/Apis/CatalogApi.cs` appear 32 times in one response.

LLMs pay per token. Every redundant byte costs money and wastes context window. The MCP output format should maximize information density.

## Current State

- All 14 MCP tools return JSON via `serde_json::to_string`
- No compression, no deduplication, no reference IDs
- Field names repeated per-item (JSON's fundamental limitation)
- File paths repeated across symbols, routes, edges, hotspots
- `ok: true/false` wrapper adds overhead per response

### Measured redundancy (eShop `architecture`):
- `file_path` field name: 117 occurrences
- `src/Catalog.API/Apis/CatalogApi.cs`: 32 occurrences (36 bytes each = 1,152 bytes wasted)
- `http_method`, `route_template`: 87 occurrences each
- Total response: 25KB JSON, estimated 40-50% redundant

## Strategy: Referential Compact Format

### Core idea: define-once, reference-by-ID

Instead of:
```json
{
  "symbols": [
    {"name": "MapCatalogApi", "file_path": "src/Catalog.API/Apis/CatalogApi.cs", "line": 11},
    {"name": "GetItems", "file_path": "src/Catalog.API/Apis/CatalogApi.cs", "line": 45},
    {"name": "GetItemById", "file_path": "src/Catalog.API/Apis/CatalogApi.cs", "line": 60}
  ]
}
```

Produce:
```
#files
F1:src/Catalog.API/Apis/CatalogApi.cs
F2:src/Ordering.API/Apis/OrdersApi.cs

#symbols
MapCatalogApi|F1:11|method|public
GetItems|F1:45|method|public
GetItemById|F1:60|method|public
```

### Estimated savings: 40-60% token reduction

---

## Implementation Plan

### Phase 1: Compact Text Format (`bw_compact`)

Add a `compact` output mode to all MCP tools. When `format: "compact"` is passed (or a global preference), responses use a referential text format instead of JSON.

**Format specification:**

```
# Section headers prefixed with #
# References use short IDs: F1, S1, P1 (files, symbols, packages)
# Fields separated by | (pipe)
# Optional fields omitted entirely (not null)

#meta
project:eShop|files:677|symbols:6237|edges:4169|resolution:99.8%

#files
F1:src/Catalog.API/Apis/CatalogApi.cs
F2:src/Ordering.API/Apis/OrdersApi.cs
F3:src/Identity.API/Quickstart/Account/AccountController.cs

#symbols
S1:MapCatalogApi|F1:11|method|public|refs:32
S2:GetItems|F1:45|method|public|refs:8
S3:Login|F3:20|method|public|refs:5

#routes
GET /api/catalog/items → S2
GET /api/catalog/items/{id} → S3
POST /api/identity/login → S3

#edges
S1 → S2|calls
S1 → S3|calls
S2 → S4|type_ref
```

**Key properties:**
- **No repeated field names** — position-based within each section
- **File paths defined once** — referenced by `F1`, `F2`, etc.
- **Symbol IDs** — referenced by `S1`, `S2`, etc.
- **Human-readable** — LLMs can parse pipe-delimited text easily
- **Backward compatible** — JSON format remains the default, compact is opt-in

### Phase 2: Per-Tool Compact Formatters

Each tool gets a compact formatter alongside its JSON serializer.

**Priority order (by typical response size and frequency):**

| Tool | Typical JSON | Estimated Compact | Savings |
|------|-------------|------------------|---------|
| `bw_architecture_overview` | 25 KB | 8 KB | 68% |
| `bw_investigate` | 15 KB | 5 KB | 67% |
| `bw_find_references` | 10 KB | 3 KB | 70% |
| `bw_blast_radius` | 12 KB | 4 KB | 67% |
| `bw_call_hierarchy` | 8 KB | 3 KB | 63% |
| `bw_file_symbols` | 6 KB | 2 KB | 67% |
| `bw_search` | 4 KB | 2 KB | 50% |
| `bw_context` | 20 KB | 8 KB | 60% |
| `bw_dead_code` | 5 KB | 2 KB | 60% |
| `bw_symbol_info` | 3 KB | 1.5 KB | 50% |
| `bw_diagnostics` | 3 KB | 1.5 KB | 50% |
| `bw_grep` | 4 KB | 2.5 KB | 38% |
| `bw_complete` | 2 KB | 1 KB | 50% |
| `bw_entry_points` | 4 KB | 2 KB | 50% |

### Phase 3: File Registry Deduplication

The biggest single win: build a file registry at the start of each response and reference files by ID throughout.

**Implementation:**

```rust
struct CompactFormatter {
    files: IndexMap<String, String>,   // path → "F1", "F2", ...
    symbols: IndexMap<i64, String>,    // symbol_id → "S1", "S2", ...
    next_file: usize,
    next_symbol: usize,
}

impl CompactFormatter {
    fn file_ref(&mut self, path: &str) -> &str {
        self.files.entry(path.to_string()).or_insert_with(|| {
            self.next_file += 1;
            format!("F{}", self.next_file)
        })
    }
    
    fn format_architecture(&mut self, overview: &ArchitectureOverview) -> String {
        let mut out = String::new();
        // Build file registry from all items
        // ... then format sections using refs
        out
    }
}
```

**Files to create:**
- `crates/bearwisdom-mcp/src/compact.rs` — compact formatter
- Modify `server.rs` — add format parameter dispatch

### Phase 4: Skeletal Mode

For `bw_symbol_info` and `bw_context`, offer a skeletal mode that strips function bodies and only returns signatures + docstrings.

Current:
```json
{
  "name": "MapCatalogApi",
  "signature": "internal static IEndpointRouteBuilder MapCatalogApi(this IEndpointRouteBuilder app)",
  "doc_comment": "/// Maps the catalog API endpoints.",
  "children": [
    {"name": "GetItems", "signature": "...", "body": "... 50 lines ..."},
  ]
}
```

Skeletal:
```
MapCatalogApi(IEndpointRouteBuilder app) → IEndpointRouteBuilder
  /// Maps the catalog API endpoints.
  ├─ GetItems(PaginationRequest, CatalogServices) → Ok<PaginatedItems>
  ├─ GetItemById(int id, CatalogServices) → Ok<CatalogItem> | NotFound
  ├─ CreateItem(CreateCatalogItemRequest, CatalogServices) → Created
  └─ DeleteItem(int id, CatalogServices) → NoContent | NotFound
```

**Savings:** Body stripping removes 60-80% of `bw_context` response size.

### Phase 5: Response Budget

Add a `max_tokens` parameter to MCP tools. The formatter truncates output to fit within the token budget, prioritizing higher-signal items.

```rust
fn format_with_budget(&mut self, data: &T, max_tokens: usize) -> String {
    // Format everything
    let full = self.format(data);
    if full.len() / 4 <= max_tokens {
        return full;
    }
    // Truncate: keep headers + top N items per section
    self.format_truncated(data, max_tokens)
}
```

**Truncation strategy:**
1. Keep meta section always
2. Keep file registry always (small, enables references)
3. Truncate symbol/route/edge lists by importance (incoming_edge_count)
4. Add `... and N more` footer

---

## Implementation Details

### Format Parameter

Add to all MCP tool params:
```rust
pub format: Option<String>,  // "json" (default) | "compact"
```

In the server dispatch:
```rust
match params.format.as_deref() {
    Some("compact") => CompactFormatter::new().format_architecture(&overview),
    _ => Self::to_json(&overview),
}
```

### CLI Integration

Add `--compact` flag to CLI:
```
bw architecture /path/to/project --compact
bw investigate /path/to/project --symbol Foo --compact
```

### Backward Compatibility

- JSON remains the default format
- Compact is opt-in via `format: "compact"` parameter
- Web API always returns JSON (frontend needs structured data)
- MCP and CLI support both formats

---

## Risks

| Risk | Mitigation |
|------|-----------|
| LLMs can't parse pipe-delimited format | Test with Claude, GPT-4, Gemini — all handle structured text well |
| Format changes break existing skill prompts | JSON default unchanged; compact is additive |
| ID references confuse LLMs | Use descriptive IDs (F1:path visible in registry) |
| Token counting is approximate | Use chars/4 as estimate; add `response_tokens` field to meta |

## Out of Scope

- **Binary/protobuf MCP transport** — MCP protocol is text-based
- **Streaming responses** — MCP tools return complete responses
- **Client-side decompression** — LLMs receive raw text, not gzipped

---

## Priority

Phase 1 (format spec + architecture formatter) is the quick win — one tool, immediate 60% savings, validates the approach. Phase 3 (file registry) is the infrastructure that makes all subsequent tools efficient. Phase 4 (skeletal) is the highest absolute savings for `bw_context`.

Recommended order: **1 → 3 → 2 → 4 → 5**
