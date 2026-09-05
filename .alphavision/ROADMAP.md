# Resolution roadmap — from 69.98% to the +20-point re-census gate

Baseline `281f8515`: corpus 69.98% edge-weighted, 9.28M resolved / 3.99M residue.
Census: `resolution-documents/2026-09-04-corpus-residue-census.md`.

**Gate (last milestone):** one full corpus recapture, run only when targeted recaptures
on the smoke set project **≥ 90.0% edge-weighted** (+20 points; ≈ 2.65M of the 3.99M
residue must resolve). Assumption: "20+% more resolved" means +20 percentage points; if it
means 20% of the residue (+0.8M refs, ≈ 76.5%) the gate moves, nothing else changes.

Discipline that applies to every milestone: generic engine first, profile data second,
hooks only when data cannot express it; no builtin lists; targeted recaptures only;
every fix must flip a cascade, not one ref; commit per landed step.

Smoke set for targeted recaptures: the 15-project TS/JS/Vue corpus plus zig-compiler-fresh,
puppet-core (ruby), php-laravel, velocity-apache-struts (java), kotlin-ktor,
pascal-castle-fresh, dart-serverpod, fsharp-fable, make-curl (c), scala-gatling.

## M0 — Hygiene

- [x] `covers()` O(packages) → ancestor walk in `SymbolLocationIndex` (ts-ever-demand 43 → ~4 min)
- [x] commit the `covers()` change
- [x] worktrees: research `target/` (6.3 GB) removed, metadata pruned, 39 merged branches deleted
- [ ] decide the 48 unmerged `worktree-*` / agent branches (list in session report; delete or keep)
- [x] dogfood index rebuilt without the 60,716 stale `.claude/worktrees/*` rows; DB vacuumed (5.6 GB + 1.5 GB WAL)
- [x] walker: never index `.claude/worktrees/**` even when the secondary import-pull scan would pull it (root cause was the file watcher bypassing the exclusions; fixed in `indexer/watch_filter.rs`)
- [x] `bw_search` returns worktree copies before the main tree — verify gone after rebuild

## M1 — Honest census (generic cause recording; expected rate change 0)

Landed: commits `b48f4089` (covers), watcher exclusions, cause recording.

The recorded `cause_kind` misfiles ~663k chain deaths as bare-root deaths. Fix recording
first, re-census, then rank resolution work on the honest numbers.

- [x] `engine/chain_root.rs:69` — value-shaped untypable root records the same-file binding's `untyped_binding`/`uncaptured_field`, else the new `untyped_root`; never `classify_unbound_root(root name)`
  - [x] failing test first: two-segment chain, root is an untyped local, assert `UntypedBinding` with the local blamed
- [x] `engine/semantic_model.rs` fallthrough keeps the walk's cause when the bare ladder also misses (member cause survives)
- [ ] `engine/semantic_model.rs:92` `chain_root_is_namespace` — only a namespace the FILE binds (import, ambient, same-package) qualifies; a same-named namespace elsewhere in the index must not re-run the chain as a bare ladder
  - [ ] failing test first: field `mapper` shadowed by package `x.mapper`; the root stays a value
- [x] `engine/compilation.rs:2396` `is_external_name` — wire to the externals surface so `unbound_external_known` fires; stdlib names leave `name_unknown`
- [x] `engine/chain.rs:2063` — the untyped-root floor now records `untyped_root` / the same-file binding; `:316` is unreachable (namespace anchor always leaves one segment)
- [ ] package-alias roots (`utilsstrings.ToLower`, `clientpkg.New`) record `import_unlinked`, not `name_unknown`
- [x] re-census on 14 projects (addendum in the census doc): `external_known` 242k is the largest honest bucket → M3 reachability outranks M4 supply
- [x] import discipline: external-head attestation removed (denied roots the scoped lookup could not see; superset −7.8k); re-enable only with materialized-module evidence (M6)
- [ ] `resolution_corpus_rust` fails at baseline `281f8515` (SelfProbe nested-module import binds the crate-root twin; AliasDoc root dies `uncaptured_field`) — pre-existing, trace and fix

## M2 — Root typing (generic engine; the largest resolution lever)

