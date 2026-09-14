# MCP agent efficiency plan

## Pilot evidence

The same three navigation questions were run with Terra at low reasoning.

| Arm | Calls | Gross tokens | Uncached input + output | Time | Quality |
|---|---:|---:|---:|---:|---|
| bounded shell | 11 | 466,609 | 63,153 | about 83s | 3/3 complete |
| guided MCP | 5 | 188,429 | 22,797 | 229s | two partial, one missing |
| unguided MCP | 9 before cancellation | incomplete | incomplete | over 407s | no final answer |

The guided arm used 63.9% fewer cache-adjusted tokens, but did less work. It
is not a win until it returns the same required evidence.

The trace showed four failures:

- The index returned an old location for `index_qname_from_source` and reported
  `last_indexed_at_ms:unknown`.
- Multi-identifier `bw_search` input became an FTS5 AND query and returned no
  rows.
- `bw_grep` limits were exhausted by Markdown and generated HTML before source
  and tests.
- `bw_investigate` returned no callers for a method with many source
  occurrences because those resolved graph edges were absent.

## Semantic retrieval recheck (2026-09-14)

The semantic surface was probed directly after adding resolved references,
call/impact evidence, forward/reverse flow, and combined call/flow traces. The
probe used checkout `47703660`; the available project index still recorded
`281f8515` and contained 349 stale worktree paths. All token figures in this
section use the same simple character-based estimate as the MCP audit table;
they are retrieval payload estimates, not end-to-end model usage.

| Payload | Tools/calls | Estimated tokens | Warm latency |
|---|---:|---:|---:|
| Entire advertised MCP schema | 24 tools | 4,437 | session cost |
| Configured Codex allowlist schema | 11 tools | 2,087 | session cost |
| Proposed core semantic schema | 5 tools | 1,022 | session cost |
| Two investigates + flow + full trace | 4 calls | 328 | 1-5 ms server time per call |
| Equivalent targeted `rg` occurrence/clue output | 3 calls | 1,664 | under 1 second total |

The four MCP responses were 80.3% smaller than the targeted lexical output.
For the first task batch, however, the configured schema plus responses cost
about 2,415 estimated tokens, 45.1% more than the lexical output. Reducing the
surface to the five core schemas would make that 1,350 tokens, 18.9% smaller,
before model reasoning.

Quality still failed the parity gate:

- `index_qname_from_source` returned the old
  `language_profile.rs:613` definition instead of current
  `name_spelling.rs:219`, with 0 resolved references against 37 source call
  occurrences.
- `all_resolvers_with_workspace` returned 0 resolved references against four
  source references (three calls plus one imported test binding).
- The flow query returned only its source row; the combined trace returned one
  node and zero flow jumps. The index contained no flow edge with a paired
  target, so it could not establish a cross-service path.

Freshness could not be repaired within an interactive budget. An incremental
refresh ran for more than 4.5 minutes without publishing a new snapshot. A
clean archive of `47703660` then ran a full index for nearly 14 minutes without
completion; both were stopped while still CPU-active. No new Terra A/B was
launched because spending model tokens against stale or incomplete evidence
would repeat the original invalid comparison.

Do not claim an MCP token win from this run. The response shape is materially
smaller, but adoption remains blocked on explicit freshness, one persistent
writer with read-only query clients, complete reference evidence, and paired
flow endpoints. Rerun end-to-end model usage only after those prerequisites
pass.

## Current shape

Before the flow tools were added, the server exposed 22 flat tools. The MCP
still has a broad compatibility surface, while Codex now allowlists the
semantic subset. Previous schemas repeated `project` and `format` across
nearly every tool, returned 50 search/grep rows, and offered an 8,000-token
context at depth two.

Each MCP process opens a watcher and launches an initial reindex. Previously,
the first call could launch another sweep in that process because the initial
task bypassed the in-flight gate. Separate processes can still refresh the
same SQLite index concurrently.

Every query also writes its full parameters and full response into the index
DB for audit. Read sessions therefore duplicate response text, enlarge the DB,
and contend with refresh writes.