Receiver-rooted chains whose root local/param/field has no captured type: zig 91%,
lua 79%, elixir 70%, ts 63%, js 54%, rust 51%, java 47%, kotlin/fsharp 48% of their
ROOT-family rows.

- [x] audit: only java/python/ruby/swift emit parameter symbols; typed params surface as `Property` in ts/csharp/go; flow runner now synthesizes binding symbols and 11 languages' flow queries seed parameter + typed-local types (`132f2482`)
- [ ] lambda / closure params typed from the callee's parameter types (call-site arg→param binding, generic)
- [ ] `this` / `self` root fails enclosing-type lookup (scala `this.modify`) — trace, fix the id-keyed enclosing lookup
- [ ] Go local seeded with the callee's QNAME as its type (`client → client.NewWithClient`) — return-type capture vs seed fallback, trace first
- [ ] pattern / destructuring bindings (`case (key, inputs)`, `const { a } = …`) seed element types
- [ ] targeted recapture: scala-gatling, java-petclinic-rest, go-fiber, zig-compiler-fresh, ts smoke set; cascade gate

## M3 — Reachability rungs (generic engine)

Genuinely bare names whose declaration IS in the index.

- [ ] wildcard-import re-export following (`import 'package:test/test.dart'` → `src/expect.dart`; scala `_`, kotlin `*`)
- [ ] transitive `#include` closure + `NamespacelessGlobalRule` reaching supplied external declarations (C `FILE`/`fprintf`: found by name, no rung reaches it)
- [ ] TS import specifier normalization (`node:path` vs `path` — 63% of ts `import_unlinked` are present by qname)
- [ ] PHP namespace `use` + qualified member (`Illuminate\Database\Eloquent` present, member unlinked)
- [ ] inherited implicit-receiver members when the enclosing type's parent is external (pascal `Create`, `AddField`)
- [ ] targeted recapture: dart-serverpod, make-curl, ts-nextjs, php-laravel, pascal-castle-fresh; cascade gate

## M4 — Supply (locators and install state; no engine code)

`name_unknown` where nothing is supplied: zig 10 external files, swift 0, r 5,
fortran 0, powershell 11, fsharp 124.

- [ ] install zig, swift, R toolchains; reindex their projects (rates are gated on install state)
- [ ] route F# to the dotnet stdlib supply that csharp already gets (77k external files)
- [ ] Maven / Gradle `-sources` jars for the java and kotlin corpora (java 86% of unresolved imports are absent from the index)
- [ ] dart pub `path:` workspace packages (serverpod monorepo: `package:serverpod_client` absent)
- [ ] zig `@import("../std.zig")` path resolution as `ImportAxes.import_resolution` profile data
- [ ] vue-vben-admin −8.9 (pnpm symlink layout vs registered prefixes) — trace, then fix in the npm locator
- [ ] targeted recapture per ecosystem touched

## M5 — Extractor attribution gaps (per-language extractor data, no resolver code)

- [ ] pascal: implementation-section bodies attributed to their declaring class (55,467 residue rows have the file-level `unknown` symbol as source)
- [ ] python / ruby: `self.x` / `@x` instance attributes assigned in method bodies are contract (paperless −3.9, chatwoot −3.8)
- [ ] pascal visible-refs artifact (castle −1.7, heidisql −3.3): confirm by cause census, document
- [ ] targeted recapture: pascal-castle-fresh, pascal-heidisql, python-paperless-ngx, ruby-chatwoot

## M6 — Member-walk residue (generic engine + externals member tables)

- [ ] `member_missing` on external receivers (typescript 64k): external member tables, declaration merging, overload alternative yields
- [ ] remaining `chain_declined` after M1 recording: trace top cascades by receiver type
- [ ] `alias_opaque`: Union / Intersection member walk where every branch carries the member
- [ ] targeted recapture: react-tanstack-query, ts-trpc, ts-inbox-zero, dotnet-squidex

## Gate — Re-census and full corpus recapture

- [ ] targeted recaptures on the smoke set project ≥ 90.0% edge-weighted corpus-wide (+20 points)
- [ ] one full 262-project recapture; baseline committed; census doc rewritten
- [ ] memory + brain updated with the closeout