## Target surface

Expose a small semantic surface by default:

1. `bw_context`: a task to a ranked, bounded evidence bundle.
2. `bw_investigate`: one selected identity with definition, resolved references,
   callers, callees, impact, nearby tests, and a short source excerpt.
3. `bw_flow`: bounded forward/reverse cross-service flow and combined
   call/flow traces with edge provenance.
4. `bw_status`: freshness, indexed commit, working-tree relation, refresh
   state, and index owner.

Defer reindexing, corpus diagnostics, quality gates, pattern search,
completion, workspace graphs, and reports until requested. Keep legacy
`project` and `format=json` inputs accepted but hidden from agent schemas.

An evidence bundle should contain:

```text
#status
state:fresh|refreshing|stale|indexed_commit:<sha>|working_tree:<clean|dirty>

#definitions
S1:<stable-id>|<name>|<kind>|F1:<line>|score:<n>

#occurrences
S1|F2:<line>|resolved-call|confidence:0.98
S1|F3:<line>|unresolved-occurrence|confidence:0.45

#tests
<test-name>|F4:<line>

#source
F1:<start>-<end>|<bounded excerpt>

#meta
tokens:<estimate>|truncated:<bool>|next:<suggested operation>
```

Resolved graph evidence and unresolved occurrence evidence must stay distinct.
An empty caller or flow list is not proof that no relationship exists when
coverage is incomplete. Method and overload lookup must use stable symbol IDs
and source spans.

Exact lexical lookup remains a native host operation (`rg` in Codex). The MCP
does not need to proxy repository grep to save tokens. `bw_grep` remains a
specialized diagnostic/API compatibility tool and is excluded from the
default agent allowlist.

## Index ownership

Use one project-scoped writer/watcher. Stdio MCP processes should be thin query
clients of that owner or elect one owner with an OS-released lock. Other
clients open read-only pools. Promotion after owner exit must be automatic.

Queries may serve the last complete snapshot during refresh, but must label it
stale. Publish refreshes atomically. Audit rows should store duration, result
size, token estimate, truncation, freshness, and a response hash. Full response
bodies should be opt-in debug data with retention limits outside the index DB.

## Agent workflow

1. Get status once when freshness is absent.
2. Request context with a 600-1,200 token budget and depth one.
3. Inspect at most three candidate symbol IDs in one compound request.
4. Use native exact text search only for missing occurrences or tests.
5. Read source directly only for edit spans and compiler/test failures.

Stop when evidence names the definition, affected callers, tests, and current
index state. Do not recover with broad repository grep queries. Give subagents
a compact evidence bundle and a fresh context instead of the full transcript.

## Delivery order

### P0 — immediate fixes

- Compact bounded output by default.
- Retry empty multi-term symbol searches as OR alternatives.
- Keep documentation out of the legacy MCP source grep unless requested.
- Route initial refresh through the in-process sweep gate.
- Hide compatibility-only project and format fields from schemas.

### P1 — freshness and one writer

- Add status with indexed commit, working-tree relation, refresh state, owner,
  and last complete snapshot time.
- Elect one writer/watcher; make other clients read-only.
- Move verbose audit bodies out of the index DB.

### P2 — complete evidence

- Boost exact identifiers and retrieve file/test names.
- Inspect by stable symbol ID.
- Return resolved callers plus labelled unresolved/text occurrences.
- Expose bounded forward/reverse flow and combined call/flow traces.
- Include bounded source and nearby tests.

### P3 — smaller surface

- Default to four agent tools and defer specialized schemas.
- Retire redundant standalone calls after compound-query parity.

### P4 — acceptance gate

Run held-out exact lookup, semantic discovery, caller tracing, test discovery,
and impact tasks. Measure schema tokens separately from tool output and model
reasoning. Compare gross/uncached tokens, calls, p50/p95 time, required-fact
recall, and false evidence.

Adopt MCP-first navigation when required facts match the shell baseline with no
stale evidence and median cache-adjusted tokens fall at least 30%. MCP-only
navigation requires the same quality at p95.
